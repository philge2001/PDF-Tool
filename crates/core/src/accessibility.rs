//! Barrierefreiheit prüfen (Phase 9.4): ein Bericht in Anlehnung an PDF/UA und WCAG.
//!
//! Geprüft wird der aktuelle Stand des Dokuments (inklusive ungespeicherter Titel, Sprache und
//! Alternativtexte). Die Prüfung ist bewusst einfach und erklärt jede Meldung in Alltagssprache;
//! sie ersetzt keine vollständige PDF/UA-Validierung (z. B. mit veraPDF oder PAC).

use crate::document::Document;
use crate::error::Result;
use lopdf::{Dictionary, Object, ObjectId};
use std::collections::BTreeSet;

/// Ergebnis eines Prüfpunkts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

/// Was ein Prüfpunkt betrifft – damit die App passende Knöpfe anbieten kann.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    Tagged,
    Language,
    Title,
    AltText,
    Fonts,
    UntaggedContent,
    Scanned,
    TabOrder,
    Permissions,
    Bookmarks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub kind: CheckKind,
    pub status: CheckStatus,
    pub title: String,
    pub detail: String,
}

impl Check {
    fn new(kind: CheckKind, status: CheckStatus, title: &str, detail: impl Into<String>) -> Self {
        Self {
            kind,
            status,
            title: title.into(),
            detail: detail.into(),
        }
    }
}

/// Ab dieser Seitenzahl sollte ein Dokument Lesezeichen haben.
const BOOKMARK_PAGES: usize = 10;

/// Liste der Seiten als "1, 3, 7 …" (höchstens acht).
fn page_list(pages: &[usize]) -> String {
    let mut s: Vec<String> = pages.iter().take(8).map(|p| (p + 1).to_string()).collect();
    if pages.len() > 8 {
        s.push("…".into());
    }
    s.join(", ")
}

