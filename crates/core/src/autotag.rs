//! Automatisches Grund-Tagging für ungetaggte Dokumente (Phase 9.4).
//!
//! Heuristik, keine Magie: Aus dem Seitenlayout (Interpreter + `paragraph::group_paragraphs`)
//! entstehen Absätze (`P`) und Überschriften (`H1`–`H3`, deutlich größere Schrift als der
//! Fließtext), Bilder werden Abbildungen (`Figure`, Alternativtext fehlt noch – der kommt im
//! Panel „Tags“ dazu). Alles andere (Linien, Flächen, Kopf- und Fußzeilen, die sich auf vielen
//! Seiten wiederholen) wird als Artefakt markiert und von Screenreadern übergangen.
//!
//! Technisch: Jede Seite bekommt einen neuen Content-Stream, in dem jede Textausgabe bzw.
//! jedes Bild in `/P <</MCID n>> BDC … EMC` steht (Pfade in `/Artifact BMC … EMC`). Dazu
//! kommen Strukturelemente, `/ParentTree` (MCID → Element), `/StructParents` an der Seite,
//! Annotationen als `Link`/`Annot`/`Form` mit OBJR, `/MarkInfo`, `/Lang`, Titel und
//! `/Tabs /S`. Die Datei wird komplett neu geschrieben.
//!
//! Die Lesereihenfolge ist die Reihenfolge im Content-Stream – bei den meisten Erzeugern
//! entspricht das der Leserichtung, bei mehrspaltigen Layouts nicht immer.

use crate::content::{self, Interpreted, Item};
use crate::error::{CoreError, Result};
use crate::paragraph::{Paragraph, group_paragraphs};
use crate::pdf_string;
use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream, dictionary};
use std::collections::HashMap;

/// Vorgaben für das Tagging.
#[derive(Debug, Clone, Default)]
pub struct AutoTagOptions {
    /// Dokumentsprache (BCP 47), leer = nicht setzen.
    pub lang: String,
    /// Titel, leer = erste Überschrift verwenden (falls vorhanden).
    pub title: String,
    /// Nicht eingebettete Standardschriften durch Liberation ersetzen (wie bei PDF/A).
    pub embed_fonts: bool,
}

/// Was getaggt wurde.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoTagReport {
    pub pages: usize,
    pub headings: usize,
    pub paragraphs: usize,
    pub figures: usize,
    pub annotations: usize,
    /// Kopf-/Fußzeilen (Textabschnitte), die als Artefakt markiert wurden.
    pub running: usize,
    /// Titel, der gesetzt wurde (leer = keiner).
    pub title: String,
    /// Weitere Meldungen (eingebettete Schriften, Probleme).
    pub notes: Vec<String>,
}

impl AutoTagReport {
    pub fn summary(&self) -> String {
        let mut s = format!(
            "{} Seite(n) getaggt: {} Überschrift(en), {} Absätze, {} Abbildung(en)",
            self.pages, self.headings, self.paragraphs, self.figures
        );
        if self.annotations > 0 {
            s.push_str(&format!(
                ", {} Link(s)/Formularfeld(er)/Kommentar(e)",
                self.annotations
            ));
        }
        s.push('.');
        if self.running > 0 {
            s.push_str(&format!(
                "\n{} Kopf-/Fußzeilen als Artefakt markiert.",
                self.running
            ));
        }
        if !self.title.is_empty() {
            s.push_str(&format!("\nTitel: „{}“", self.title));
        }
        for n in &self.notes {
            s.push('\n');
            s.push_str(n);
        }
        if self.figures > 0 {
            s.push_str("\nBitte im Panel „Tags“ die Alternativtexte der Abbildungen ergänzen.");
        }
        s
    }
}

/// Rolle eines Strukturelements.
#[derive(Debug, Clone, PartialEq)]
enum Role {
    Heading(u8),
    P,
    Figure,
}

impl Role {
    fn name(&self) -> String {
        match self {
            Role::Heading(l) => format!("H{l}"),
            Role::P => "P".into(),
            Role::Figure => "Figure".into(),
        }
    }
}

