//! PDF -> Modell: liest den Feldbaum aus /AcroForm.
//!
//! Viele Feldattribute sind vererbbar (/FT, /Ff, /V, /DV, /DA, /Q, /MaxLen): Sie dürfen
//! auch bei einem Vorfahren stehen. Beim Abstieg durch den Baum wird deshalb ein
//! [`Inherited`]-Stand mitgeführt und an jedem Knoten überschrieben.

use super::{Align, ChoiceOption, FieldId, FieldKind, FieldValue, FormField, Widget};
use crate::annotation::import::{array, deref, float, int, numbers, page_annots, parse_da, rect};
use crate::document::PageInfo;
use crate::geometry::Color;
use crate::pdf_string;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::{HashMap, HashSet};

// Feld-Flags (/Ff), Bitnummern laut Spezifikation minus 1.
pub(crate) const FF_READ_ONLY: i64 = 1;
pub(crate) const FF_REQUIRED: i64 = 1 << 1;
pub(crate) const FF_MULTILINE: i64 = 1 << 12;
pub(crate) const FF_PASSWORD: i64 = 1 << 13;
pub(crate) const FF_RADIO: i64 = 1 << 15;
pub(crate) const FF_PUSHBUTTON: i64 = 1 << 16;
pub(crate) const FF_COMBO: i64 = 1 << 17;
pub(crate) const FF_EDIT: i64 = 1 << 18;
pub(crate) const FF_MULTISELECT: i64 = 1 << 21;

/// Maximale Baumtiefe – schützt vor kaputten Dateien mit Zyklen.
const MAX_DEPTH: usize = 32;

/// Vererbbare Attribute auf dem Weg von der Wurzel zum Feld.
#[derive(Clone, Default)]
struct Inherited {
    ft: Option<Vec<u8>>,
    ff: i64,
    v: Option<Object>,
    dv: Option<Object>,
    da: Option<Vec<u8>>,
    q: i64,
    max_len: Option<i64>,
}

impl Inherited {
    /// Übernimmt alles, was in `d` steht (Kinder überschreiben Vorfahren).
    fn update(&mut self, pdf: &lopdf::Document, d: &Dictionary) {
        if let Ok(ft) = d.get(b"FT").map(|o| deref(pdf, o))
            && let Ok(name) = ft.as_name()
        {
            self.ft = Some(name.to_vec());
        }
        if let Some(ff) = int(pdf, d, b"Ff") {
            self.ff = ff;
        }
        if let Ok(v) = d.get(b"V") {
            self.v = Some(deref(pdf, v).clone());
        }
        if let Ok(v) = d.get(b"DV") {
            self.dv = Some(deref(pdf, v).clone());
        }
        if let Ok(da) = d.get(b"DA").map(|o| deref(pdf, o))
            && let Ok(s) = da.as_str()
        {
            self.da = Some(s.to_vec());
        }
        if let Some(q) = int(pdf, d, b"Q") {
            self.q = q;
        }
        if let Some(m) = int(pdf, d, b"MaxLen") {
            self.max_len = Some(m);
        }
    }
}

/// Liest alle Felder. `pages` liefert die Zuordnung Widget -> Seite.
pub fn read(pdf: &lopdf::Document, pages: &[PageInfo]) -> Vec<FormField> {
    let Some(acro) = acroform(pdf) else {
        return Vec::new();
    };
    // Welche Annotation liegt auf welcher Seite? Einmal vorab für alle Seiten.
    let mut page_of: HashMap<ObjectId, ObjectId> = HashMap::new();
    for p in pages {
        for o in page_annots(pdf, p.object_id) {
            if let Ok(id) = o.as_reference() {
                page_of.entry(id).or_insert(p.object_id);
            }
        }
    }
    let mut root = Inherited::default();
    root.update(pdf, acro); // /DA und /Q gelten dokumentweit als Standard
    root.ft = None;
    root.v = None;
    root.dv = None;

    let mut ctx = Ctx {
        pdf,
        page_of,
        visited: HashSet::new(),
        out: Vec::new(),
        next_id: 1,
    };
    for f in array(pdf, acro, b"Fields").into_iter().flatten() {
        if let Ok(id) = f.as_reference() {
            ctx.walk(id, "", &root, 0);
        }
    }
    ctx.out
}

/// Das /AcroForm-Dictionary des Katalogs (direkt oder als Verweis).
pub(crate) fn acroform(pdf: &lopdf::Document) -> Option<&Dictionary> {
    let o = pdf.catalog().ok()?.get(b"AcroForm").ok()?;
    deref(pdf, o).as_dict().ok()
}

struct Ctx<'a> {
    pdf: &'a lopdf::Document,
    page_of: HashMap<ObjectId, ObjectId>,
    visited: HashSet<ObjectId>,
    out: Vec<FormField>,
    next_id: u32,
}

