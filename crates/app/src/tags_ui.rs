//! Panel "Tags" (Barrierefreiheit): Dokumenttitel und -sprache, Strukturbaum in
//! Lesereihenfolge, Alternativtexte für Abbildungen und eine Overlay-Ansicht der
//! Lesereihenfolge auf der Seite.
//!
//! Der Baum kommt aus `Document::structure_tree` (core), die Texte und Rechtecke je
//! Strukturelement aus `Document::marked_content` (Interpreter). Beides wird zwischengespeichert:
//! der Baum, bis sich Dateibytes, Seitenreihenfolge oder Alternativtexte ändern, die
//! markierten Inhalte pro Seite, bis sich deren Inhalt (Stempel) ändert.

use crate::app::OpenDocument;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, Color32, RichText, Stroke, StrokeKind};
use pdftool_core::ObjectId;
use pdftool_core::content::MarkedContent;
use pdftool_core::tags::{DocProperties, SetAltText, SetProperties};
use std::collections::{BTreeMap, HashMap};

/// Häufige Sprachkennungen für das Auswahlmenü (BCP 47).
const LANGUAGES: &[(&str, &str)] = &[
    ("de-DE", "Deutsch (Deutschland)"),
    ("de-AT", "Deutsch (Österreich)"),
    ("de-CH", "Deutsch (Schweiz)"),
    ("en-US", "Englisch (USA)"),
    ("en-GB", "Englisch (Großbritannien)"),
    ("fr-FR", "Französisch"),
    ("it-IT", "Italienisch"),
    ("es-ES", "Spanisch"),
    ("nl-NL", "Niederländisch"),
    ("pl-PL", "Polnisch"),
    ("tr-TR", "Türkisch"),
];

/// Ein Strukturelement als Zeile der flachen Liste (Tiefensuche = Lesereihenfolge).
#[derive(Debug, Clone)]
struct Row {
    depth: usize,
    role: String,
    raw_role: String,
    id: Option<ObjectId>,
    alt: Option<String>,
    actual_text: Option<String>,
    content: Vec<(usize, i64)>,
    figure: bool,
    /// Ende des Teilbaums (exklusiv): Nachfahren sind `rows[i + 1..end]`.
    end: usize,
}

/// Wofür `rows` gilt: Dateibytes (Zeiger), Seitenreihenfolge, Alternativtexte.
type TreeKey = (usize, Vec<ObjectId>, BTreeMap<ObjectId, Option<String>>);

#[derive(Default)]
pub struct TagState {
    key: Option<TreeKey>,
    tagged: bool,
    rows: Vec<Row>,
    /// Markierte Inhalte je Seitenobjekt, mit Inhaltsstempel.
    marked: HashMap<ObjectId, (u64, BTreeMap<i64, MarkedContent>)>,
    pub selected: Option<usize>,
    /// Lesereihenfolge auf den Seiten einblenden.
    pub show_order: bool,
    /// Eingabefelder Titel/Sprache und der Stand, aus dem sie stammen.
    title: String,
    lang: String,
    props_for: Option<DocProperties>,
    /// Eingabe Alternativtext und für welche Zeile/welchen Wert sie gilt.
    alt_input: String,
    alt_for: Option<(usize, Option<String>)>,
    error: Option<String>,
    /// Knopf "Automatisch taggen…" gedrückt (die App öffnet den Dialog).
    pub request_autotag: bool,
}

impl OpenDocument {
    /// Baum neu lesen, falls sich etwas geändert hat.
    fn refresh_tags(&mut self) {
        let bytes_ptr = std::sync::Arc::as_ptr(self.doc.bytes()) as usize;
        let key: TreeKey = (
            bytes_ptr,
            self.doc.pages().iter().map(|p| p.object_id).collect(),
            self.doc.model().alt_texts.clone(),
        );
        let t = &mut self.tags;
        if t.key.as_ref() == Some(&key) {
            return;
        }
        if t.key.as_ref().is_none_or(|k| k.0 != bytes_ptr) {
            // Neue Datei-Bytes (Speichern, Ersetzen): alle markierten Inhalte verwerfen.
            t.marked.clear();
        }
        t.key = Some(key);
        t.tagged = self.doc.is_tagged();
        t.rows.clear();
        if let Some(tree) = self.doc.structure_tree() {
            let nodes = tree.nodes();
            for (i, (depth, n)) in nodes.iter().enumerate() {
                // Teilbaum endet beim nächsten Knoten mit gleicher oder kleinerer Tiefe.
                let end = nodes[i + 1..]
                    .iter()
                    .position(|(d, _)| d <= depth)
                    .map_or(nodes.len(), |p| i + 1 + p);
                t.rows.push(Row {
                    depth: *depth,
                    role: n.role.clone(),
                    raw_role: n.raw_role.clone(),
                    id: n.id,
                    alt: n.alt.clone(),
                    actual_text: n.actual_text.clone(),
                    content: n.content.clone(),
                    figure: n.is_figure(),
                    end,
                });
            }
        }
        if t.selected.is_some_and(|s| s >= t.rows.len()) {
            t.selected = None;
        }
    }

