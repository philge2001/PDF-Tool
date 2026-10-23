//! Formularmodell (AcroForm): Felder, ihre Werte und ihre Widgets auf den Seiten.
//!
//! Begriffe aus der PDF-Spezifikation:
//! - Ein **Feld** hat einen Namen und einen Wert (`/T`, `/V`). Felder bilden einen Baum,
//!   der volle Name entsteht aus den Namen der Vorfahren ("person.name").
//! - Ein **Widget** ist die sichtbare Annotation eines Felds auf einer Seite. Ein Feld kann
//!   mehrere Widgets haben (z. B. jede Option einer Optionsfeld-Gruppe). Hat ein Feld genau
//!   ein Widget, stehen Feld und Widget oft im selben Objekt.
//!
//! Wie bei Annotationen gilt: Was wir verstehen, landet im Modell und wird von PDF Tool
//! gezeichnet und gespeichert; der Rest bleibt in der Datei, wie er ist.

pub mod appearance;
pub mod import;

use crate::geometry::{Color, Rect};
use lopdf::ObjectId;

/// Sitzungsweit eindeutige ID eines Felds (nicht im PDF gespeichert).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId(pub u32);

/// Feldart samt artspezifischer Eigenschaften.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text {
        multiline: bool,
        password: bool,
        /// Höchstzahl an Zeichen (/MaxLen).
        max_len: Option<u32>,
    },
    CheckBox,
    /// Optionsfeld-Gruppe: genau ein Widget ist "an".
    Radio,
    /// Kombinationsfeld (Aufklappliste), optional mit freier Eingabe.
    Combo {
        editable: bool,
    },
    /// Listenfeld, optional mit Mehrfachauswahl.
    List {
        multi: bool,
    },
    /// Schaltfläche ohne Wert (wird nicht ausgefüllt, PDFium zeichnet sie).
    PushButton,
    /// Signaturfeld (Phase 4; heute nur angezeigt).
    Signature,
}

impl FieldKind {
    /// Kann der Nutzer einen Wert eintragen? Nur solche Felder zeichnet PDF Tool selbst.
    pub fn is_fillable(&self) -> bool {
        !matches!(self, FieldKind::PushButton | FieldKind::Signature)
    }

    pub fn label(&self) -> &'static str {
        match self {
            FieldKind::Text { .. } => "Textfeld",
            FieldKind::CheckBox => "Kontrollkästchen",
            FieldKind::Radio => "Optionsfeld",
            FieldKind::Combo { .. } => "Kombinationsfeld",
            FieldKind::List { .. } => "Listenfeld",
            FieldKind::PushButton => "Schaltfläche",
            FieldKind::Signature => "Signaturfeld",
        }
    }
}

/// Der Wert eines Felds.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FieldValue {
    /// Kein Wert (Schaltflächen, Signaturfelder, leere Felder ohne /V).
    #[default]
    None,
    Text(String),
    /// Kontrollkästchen/Optionsfeld: Name des eingeschalteten Zustands, `None` = aus.
    Toggle(Option<String>),
    /// Auswahl- und Listenfelder: Exportwerte der gewählten Einträge.
    Choice(Vec<String>),
}

impl FieldValue {
    /// Der Wert als einfacher Text (z. B. für Anzeige und Tests).
    pub fn as_text(&self) -> String {
        match self {
            FieldValue::None | FieldValue::Toggle(None) => String::new(),
            FieldValue::Text(s) | FieldValue::Toggle(Some(s)) => s.clone(),
            FieldValue::Choice(v) => v.join(", "),
        }
    }
}

/// Eintrag eines Auswahl-/Listenfelds: Exportwert (steht in /V) und angezeigter Text.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceOption {
    pub export: String,
    pub display: String,
}

/// Textausrichtung (/Q).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// Die sichtbare Darstellung eines Felds auf einer Seite.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    /// Seite als Objektnummer des Seiten-Dictionarys. Anders als ein Index bleibt sie beim
    /// Umsortieren gleich; wird die Seite gelöscht, verschwindet das Widget einfach mit.
    pub page: ObjectId,
    pub rect: Rect,
    /// Objektnummer der Widget-Annotation in der Datei (`None` = neu, noch nicht gespeichert).
    pub object_id: Option<ObjectId>,
    /// Zustand "an" bei Kontrollkästchen und Optionsfeldern (z. B. "Ja", "post").
    pub on_state: Option<String>,
    /// Hintergrund- und Rahmenfarbe (/MK /BG, /BC).
    pub background: Option<Color>,
    pub border: Option<Color>,
    pub border_width: f32,
    /// Ausgeblendet (/F Bit 2) – wird weder gezeichnet noch bedient.
    pub hidden: bool,
}

impl Widget {
    pub fn new(page: ObjectId, rect: Rect) -> Self {
        Self {
            page,
            rect,
            object_id: None,
            on_state: None,
            background: None,
            border: None,
            border_width: 1.0,
            hidden: false,
        }
    }
}

/// Ein (terminales) Formularfeld.
#[derive(Debug, Clone, PartialEq)]
pub struct FormField {
    pub id: FieldId,
    /// Voller Name, z. B. "person.name".
    pub name: String,
    /// Kurzbeschreibung (/TU), z. B. als Tooltip.
    pub tooltip: String,
    pub kind: FieldKind,
    pub value: FieldValue,
    /// Standardwert (/DV) für "Formular zurücksetzen".
    pub default: FieldValue,
    pub read_only: bool,
    pub required: bool,
    /// Einträge für Auswahl- und Listenfelder.
    pub options: Vec<ChoiceOption>,
    /// Schriftgröße aus /DA; 0 = automatisch an das Feld anpassen.
    pub font_size: f32,
    pub text_color: Color,
    pub align: Align,
    pub widgets: Vec<Widget>,
    /// Objektnummer des Feld-Dictionarys in der Datei (`None` = neues Feld).
    pub source: Option<ObjectId>,
}