impl Document {
    /// Prüft das Dokument auf Barrierefreiheit. Reihenfolge = Wichtigkeit.
    pub fn accessibility_check(&self) -> Result<Vec<Check>> {
        use CheckKind as K;
        use CheckStatus::{Fail, Pass, Warn};
        let mut out = Vec::new();
        let tree = self.structure_tree();
        let tagged = self.is_tagged();

        // 1) Getaggt?
        out.push(match (tagged, &tree) {
            (true, _) => Check::new(
                K::Tagged,
                Pass,
                "Getaggtes PDF",
                "Das Dokument hat einen Strukturbaum.",
            ),
            (false, Some(_)) => Check::new(
                K::Tagged,
                Warn,
                "Getaggtes PDF",
                "Ein Strukturbaum ist vorhanden, aber das Dokument ist nicht als getaggt \
                 markiert (/MarkInfo).",
            ),
            (false, None) => Check::new(
                K::Tagged,
                Fail,
                "Getaggtes PDF",
                "Kein Strukturbaum: Screenreader kennen weder Überschriften noch Absätze oder \
                 Bilder. Abhilfe: „Automatisch taggen“.",
            ),
        });

        // 2) Sprache, 3) Titel.
        let props = self.properties();
        out.push(if props.lang.trim().is_empty() {
            Check::new(
                K::Language,
                Fail,
                "Dokumentsprache",
                "Keine Sprache angegeben – Screenreader wissen nicht, wie sie den Text \
                 aussprechen sollen.",
            )
        } else {
            Check::new(K::Language, Pass, "Dokumentsprache", props.lang.clone())
        });
        let display_title = *props != self.saved_properties() || self.display_doc_title();
        out.push(match (props.title.trim().is_empty(), display_title) {
            (true, _) => Check::new(
                K::Title,
                Fail,
                "Dokumenttitel",
                "Kein Titel – angezeigt wird nur der Dateiname.",
            ),
            (false, false) => Check::new(
                K::Title,
                Warn,
                "Dokumenttitel",
                format!(
                    "„{}“, aber PDF-Programme zeigen den Dateinamen statt des Titels \
                     (DisplayDocTitle). Titel einmal übernehmen behebt das.",
                    props.title
                ),
            ),
            (false, true) => Check::new(K::Title, Pass, "Dokumenttitel", props.title.clone()),
        });

        // 4) Alternativtexte.
        if let Some(tree) = &tree {
            let nodes = tree.nodes();
            let figures = nodes.iter().filter(|(_, n)| n.is_figure()).count();
            let missing = nodes
                .iter()
                .filter(|(_, n)| n.is_figure() && n.alt.is_none() && n.actual_text.is_none())
                .count();
            out.push(match (figures, missing) {
                (0, _) => Check::new(K::AltText, Pass, "Alternativtexte", "Keine Abbildungen."),
                (_, 0) => Check::new(
                    K::AltText,
                    Pass,
                    "Alternativtexte",
                    format!("Alle {figures} Abbildungen sind beschrieben."),
                ),
                _ => Check::new(
                    K::AltText,
                    Fail,
                    "Alternativtexte",
                    format!(
                        "{missing} von {figures} Abbildungen ohne Alternativtext \
                         (im Panel „Tags“ ergänzen)."
                    ),
                ),
            });
        }

        // 5) Schriften, 6) Inhalte ohne Tag, 7) Scans, 8) Tab-Reihenfolge – seitenweise.
        let mut not_embedded = BTreeSet::new();
        let mut font_seen = BTreeSet::new();
        let (mut untagged_text, mut untagged_images, mut untagged_other) = (0, 0, 0);
        let mut untagged_pages = Vec::new();
        let mut scanned = Vec::new();
        let mut tabs = Vec::new();
        let pdf = self.pdf();
        for (i, info) in self.pages().iter().enumerate() {
            if let Some(res) = crate::content::page_resources(pdf, info.object_id) {
                collect_fonts(pdf, res, &mut font_seen, &mut not_embedded, 0);
            }
            let Ok(pc) = self.page_content(i) else {
                continue; // eingefügte, ungespeicherte Seite
            };
            let it = self.interpret_content(info.object_id, &pc)?;
            let u = it.unmarked;
            if tagged && (u.text > 0 || u.images > 0 || u.other > 0) {
                untagged_text += u.text;
                untagged_images += u.images;
                untagged_other += u.other;
                untagged_pages.push(i);
            }
            let has_text = it.items.iter().any(|item| {
                matches!(item, crate::content::Item::Text(t)
                    if t.glyphs.iter().any(|g| !g.text.trim().is_empty()))
            });
            let has_image = it
                .items
                .iter()
                .any(|item| matches!(item, crate::content::Item::Image { .. }));
            if !has_text && has_image {
                scanned.push(i);
            }
            let annots = self.annotations().on_page(i).next().is_some()
                || pdf
                    .get_dictionary(info.object_id)
                    .ok()
                    .and_then(|d| d.get(b"Annots").ok())
                    .and_then(|o| pdf.dereference(o).ok())
                    .and_then(|(_, o)| o.as_array().ok())
                    .is_some_and(|a| !a.is_empty());
            let tabs_s = pdf
                .get_dictionary(info.object_id)
                .ok()
                .and_then(|d| d.get(b"Tabs").ok())
                .and_then(|o| o.as_name().ok())
                == Some(b"S");
            if annots && !tabs_s {
                tabs.push(i);
            }
        }
        out.push(if not_embedded.is_empty() {
            Check::new(
                K::Fonts,
                Pass,
                "Schriften eingebettet",
                "Alle Schriften sind eingebettet.",
            )
        } else {
            let names: Vec<_> = not_embedded.into_iter().collect();
            Check::new(
                K::Fonts,
                Fail,
                "Schriften eingebettet",
                format!(
                    "Nicht eingebettet: {}. Abhilfe: „Nach PDF/A umwandeln“ bettet \
                     Standardschriften ein.",
                    names.join(", ")
                ),
            )
        });
        if tagged {
            out.push(if untagged_pages.is_empty() {
                Check::new(
                    K::UntaggedContent,
                    Pass,
                    "Alle Inhalte getaggt",
                    "Jeder sichtbare Inhalt gehört zur Struktur oder ist als Artefakt markiert.",
                )
            } else {
                let status = if untagged_text + untagged_images > 0 {
                    Fail
                } else {
                    Warn
                };
                Check::new(
                    K::UntaggedContent,
                    status,
                    "Alle Inhalte getaggt",
                    format!(
                        "Ohne Tag: {untagged_text} Textstellen, {untagged_images} Bilder, \
                         {untagged_other} Grafiken (Seite {}). Screenreader überspringen sie.",
                        page_list(&untagged_pages)
                    ),
                )
            });
        }
        if !scanned.is_empty() {
            out.push(Check::new(
                K::Scanned,
                Fail,
                "Text statt Bild",
                format!(
                    "Seite {} besteht nur aus Bildern (Scan?). Abhilfe: Texterkennung (OCR).",
                    page_list(&scanned)
                ),
            ));
        }
        if !tabs.is_empty() {
            out.push(Check::new(
                K::TabOrder,
                Warn,
                "Tab-Reihenfolge",
                format!(
                    "Seite {} mit Links/Feldern/Kommentaren folgt nicht der Struktur (/Tabs /S).",
                    page_list(&tabs)
                ),
            ));
        }

        // 9) Rechte, 10) Lesezeichen.
        if let Some(s) = self.security()
            && !s.permissions.copy
        {
            out.push(Check::new(
                K::Permissions,
                Warn,
                "Zugriff für Hilfsprogramme",
                "Kopieren ist gesperrt – manche Screenreader respektieren das und lesen dann \
                 nichts vor.",
            ));
        }
        if self.page_count() >= BOOKMARK_PAGES {
            let has_outlines = pdf.catalog().ok().is_some_and(|c| c.has(b"Outlines"));
            out.push(Check::new(
                K::Bookmarks,
                if has_outlines { Pass } else { Warn },
                "Lesezeichen",
                if has_outlines {
                    "Vorhanden."
                } else {
                    "Längere Dokumente sollten Lesezeichen zur Navigation haben."
                },
            ));
        }
        Ok(out)
    }