    /// Markierte Inhalte einer Seite (zwischengespeichert).
    fn page_marked(&mut self, page: usize) -> Option<&BTreeMap<i64, MarkedContent>> {
        let obj = self.doc.page(page).ok()?.object_id;
        let stamp = self.doc.content_stamp(obj);
        let fresh = self.tags.marked.get(&obj).is_some_and(|(s, _)| *s == stamp);
        if !fresh {
            // Fehler (z. B. eingefügte, ungespeicherte Seite) = keine markierten Inhalte.
            let m = self.doc.marked_content(page).unwrap_or_default();
            self.tags.marked.insert(obj, (stamp, m));
        }
        self.tags.marked.get(&obj).map(|(_, m)| m)
    }

    /// Text eines Strukturelements (nur eigene Inhalte, gekürzt).
    fn row_text(&mut self, i: usize) -> String {
        let row = &self.tags.rows[i];
        if let Some(a) = &row.actual_text {
            return a.clone();
        }
        let content = row.content.clone();
        let mut s = String::new();
        for (page, mcid) in content {
            if let Some(m) = self.page_marked(page).and_then(|m| m.get(&mcid)) {
                if !s.is_empty() && !m.text.is_empty() {
                    s.push(' ');
                }
                s.push_str(m.text.trim());
            }
            if s.chars().count() > 80 {
                break;
            }
        }
        s
    }

    /// Rechteck eines Elements samt Nachfahren auf der ersten Seite, auf der es Inhalt hat.
    fn row_bounds(&mut self, i: usize) -> Option<(usize, pdftool_core::Rect)> {
        let end = self.tags.rows[i].end;
        let content: Vec<(usize, i64)> = self.tags.rows[i..end]
            .iter()
            .flat_map(|r| r.content.iter().copied())
            .collect();
        let page = content.first()?.0;
        let marked = self.page_marked(page)?;
        content
            .iter()
            .filter(|(p, _)| *p == page)
            .filter_map(|(_, mcid)| marked.get(mcid)?.bounds)
            .reduce(|a, b| a.union(&b))
            .map(|r| (page, r))
    }

    fn select_tag(&mut self, i: usize) {
        self.tags.selected = Some(i);
        match self.row_bounds(i) {
            Some((page, rect)) => {
                if let Ok(info) = self.doc.page(page) {
                    let unit = transform::pdf_rect_to_unit(info, rect);
                    self.viewer.reveal(page, unit);
                }
            }
            None => {
                if let Some(&(page, _)) = self.tags.rows[i].content.first() {
                    self.viewer.scroll_to_page(page);
                }
            }
        }
    }

    fn execute_tag_command(&mut self, cmd: Box<dyn pdftool_core::command::Command>) {
        if let Err(e) = self.doc.execute(cmd) {
            self.tags.error = Some(e.to_string());
        }
    }