impl FormField {
    /// Neues Feld mit einem Widget und üblichen Standardwerten (für den Formular-Editor).
    pub fn new(kind: FieldKind, name: impl Into<String>, page: ObjectId, rect: Rect) -> Self {
        let mut widget = Widget::new(page, rect);
        widget.border = Some(Color::rgb(0.45, 0.45, 0.45));
        let value = match kind {
            FieldKind::CheckBox => {
                widget.on_state = Some("Ja".into());
                FieldValue::Toggle(None)
            }
            FieldKind::Radio => {
                widget.on_state = Some("Option1".into());
                FieldValue::Toggle(None)
            }
            FieldKind::Text { .. } => FieldValue::Text(String::new()),
            FieldKind::Combo { .. } | FieldKind::List { .. } => FieldValue::Choice(Vec::new()),
            FieldKind::PushButton | FieldKind::Signature => FieldValue::None,
        };
        Self {
            id: FieldId(0),
            name: name.into(),
            tooltip: String::new(),
            kind,
            default: value.clone(),
            value,
            read_only: false,
            required: false,
            options: Vec::new(),
            font_size: 0.0,
            text_color: Color::BLACK,
            align: Align::Left,
            widgets: vec![widget],
            source: None,
        }
    }

    /// Stehen Feld und (einziges) Widget im selben PDF-Objekt? Dann kann kein zweites
    /// Widget hinzukommen, ohne die Struktur umzubauen.
    pub fn is_merged(&self) -> bool {
        self.source.is_some() && self.widgets.first().and_then(|w| w.object_id) == self.source
    }

    /// Angezeigter Text eines Exportwerts (bei Auswahlfeldern).
    pub fn display_of<'a>(&'a self, export: &'a str) -> &'a str {
        self.options
            .iter()
            .find(|o| o.export == export)
            .map_or(export, |o| o.display.as_str())
    }

    /// Ist ein Widget (Kontrollkästchen/Optionsfeld) gerade eingeschaltet?
    pub fn widget_is_on(&self, w: &Widget) -> bool {
        match (&self.value, &w.on_state) {
            (FieldValue::Toggle(Some(v)), Some(on)) => v == on,
            _ => false,
        }
    }

    /// Kurzer Name für Menüs ("name" statt "person.name").
    pub fn short_name(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }
}

/// Alle Felder des Dokuments in Dateireihenfolge (= Standard-Tab-Reihenfolge).
#[derive(Debug, Default, Clone)]
pub struct FormStore {
    fields: Vec<FormField>,
    next_id: u32,
}

impl FormStore {
    pub fn allocate_id(&mut self) -> FieldId {
        self.next_id += 1;
        FieldId(self.next_id)
    }

    /// Fügt ein Feld hinzu; hat es noch keine ID (0), wird eine vergeben.
    pub fn push(&mut self, mut f: FormField) -> FieldId {
        if f.id.0 == 0 {
            f.id = self.allocate_id();
        }
        self.next_id = self.next_id.max(f.id.0);
        let id = f.id;
        self.fields.push(f);
        id
    }

    pub fn get(&self, id: FieldId) -> Option<&FormField> {
        self.fields.iter().find(|f| f.id == id)
    }

    pub fn by_name(&self, name: &str) -> Option<&FormField> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Freier Name der Form "Textfeld1", "Textfeld2" …
    pub fn unique_name(&self, prefix: &str) -> String {
        (1..)
            .map(|i| format!("{prefix}{i}"))
            .find(|n| self.by_name(n).is_none())
            .unwrap_or_default()
    }

    /// Fügt ein Feld an einer Position ein (für Undo).
    pub fn insert(&mut self, index: usize, f: FormField) {
        self.next_id = self.next_id.max(f.id.0);
        let index = index.min(self.fields.len());
        self.fields.insert(index, f);
    }

    /// Entfernt ein Feld; liefert Position und Inhalt (für Undo).
    pub fn remove(&mut self, id: FieldId) -> Option<(usize, FormField)> {
        let i = self.fields.iter().position(|f| f.id == id)?;
        Some((i, self.fields.remove(i)))
    }

    /// Ersetzt das Feld mit derselben ID und liefert die alte Fassung.
    pub fn replace(&mut self, f: FormField) -> Option<FormField> {
        let slot = self.fields.iter_mut().find(|x| x.id == f.id)?;
        Some(std::mem::replace(slot, f))
    }

    pub fn iter(&self) -> impl Iterator<Item = &FormField> {
        self.fields.iter()
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Alle Widgets auf einer Seite (Seite als Objektnummer) samt ihrem Feld.
    pub fn widgets_on(&self, page: ObjectId) -> impl Iterator<Item = (&FormField, &Widget)> {
        self.fields
            .iter()
            .flat_map(|f| f.widgets.iter().map(move |w| (f, w)))
            .filter(move |(_, w)| w.page == page)
    }

    /// Objektnummern aller Widgets, die PDF Tool selbst zeichnet (ausfüllbare Felder).
    pub fn managed_widget_ids(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.fields
            .iter()
            .filter(|f| f.kind.is_fillable())
            .flat_map(|f| f.widgets.iter())
            .filter(|w| !w.hidden)
            .filter_map(|w| w.object_id)
    }
}