impl Ctx<'_> {
    fn walk(&mut self, id: ObjectId, parent_name: &str, inh: &Inherited, depth: usize) {
        if depth > MAX_DEPTH || !self.visited.insert(id) {
            return;
        }
        let pdf = self.pdf;
        let Ok(d) = pdf.get_dictionary(id) else {
            return;
        };
        let mut inh = inh.clone();
        inh.update(pdf, d);
        let partial = pdf_string::get(d, b"T");
        let name = match (parent_name.is_empty(), partial.is_empty()) {
            (_, true) => parent_name.to_string(),
            (true, false) => partial,
            (false, false) => format!("{parent_name}.{partial}"),
        };

        let kids: Vec<ObjectId> = array(pdf, d, b"Kids")
            .into_iter()
            .flatten()
            .filter_map(|k| k.as_reference().ok())
            .collect();
        // Ein Kind ohne /T, das eine Widget-Annotation ist, ist nur eine Darstellung dieses
        // Felds. Alles andere sind Unterfelder.
        let is_widget_only = |k: &ObjectId| {
            pdf.get_dictionary(*k).is_ok_and(|kd| {
                !kd.has(b"T")
                    && kd.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget")
            })
        };
        if kids.iter().any(|k| !is_widget_only(k)) {
            for k in kids {
                self.walk(k, &name, &inh, depth + 1);
            }
            return;
        }

        let widget_ids = if kids.is_empty() { vec![id] } else { kids };
        if let Some(field) = self.field(id, d, name, &inh, &widget_ids) {
            self.out.push(field);
        }
    }

    fn field(
        &mut self,
        id: ObjectId,
        d: &Dictionary,
        name: String,
        inh: &Inherited,
        widget_ids: &[ObjectId],
    ) -> Option<FormField> {
        let pdf = self.pdf;
        let ff = inh.ff;
        let kind = match inh.ft.as_deref()? {
            b"Tx" => FieldKind::Text {
                multiline: ff & FF_MULTILINE != 0,
                password: ff & FF_PASSWORD != 0,
                max_len: inh.max_len.and_then(|m| u32::try_from(m).ok()),
            },
            b"Btn" if ff & FF_PUSHBUTTON != 0 => FieldKind::PushButton,
            b"Btn" if ff & FF_RADIO != 0 => FieldKind::Radio,
            b"Btn" => FieldKind::CheckBox,
            b"Ch" if ff & FF_COMBO != 0 => FieldKind::Combo {
                editable: ff & FF_EDIT != 0,
            },
            b"Ch" => FieldKind::List {
                multi: ff & FF_MULTISELECT != 0,
            },
            b"Sig" => FieldKind::Signature,
            _ => return None,
        };

        let mut widgets: Vec<Widget> = widget_ids
            .iter()
            .filter_map(|&wid| self.widget(wid))
            .collect();
        if widgets.is_empty() {
            return None;
        }
        // Kontrollkästchen/Optionsfelder ohne Erscheinungsbild: Standardnamen vergeben.
        for (i, w) in widgets.iter_mut().enumerate() {
            if w.on_state.is_none() {
                match kind {
                    FieldKind::CheckBox => w.on_state = Some("Yes".into()),
                    FieldKind::Radio => w.on_state = Some(format!("Option{}", i + 1)),
                    _ => {}
                }
            }
        }

        let value = match &inh.v {
            Some(v) => parse_value(pdf, &kind, v),
            None => default_value(&kind),
        };
        // Fehlt /V bei Schaltern, gilt der Zustand der Widgets (/AS).
        let value = match (&kind, value) {
            (FieldKind::CheckBox | FieldKind::Radio, FieldValue::Toggle(None))
                if inh.v.is_none() =>
            {
                FieldValue::Toggle(
                    widget_ids
                        .iter()
                        .filter_map(|&w| pdf.get_dictionary(w).ok())
                        .filter_map(|wd| wd.get(b"AS").and_then(Object::as_name).ok())
                        .find(|s| *s != b"Off")
                        .map(|s| String::from_utf8_lossy(s).into_owned()),
                )
            }
            (_, v) => v,
        };
        let default = match &inh.dv {
            Some(v) => parse_value(pdf, &kind, v),
            None => default_value(&kind),
        };

        let (font_size, color) = inh.da.as_deref().map(parse_da).unwrap_or((None, None));
        let options = array(pdf, d, b"Opt")
            .into_iter()
            .flatten()
            .filter_map(|o| parse_option(pdf, o))
            .collect();

        let field = FormField {
            id: FieldId(self.next_id),
            name,
            tooltip: pdf_string::get(d, b"TU"),
            kind,
            value,
            default,
            read_only: ff & FF_READ_ONLY != 0,
            required: ff & FF_REQUIRED != 0,
            options,
            font_size: font_size.unwrap_or(0.0),
            text_color: color.unwrap_or(Color::BLACK),
            align: match inh.q {
                1 => Align::Center,
                2 => Align::Right,
                _ => Align::Left,
            },
            widgets,
            source: Some(id),
        };
        self.next_id += 1;
        Some(field)
    }

    fn widget(&self, id: ObjectId) -> Option<Widget> {
        let pdf = self.pdf;
        let d = pdf.get_dictionary(id).ok()?;
        let rect = rect(pdf, d, b"Rect")?;
        // Seite: bevorzugt aus /Annots (zuverlässig), sonst aus /P.
        let page = self.page_of.get(&id).copied().or_else(|| {
            let p = d.get(b"P").ok()?.as_reference().ok()?;
            self.page_of.values().any(|v| *v == p).then_some(p)
        })?;
        let flags = int(pdf, d, b"F").unwrap_or(0);

        // Zustand "an": jeder Name in /AP /N außer "Off".
        let on_state = d
            .get(b"AP")
            .ok()
            .and_then(|ap| deref(pdf, ap).as_dict().ok())
            .and_then(|ap| ap.get(b"N").ok())
            .and_then(|n| deref(pdf, n).as_dict().ok())
            .and_then(|n| {
                n.iter()
                    .map(|(k, _)| k)
                    .find(|k| k.as_slice() != b"Off")
                    .map(|k| String::from_utf8_lossy(k).into_owned())
            });

        let mk = d.get(b"MK").ok().and_then(|o| deref(pdf, o).as_dict().ok());
        let mk_color = |key: &[u8]| mk.and_then(|mk| color_array(pdf, mk.get(key).ok()?));
        let border_width = d
            .get(b"BS")
            .ok()
            .and_then(|bs| deref(pdf, bs).as_dict().ok())
            .and_then(|bs| float(pdf, bs, b"W"))
            .unwrap_or(1.0);

        Some(Widget {
            page,
            rect,
            object_id: Some(id),
            on_state,
            background: mk_color(b"BG"),
            border: mk_color(b"BC"),
            border_width,
            hidden: flags & 2 != 0,
        })
    }
}