/// Wie eine Operation markiert wird.
#[derive(Debug, Clone, Copy)]
enum Mark {
    /// Inhalt des Elements mit diesem Index (auf der Seite).
    Elem(usize),
    Artifact,
}

struct Elem {
    role: Role,
    /// Erste Operation (für die Reihenfolge).
    first_op: usize,
    mcids: Vec<i64>,
}

/// Seite nach dem ersten Durchgang.
struct PageScan {
    id: ObjectId,
    it: Interpreted,
    paras: Vec<Paragraph>,
    crop_height: f32,
    crop_y0: f32,
}

/// Grenze für Kopf-/Fußzeilen: oberstes bzw. unterstes Stück der Seite.
const RUNNING_BAND: f32 = 0.08;

fn running_key(p: &Paragraph) -> String {
    p.text
        .chars()
        .filter(|c| !c.is_ascii_digit())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn in_band(p: &Paragraph, scan: &PageScan) -> bool {
    let top = scan.crop_y0 + scan.crop_height * (1.0 - RUNNING_BAND);
    let bottom = scan.crop_y0 + scan.crop_height * RUNNING_BAND;
    p.bounds.y0 >= top || p.bounds.y1 <= bottom
}

/// Taggt ein ungetaggtes Dokument. `progress(Anteil, Text)` liefert `false` = abbrechen.
pub fn auto_tag_pdf(
    bytes: &[u8],
    opts: &AutoTagOptions,
    progress: &dyn Fn(f32, &str) -> bool,
) -> Result<(Vec<u8>, AutoTagReport)> {
    let doc = crate::Document::from_bytes(bytes.to_vec())?;
    if doc.pdf().is_encrypted() {
        return Err(CoreError::Other(
            "Verschlüsselte Dateien zuerst entschlüsseln.".into(),
        ));
    }
    if doc.pdf().catalog().is_ok_and(|c| c.has(b"StructTreeRoot")) {
        return Err(CoreError::Other(
            "Das Dokument ist bereits getaggt (Strukturbaum vorhanden).".into(),
        ));
    }
    let mut report = AutoTagReport::default();

    // ----- 1) Seiten lesen, Absätze bilden -----
    let n = doc.page_count();
    let mut scans = Vec::with_capacity(n);
    for (i, info) in doc.pages().iter().enumerate() {
        if !progress(0.6 * i as f32 / n.max(1) as f32, "Seiten analysieren …") {
            return Err(CoreError::Other("Abgebrochen.".into()));
        }
        let it = content::interpret_page(doc.pdf(), info.object_id)?;
        let paras = group_paragraphs(&it, 0);
        scans.push(PageScan {
            id: info.object_id,
            it,
            paras,
            crop_height: info.crop_box.height(),
            crop_y0: info.crop_box.y0,
        });
    }

    // Kopf-/Fußzeilen: gleicher Text (ohne Ziffern) im Randbereich auf vielen Seiten.
    let mut running_count: HashMap<String, usize> = HashMap::new();
    for s in &scans {
        let mut keys: Vec<String> = s
            .paras
            .iter()
            .filter(|p| in_band(p, s))
            .map(running_key)
            .collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            *running_count.entry(k).or_default() += 1;
        }
    }
    let is_running = |p: &Paragraph, s: &PageScan| {
        n >= 2
            && in_band(p, s)
            && running_count
                .get(&running_key(p))
                .is_some_and(|&c| c >= 2 && c * 2 >= n)
    };

    // Fließtextgröße = Schriftgröße mit den meisten Zeichen; Überschriften deutlich größer.
    let size_key = |p: &Paragraph| (p.size_on_page * 2.0).round() as i32;
    let mut chars_by_size: HashMap<i32, usize> = HashMap::new();
    for s in &scans {
        for p in &s.paras {
            *chars_by_size.entry(size_key(p)).or_default() += p.text.chars().count();
        }
    }
    let body = chars_by_size
        .iter()
        .max_by_key(|(k, c)| (**c, -**k))
        .map_or(0, |(k, _)| *k);
    let is_heading = |p: &Paragraph| {
        let len = p.text.chars().count();
        size_key(p) as f32 >= body as f32 * 1.15
            && p.lines <= 3
            && (1..=150).contains(&len)
            && p.text.chars().any(char::is_alphabetic)
    };
    let mut heading_sizes: Vec<i32> = scans
        .iter()
        .flat_map(|s| s.paras.iter())
        .filter(|p| is_heading(p))
        .map(size_key)
        .collect();
    heading_sizes.sort_unstable_by(|a, b| b.cmp(a));
    heading_sizes.dedup();
    let level = |p: &Paragraph| -> u8 {
        let rank = heading_sizes
            .iter()
            .position(|s| *s == size_key(p))
            .unwrap_or(2);
        (rank.min(2) + 1) as u8
    };

    // ----- 2) Seiten neu schreiben, Strukturelemente anlegen -----
    let mut pdf = doc.pdf().clone();
    let root_id = pdf.new_object_id();
    let document_id = pdf.new_object_id();
    let mut document_kids: Vec<Object> = Vec::new();
    let mut parent_tree: Vec<Object> = Vec::new();
    let mut next_key = scans.len() as i64;
    let mut first_heading: Option<String> = None;

    for (page_idx, scan) in scans.iter().enumerate() {
        if !progress(
            0.6 + 0.35 * page_idx as f32 / n.max(1) as f32,
            "Tags schreiben …",
        ) {
            return Err(CoreError::Other("Abgebrochen.".into()));
        }
        let mut elems: Vec<Elem> = Vec::new();
        let mut marks: HashMap<usize, Mark> = HashMap::new();
        // Pfade: Start-Op -> Mal-Op.
        let mut paths: HashMap<usize, usize> = HashMap::new();

        // Absätze -> Elemente.
        let mut para_elem: HashMap<usize, Mark> = HashMap::new();
        for p in &scan.paras {
            let mark = if is_running(p, scan) {
                report.running += 1;
                Mark::Artifact
            } else {
                let role = if is_heading(p) {
                    report.headings += 1;
                    if first_heading.is_none() {
                        first_heading = Some(p.text.clone());
                    }
                    Role::Heading(level(p))
                } else {
                    report.paragraphs += 1;
                    Role::P
                };
                elems.push(Elem {
                    role,
                    first_op: p.shows.iter().copied().min().unwrap_or(0),
                    mcids: Vec::new(),
                });
                Mark::Elem(elems.len() - 1)
            };
            for &op in &p.shows {
                para_elem.insert(op, mark);
            }
        }
        for item in &scan.it.items {
            match item {
                Item::Text(t) => {
                    let m = para_elem.get(&t.op).copied().unwrap_or(Mark::Artifact);
                    marks.entry(t.op).or_insert(m);
                }
                Item::Image { op, bounds, .. } => {
                    // Winzige Bilder (Linien, Punkte) sind Dekoration.
                    let m = if bounds.width() * bounds.height() < 16.0 {
                        Mark::Artifact
                    } else {
                        report.figures += 1;
                        elems.push(Elem {
                            role: Role::Figure,
                            first_op: *op,
                            mcids: Vec::new(),
                        });
                        Mark::Elem(elems.len() - 1)
                    };
                    marks.insert(*op, m);
                }
                Item::Form { op, id, matrix, .. } => {
                    let m = match form_role(&pdf, *id, *matrix) {
                        Some(role) => {
                            match role {
                                Role::Figure => report.figures += 1,
                                _ => report.paragraphs += 1,
                            }
                            elems.push(Elem {
                                role,
                                first_op: *op,
                                mcids: Vec::new(),
                            });
                            Mark::Elem(elems.len() - 1)
                        }
                        None => Mark::Artifact,
                    };
                    marks.insert(*op, m);
                }
                Item::Path { start, paint, .. } => {
                    paths.insert(*start, *paint);
                }
                Item::Shading { op } => {
                    marks.insert(*op, Mark::Artifact);
                }
            }
        }

        // Neue Operationen: jede markierte Operation einzeln einklammern.
        let mut ops: Vec<Operation> = Vec::with_capacity(scan.it.ops.len() * 2);
        let mut mcid_elem: Vec<usize> = Vec::new(); // Index = MCID
        let mut path_end: Option<usize> = None;
        for (i, op) in scan.it.ops.iter().enumerate() {
            if path_end.is_none()
                && let Some(&end) = paths.get(&i)
            {
                ops.push(Operation::new(
                    "BMC",
                    vec![Object::Name(b"Artifact".to_vec())],
                ));
                path_end = Some(end);
            }
            match marks.get(&i) {
                Some(Mark::Elem(e)) if path_end.is_none() => {
                    let mcid = mcid_elem.len() as i64;
                    mcid_elem.push(*e);
                    elems[*e].mcids.push(mcid);
                    ops.push(Operation::new(
                        "BDC",
                        vec![
                            Object::Name(elems[*e].role.name().into_bytes()),
                            Object::Dictionary(dictionary! { "MCID" => mcid }),
                        ],
                    ));
                    ops.push(op.clone());
                    ops.push(Operation::new("EMC", vec![]));
                }
                Some(_) if path_end.is_none() => {
                    ops.push(Operation::new(
                        "BMC",
                        vec![Object::Name(b"Artifact".to_vec())],
                    ));
                    ops.push(op.clone());
                    ops.push(Operation::new("EMC", vec![]));
                }
                _ => ops.push(op.clone()),
            }
            if path_end == Some(i) {
                ops.push(Operation::new("EMC", vec![]));
                path_end = None;
            }
        }
        if path_end.is_some() {
            ops.push(Operation::new("EMC", vec![]));
        }
        let data = Content { operations: ops }.encode()?;
        let mut stream = Stream::new(Dictionary::new(), data);
        let _ = stream.compress();
        let content_id = pdf.add_object(stream);

        // Strukturelemente in Inhaltsreihenfolge.
        let mut order: Vec<usize> = (0..elems.len())
            .filter(|&e| !elems[e].mcids.is_empty())
            .collect();
        order.sort_by_key(|&e| elems[e].first_op);
        let mut elem_ids: HashMap<usize, ObjectId> = HashMap::new();
        for e in order {
            let el = &elems[e];
            let k = if el.mcids.len() == 1 {
                Object::Integer(el.mcids[0])
            } else {
                Object::Array(el.mcids.iter().map(|m| Object::Integer(*m)).collect())
            };
            let d = dictionary! {
                "Type" => "StructElem",
                "S" => Object::Name(el.role.name().into_bytes()),
                "P" => document_id,
                "Pg" => scan.id,
                "K" => k,
            };
            let id = pdf.add_object(d);
            elem_ids.insert(e, id);
            document_kids.push(Object::Reference(id));
        }
        parent_tree.push(Object::Integer(page_idx as i64));
        parent_tree.push(Object::Array(
            mcid_elem
                .iter()
                .map(|e| Object::Reference(elem_ids[e]))
                .collect(),
        ));

        // Annotationen: Link/Form/Annot mit Objektverweis (OBJR).
        let annots: Vec<ObjectId> = pdf
            .get_dictionary(scan.id)
            .ok()
            .and_then(|d| d.get(b"Annots").ok())
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_array().ok())
            .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        let mut has_annots = false;
        for a in annots {
            let Ok(dict) = pdf.get_dictionary(a) else {
                continue;
            };
            let role: &[u8] = match dict.get(b"Subtype").and_then(Object::as_name) {
                Ok(b"Popup") => continue,
                Ok(b"Link") => b"Link",
                Ok(b"Widget") => b"Form",
                _ => b"Annot",
            };
            // Ausgeblendete Annotationen (Flag 2) brauchen keinen Tag.
            if dict.get(b"F").and_then(Object::as_i64).unwrap_or(0) & 2 != 0 {
                continue;
            }
            let key = next_key;
            next_key += 1;
            let mut elem = dictionary! {
                "Type" => "StructElem",
                "S" => Object::Name(role.to_vec()),
                "P" => document_id,
                "Pg" => scan.id,
                "K" => dictionary! { "Type" => "OBJR", "Obj" => a, "Pg" => scan.id },
            };
            // PDF/UA: Annotationen brauchen eine Beschreibung (/Contents oder /Alt).
            let alt = annot_description(&pdf, dict);
            let is_link = role == b"Link";
            if let Some(alt) = &alt {
                elem.set("Alt", pdf_string::encode(alt));
            }
            let elem = pdf.add_object(elem);
            if let Ok(d) = pdf.get_object_mut(a).and_then(Object::as_dict_mut) {
                // Links brauchen die Beschreibung zusätzlich in /Contents.
                if is_link
                    && !d.has(b"Contents")
                    && let Some(alt) = &alt
                {
                    d.set("Contents", pdf_string::encode(alt));
                }
            }
            if let Ok(d) = pdf.get_object_mut(a).and_then(Object::as_dict_mut) {
                d.set("StructParent", key);
            }
            document_kids.push(Object::Reference(elem));
            parent_tree.push(Object::Integer(key));
            parent_tree.push(Object::Reference(elem));
            report.annotations += 1;
            has_annots = true;
        }

        let page = pdf.get_object_mut(scan.id).and_then(Object::as_dict_mut)?;
        page.set("Contents", content_id);
        page.set("StructParents", page_idx as i64);
        if has_annots {
            page.set("Tabs", Object::Name(b"S".to_vec()));
        }
        report.pages += 1;
    }

    // ----- 3) Strukturbaum, Katalog, Titel -----
    let parent_tree_id = pdf.add_object(dictionary! { "Nums" => parent_tree });
    pdf.objects.insert(
        document_id,
        Object::Dictionary(dictionary! {
            "Type" => "StructElem",
            "S" => "Document",
            "P" => root_id,
            "K" => document_kids,
        }),
    );
    pdf.objects.insert(
        root_id,
        Object::Dictionary(dictionary! {
            "Type" => "StructTreeRoot",
            "K" => document_id,
            "ParentTree" => parent_tree_id,
            "ParentTreeNextKey" => next_key,
        }),
    );
    let title = if opts.title.trim().is_empty() {
        first_heading
            .map(|t| t.chars().take(120).collect::<String>())
            .unwrap_or_default()
    } else {
        opts.title.trim().to_string()
    };
    let cat_id = pdf.trailer.get(b"Root")?.as_reference()?;
    let viewer_prefs = {
        let cat = pdf.get_dictionary(cat_id)?;
        let mut vp = cat
            .get(b"ViewerPreferences")
            .ok()
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok())
            .cloned()
            .unwrap_or_default();
        if !title.is_empty() {
            vp.set("DisplayDocTitle", true);
        }
        vp
    };
    let cat = pdf.get_object_mut(cat_id).and_then(Object::as_dict_mut)?;
    cat.set("StructTreeRoot", root_id);
    cat.set("MarkInfo", dictionary! { "Marked" => true });
    if !opts.lang.trim().is_empty() {
        cat.set("Lang", pdf_string::encode(opts.lang.trim()));
    }
    if !viewer_prefs.is_empty() {
        cat.set("ViewerPreferences", viewer_prefs);
    }
    if !title.is_empty() {
        set_info_title(&mut pdf, &title)?;
        report.title = title;
    }

    // XMP-Metadaten mit Titel und PDF/UA-Kennung (PDF/UA verlangt beides); eine
    // PDF/A-2-Kennung bleibt erhalten.
    let was_pdfa2 = pdf
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Metadata").ok())
        .and_then(|o| pdf.dereference(o).ok())
        .and_then(|(_, o)| o.as_stream().ok())
        .map(|s| {
            s.decompressed_content()
                .unwrap_or_else(|_| s.content.clone())
        })
        .is_some_and(|x| {
            let x = String::from_utf8_lossy(&x);
            x.contains("<pdfaid:part>2<") || x.contains("pdfaid:part=\"2\"")
        });
    crate::pdfa::write_metadata(&mut pdf, was_pdfa2, true)?;

    if opts.embed_fonts {
        crate::pdfa::promote_direct_fonts(&mut pdf);
        let mut problems = Vec::new();
        crate::pdfa::embed_standard_fonts(&mut pdf, &mut report.notes, &mut problems)?;
        report.notes.extend(problems);
    }

    progress(0.97, "Speichern …");
    pdf.prune_objects();
    pdf.compress();
    let mut out = Vec::new();
    pdf.save_to(&mut out)
        .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
    Ok((out, report))
}