    /// Rechtes Panel "Tags".
    pub fn tags_panel(&mut self, ui: &mut egui::Ui) {
        self.refresh_tags();

        // ----- Dokument: Titel und Sprache -----
        let props = self.doc.properties().clone();
        if self.tags.props_for.as_ref() != Some(&props) {
            // Erstes Bild oder Änderung von außen (Undo): Eingaben neu füllen.
            self.tags.title = props.title.clone();
            self.tags.lang = props.lang.clone();
            self.tags.props_for = Some(props.clone());
        }
        let mut apply_props = false;
        egui::Grid::new("doc_props").num_columns(2).show(ui, |ui| {
            ui.label("Titel:");
            let r = ui.add(egui::TextEdit::singleline(&mut self.tags.title).desired_width(180.0));
            apply_props |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.end_row();
            ui.label("Sprache:");
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.tags.lang).desired_width(80.0));
                apply_props |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.menu_button("Auswahl", |ui| {
                    for (code, name) in LANGUAGES {
                        if ui.button(format!("{name} ({code})")).clicked() {
                            self.tags.lang = (*code).to_string();
                            apply_props = true;
                            ui.close();
                        }
                    }
                });
            });
            ui.end_row();
        });
        let wanted = DocProperties {
            title: self.tags.title.trim().to_string(),
            lang: self.tags.lang.trim().to_string(),
        };
        let changed = wanted != props;
        if ui
            .add_enabled(changed, egui::Button::new("Übernehmen"))
            .clicked()
        {
            apply_props = true;
        }
        if apply_props && changed {
            self.execute_tag_command(Box::new(SetProperties::new(props, wanted)));
        }
        ui.label(
            RichText::new("Der Titel wird in der Fensterleiste von PDF-Programmen angezeigt.")
                .weak(),
        );
        ui.separator();

        // ----- Struktur -----
        if self.tags.rows.is_empty() {
            ui.label("Dieses Dokument ist nicht getaggt (kein Strukturbaum).");
            ui.label(
                RichText::new(
                    "Screenreader lesen den Text dann nur in Seitenreihenfolge vor, \
                     Bilder haben keine Beschreibung.",
                )
                .weak(),
            );
            if ui.button("Automatisch taggen…").clicked() {
                self.tags.request_autotag = true;
            }
            self.show_tag_error(ui);
            return;
        }
        if !self.tags.tagged {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Strukturbaum vorhanden, aber das Dokument ist nicht als getaggt markiert (/MarkInfo).",
            );
        }
        let figures = self.tags.rows.iter().filter(|r| r.figure).count();
        let missing = self
            .tags
            .rows
            .iter()
            .filter(|r| r.figure && r.alt.is_none())
            .count();
        ui.horizontal(|ui| {
            ui.label(format!(
                "{} Elemente, {figures} Abbildungen",
                self.tags.rows.len()
            ));
            if missing > 0 {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{missing} ohne Alternativtext"),
                );
            }
        });
        ui.checkbox(
            &mut self.tags.show_order,
            "Lesereihenfolge auf den Seiten zeigen",
        );

        // Alternativtext der ausgewählten Abbildung (oben, damit er nicht wegscrollt).
        self.alt_editor(ui);
        self.show_tag_error(ui);
        ui.separator();

        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        let mut clicked = None;
        let mut next_missing = false;
        if missing > 0
            && ui
                .small_button("Nächste Abbildung ohne Alternativtext")
                .clicked()
        {
            next_missing = true;
        }
        let n = self.tags.rows.len();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_h, n, |ui, range| {
                for i in range {
                    let text = self.row_text(i);
                    let row = &self.tags.rows[i];
                    ui.horizontal(|ui| {
                        ui.add_space(row.depth as f32 * 12.0);
                        let role = if row.raw_role != row.role && !row.raw_role.is_empty() {
                            format!("{} → {}", row.raw_role, row.role)
                        } else {
                            row.role.clone()
                        };
                        let selected = self.tags.selected == Some(i);
                        let mut label = RichText::new(role).strong();
                        if row.figure && row.alt.is_none() && !selected {
                            label = label.color(ui.visuals().error_fg_color);
                        }
                        let r = ui.selectable_label(selected, label);
                        let detail = if row.figure {
                            match &row.alt {
                                Some(a) => format!("„{a}“"),
                                None => "kein Alternativtext".into(),
                            }
                        } else if text.is_empty()
                            && let Some(a) = &row.alt
                        {
                            // z. B. Links und Kommentare: ihre Beschreibung.
                            format!("„{a}“")
                        } else {
                            text
                        };
                        let detail = ui
                            .add(egui::Label::new(RichText::new(&detail).weak()).truncate())
                            .on_hover_text(&detail);
                        if r.clicked() || detail.clicked() {
                            clicked = Some(i);
                        }
                    });
                }
            });
        if next_missing {
            let start = self.tags.selected.map_or(0, |s| s + 1);
            let rows = &self.tags.rows;
            clicked = (start..rows.len())
                .chain(0..start)
                .find(|&i| rows[i].figure && rows[i].alt.is_none());
        }
        if let Some(i) = clicked {
            self.select_tag(i);
        }
    }

    fn show_tag_error(&mut self, ui: &mut egui::Ui) {
        if let Some(e) = &self.tags.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    }

    /// Eingabe des Alternativtexts für die ausgewählte Abbildung.
    fn alt_editor(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.tags.selected else { return };
        let row = &self.tags.rows[i];
        let (Some(id), true) = (row.id, row.figure) else {
            return;
        };
        let current = row.alt.clone();
        if self.tags.alt_for != Some((i, current.clone())) {
            self.tags.alt_input = current.clone().unwrap_or_default();
            self.tags.alt_for = Some((i, current.clone()));
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Alternativtext der Abbildung").strong());
        ui.add(
            egui::TextEdit::multiline(&mut self.tags.alt_input)
                .desired_rows(3)
                .desired_width(f32::INFINITY)
                .hint_text("Was zeigt das Bild? (wird vorgelesen)"),
        );
        let input = self.tags.alt_input.trim().to_string();
        let mut cmd = None;
        ui.horizontal(|ui| {
            let changed = Some(&input) != current.as_ref() && !input.is_empty();
            if ui
                .add_enabled(changed, egui::Button::new("Übernehmen"))
                .clicked()
            {
                cmd = Some(SetAltText::new(id, Some(input.clone())));
            }
            if ui
                .add_enabled(current.is_some(), egui::Button::new("Entfernen"))
                .clicked()
            {
                cmd = Some(SetAltText::new(id, None));
            }
        });
        if let Some(c) = cmd {
            self.execute_tag_command(Box::new(c));
        }
    }

    /// Overlay: ausgewähltes Element hervorheben, optional die Lesereihenfolge nummerieren.
    pub fn paint_tags(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        if self.tags.rows.is_empty() {
            return;
        }
        let painter = ui.painter_at(view.clip);
        let accent = Color32::from_rgb(40, 110, 220);
        if self.tags.show_order {
            // Nummer = Position in der Lesereihenfolge (nur Elemente mit eigenem Inhalt).
            let mut number = 0;
            let rows: Vec<(bool, Vec<(usize, i64)>)> = self
                .tags
                .rows
                .iter()
                .map(|r| (r.figure, r.content.clone()))
                .collect();
            for (figure, content) in rows {
                if content.is_empty() {
                    continue;
                }
                number += 1;
                for &(page, page_rect) in &view.pages {
                    let Some(marked) = self.page_marked(page) else {
                        continue;
                    };
                    let rect = content
                        .iter()
                        .filter(|(p, _)| *p == page)
                        .filter_map(|(_, mcid)| marked.get(mcid)?.bounds)
                        .reduce(|a, b| a.union(&b));
                    let (Some(rect), Ok(info)) = (rect, self.doc.page(page)) else {
                        continue;
                    };
                    let sr = transform::pdf_rect_to_screen(info, page_rect, rect).expand(1.0);
                    let color = if figure {
                        Color32::from_rgb(170, 60, 190)
                    } else {
                        accent
                    };
                    painter.rect_stroke(sr, 1.0, Stroke::new(1.0, color), StrokeKind::Outside);
                    let tag = egui::Rect::from_min_size(sr.left_top(), egui::vec2(20.0, 14.0))
                        .translate(egui::vec2(-21.0, 0.0));
                    painter.rect_filled(tag, 2.0, color);
                    painter.text(
                        tag.center(),
                        egui::Align2::CENTER_CENTER,
                        number.to_string(),
                        egui::FontId::proportional(10.0),
                        Color32::WHITE,
                    );
                }
            }
        }
        if let Some(i) = self.tags.selected
            && let Some((page, rect)) = self.row_bounds(i)
            && let Some(&(_, page_rect)) = view.pages.iter().find(|(p, _)| *p == page)
            && let Ok(info) = self.doc.page(page)
        {
            let sr = transform::pdf_rect_to_screen(info, page_rect, rect).expand(2.0);
            painter.rect_filled(sr, 2.0, Color32::from_rgba_unmultiplied(40, 110, 220, 40));
            painter.rect_stroke(sr, 2.0, Stroke::new(2.0, accent), StrokeKind::Outside);
        }
    }
}