fn default_value(kind: &FieldKind) -> FieldValue {
    match kind {
        FieldKind::Text { .. } => FieldValue::Text(String::new()),
        FieldKind::CheckBox | FieldKind::Radio => FieldValue::Toggle(None),
        FieldKind::Combo { .. } | FieldKind::List { .. } => FieldValue::Choice(Vec::new()),
        FieldKind::PushButton | FieldKind::Signature => FieldValue::None,
    }
}

fn parse_value(pdf: &lopdf::Document, kind: &FieldKind, v: &Object) -> FieldValue {
    let text =
        |o: &Object| -> Option<String> { deref(pdf, o).as_str().ok().map(pdf_string::decode) };
    match kind {
        FieldKind::Text { .. } => {
            // Zeilenumbrüche vereinheitlichen: \r\n und \r -> \n.
            let s = text(v).unwrap_or_default();
            FieldValue::Text(s.replace("\r\n", "\n").replace('\r', "\n"))
        }
        FieldKind::CheckBox | FieldKind::Radio => FieldValue::Toggle(
            v.as_name()
                .ok()
                .filter(|n| *n != b"Off")
                .map(|n| String::from_utf8_lossy(n).into_owned()),
        ),
        FieldKind::Combo { .. } | FieldKind::List { .. } => FieldValue::Choice(match v {
            Object::Array(a) => a.iter().filter_map(text).collect(),
            o => text(o).into_iter().filter(|s| !s.is_empty()).collect(),
        }),
        FieldKind::PushButton | FieldKind::Signature => FieldValue::None,
    }
}

fn parse_option(pdf: &lopdf::Document, o: &Object) -> Option<ChoiceOption> {
    let text = |o: &Object| deref(pdf, o).as_str().ok().map(pdf_string::decode);
    match deref(pdf, o) {
        Object::Array(pair) if pair.len() == 2 => Some(ChoiceOption {
            export: text(&pair[0])?,
            display: text(&pair[1])?,
        }),
        other => {
            let s = text(other)?;
            Some(ChoiceOption {
                export: s.clone(),
                display: s,
            })
        }
    }
}

/// Farbe aus einem Zahlen-Array (Grau, RGB oder CMYK). Leeres Array = transparent.
fn color_array(pdf: &lopdf::Document, o: &Object) -> Option<Color> {
    match numbers(pdf, o)?.as_slice() {
        [g] => Some(Color::rgb(*g, *g, *g)),
        [r, g, b] => Some(Color::rgb(*r, *g, *b)),
        [c, m, y, k] => Some(Color::rgb(
            (1.0 - c) * (1.0 - k),
            (1.0 - m) * (1.0 - k),
            (1.0 - y) * (1.0 - k),
        )),
        _ => None,
    }
}