/// Rolle eines Formular-XObjects: Text -> `P`, Bild -> `Figure`, sonst Artefakt.
fn form_role(pdf: &lopdf::Document, id: Option<ObjectId>, matrix: content::Matrix) -> Option<Role> {
    let stream = pdf.get_object(id?).ok()?.as_stream().ok()?;
    let it = content::interpret_form(pdf, stream, None, matrix).ok()?;
    let has_text = it
        .items
        .iter()
        .any(|i| matches!(i, Item::Text(t) if t.glyphs.iter().any(|g| !g.text.trim().is_empty())));
    if has_text {
        return Some(Role::P);
    }
    it.items
        .iter()
        .any(
            |i| matches!(i, Item::Image { bounds, .. } if bounds.width() * bounds.height() >= 16.0),
        )
        .then_some(Role::Figure)
}

/// Beschreibung einer Annotation für Screenreader: /Contents, bei Formularfeldern /TU
/// (sonst der Feldname), bei Links das Ziel. `None`, wenn /Contents schon reicht.
fn annot_description(pdf: &lopdf::Document, annot: &Dictionary) -> Option<String> {
    let contents = pdf_string::get(annot, b"Contents");
    let subtype = annot.get(b"Subtype").and_then(Object::as_name).ok()?;
    match subtype {
        b"Widget" => {
            // /TU bzw. /T stehen am Widget oder an einem Elternfeld.
            let mut d = annot;
            for _ in 0..16 {
                for key in [&b"TU"[..], b"T"] {
                    let v = pdf_string::get(d, key);
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
                d = d
                    .get(b"Parent")
                    .ok()
                    .and_then(|o| pdf.dereference(o).ok())
                    .and_then(|(_, o)| o.as_dict().ok())?;
            }
            None
        }
        _ if !contents.is_empty() => Some(contents),
        b"Link" => {
            let uri = annot
                .get(b"A")
                .ok()
                .and_then(|o| pdf.dereference(o).ok())
                .and_then(|(_, o)| o.as_dict().ok())
                .map(|a| pdf_string::get(a, b"URI"))
                .unwrap_or_default();
            Some(if uri.is_empty() {
                "Verweis im Dokument".into()
            } else {
                format!("Link: {uri}")
            })
        }
        other => Some(format!("Kommentar ({})", String::from_utf8_lossy(other))),
    }
}

/// Titel in /Info eintragen (Info-Dictionary anlegen, falls nötig).
fn set_info_title(pdf: &mut lopdf::Document, title: &str) -> Result<()> {
    let info_id = match pdf.trailer.get(b"Info").and_then(Object::as_reference) {
        Ok(id) => id,
        Err(_) => {
            if let Ok(Object::Dictionary(d)) = pdf.trailer.get_mut(b"Info") {
                d.set("Title", pdf_string::encode(title));
                return Ok(());
            }
            let id = pdf.add_object(Dictionary::new());
            pdf.trailer.set("Info", id);
            id
        }
    };
    let info = pdf.get_object_mut(info_id).and_then(Object::as_dict_mut)?;
    info.set("Title", pdf_string::encode(title));
    Ok(())
}