    /// Steht /ViewerPreferences /DisplayDocTitle true in der Datei?
    fn display_doc_title(&self) -> bool {
        let pdf = self.pdf();
        pdf.catalog()
            .ok()
            .and_then(|c| c.get(b"ViewerPreferences").ok())
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .and_then(|d| d.get(b"DisplayDocTitle").ok())
            .and_then(|o| o.as_bool().ok())
            .unwrap_or(false)
    }
}

/// Sammelt nicht eingebettete Schriften aus Ressourcen (Formular-XObjects rekursiv).
fn collect_fonts(
    pdf: &lopdf::Document,
    res: &Dictionary,
    seen: &mut BTreeSet<ObjectId>,
    missing: &mut BTreeSet<String>,
    depth: usize,
) {
    // Eigene Funktion statt Closure: So kann der Rückgabewert die Lebensdauer von `pdf`
    // tragen (bei Closures leitet Rust sie nicht ab).
    fn deref<'a>(
        pdf: &'a lopdf::Document,
        o: &'a Object,
    ) -> Option<(Option<ObjectId>, &'a Dictionary)> {
        let id = o.as_reference().ok();
        Some((id, pdf.dereference(o).ok()?.1.as_dict().ok()?))
    }
    if let Some((_, fonts)) = res.get(b"Font").ok().and_then(|o| deref(pdf, o)) {
        for (_, f) in fonts.iter() {
            let Some((id, font)) = deref(pdf, f) else {
                continue;
            };
            if let Some(id) = id
                && !seen.insert(id)
            {
                continue;
            }
            let subtype = font
                .get(b"Subtype")
                .and_then(Object::as_name)
                .unwrap_or(b"");
            let embedded = match subtype {
                b"Type3" => true,
                b"Type0" => font
                    .get(b"DescendantFonts")
                    .ok()
                    .and_then(|o| pdf.dereference(o).ok())
                    .and_then(|(_, o)| o.as_array().ok())
                    .and_then(|a| a.first())
                    .and_then(|o| deref(pdf, o))
                    .is_some_and(|(_, d)| crate::pdfa::has_font_file(pdf, d)),
                _ => crate::pdfa::has_font_file(pdf, font),
            };
            if !embedded {
                let name = font
                    .get(b"BaseFont")
                    .and_then(Object::as_name)
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_else(|_| "(ohne Namen)".into());
                missing.insert(name);
            }
        }
    }
    if depth > 8 {
        return;
    }
    if let Some((_, xobjects)) = res.get(b"XObject").ok().and_then(|o| deref(pdf, o)) {
        for (_, x) in xobjects.iter() {
            let Some(id) = x.as_reference().ok() else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            let Ok(stream) = pdf.get_object(id).and_then(Object::as_stream) else {
                continue;
            };
            if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Form")
                && let Some((_, r)) = stream
                    .dict
                    .get(b"Resources")
                    .ok()
                    .and_then(|o| deref(pdf, o))
            {
                collect_fonts(pdf, r, seen, missing, depth + 1);
            }
        }
    }
}
