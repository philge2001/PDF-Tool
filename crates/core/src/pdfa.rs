//! Umwandlung nach PDF/A-2b – „bestmöglich“ (Phase 8).
//!
//! PDF/A ist PDF für die Langzeitarchivierung: alles Nötige steckt in der Datei, nichts
//! Dynamisches. Wir beheben, was sich sicher beheben lässt:
//! - **Schriften einbetten:** Nicht eingebettete Standardschriften (Helvetica, Times, Courier
//!   samt Varianten, auch Arial/Times New Roman/Courier New) werden durch die metrisch
//!   kompatiblen Liberation-Schriften (OFL) ersetzt und vollständig eingebettet.
//! - **Farben:** sRGB-OutputIntent (ICC-Profil, CC0).
//! - **Metadaten:** XMP mit `pdfaid:part=2`, `conformance=B`, abgeglichen mit /Info.
//! - **Verboten/unerwünscht:** JavaScript, eingebettete Dateien, /AA, Transferfunktionen,
//!   LZW-Kompression, /Interpolate, /NeedAppearances; Annotationen werden druckbar.
//! - **Dateistruktur:** Version 1.7, /ID im Trailer, komplett neu geschrieben.
//!
//! Was sich nicht automatisch lösen lässt (z. B. Symbol-Schriften, CMYK ohne passendes Profil,
//! Annotationen ohne Erscheinungsbild), steht als Problem im Bericht.

use crate::date::{pdf_date_now, pdf_date_to_xmp};
use crate::error::{CoreError, Result};
use crate::pdf_string;
use crate::sanitize::{SanitizeOptions, sanitize_pdf};
use lopdf::{Dictionary, Object, ObjectId, Stream, StringFormat, dictionary};
use std::collections::{BTreeSet, HashMap};

/// sRGB-Profil (Compact-ICC-Profiles, CC0).
static SRGB_ICC: &[u8] = include_bytes!("../../../assets/icc/sRGB-v2-magic.icc");

/// Liberation-Schriften (SIL OFL 1.1): (Datei-Kennung, Daten).
static LIBERATION: &[(&str, &[u8])] = &[
    (
        "LiberationSans-Regular",
        include_bytes!("../../../assets/fonts/liberation/LiberationSans-Regular.ttf"),
    ),
    (
        "LiberationSans-Bold",
        include_bytes!("../../../assets/fonts/liberation/LiberationSans-Bold.ttf"),
    ),
    (
        "LiberationSans-Italic",
        include_bytes!("../../../assets/fonts/liberation/LiberationSans-Italic.ttf"),
    ),
    (
        "LiberationSans-BoldItalic",
        include_bytes!("../../../assets/fonts/liberation/LiberationSans-BoldItalic.ttf"),
    ),
    (
        "LiberationSerif-Regular",
        include_bytes!("../../../assets/fonts/liberation/LiberationSerif-Regular.ttf"),
    ),
    (
        "LiberationSerif-Bold",
        include_bytes!("../../../assets/fonts/liberation/LiberationSerif-Bold.ttf"),
    ),
    (
        "LiberationSerif-Italic",
        include_bytes!("../../../assets/fonts/liberation/LiberationSerif-Italic.ttf"),
    ),
    (
        "LiberationSerif-BoldItalic",
        include_bytes!("../../../assets/fonts/liberation/LiberationSerif-BoldItalic.ttf"),
    ),
    (
        "LiberationMono-Regular",
        include_bytes!("../../../assets/fonts/liberation/LiberationMono-Regular.ttf"),
    ),
    (
        "LiberationMono-Bold",
        include_bytes!("../../../assets/fonts/liberation/LiberationMono-Bold.ttf"),
    ),
    (
        "LiberationMono-Italic",
        include_bytes!("../../../assets/fonts/liberation/LiberationMono-Italic.ttf"),
    ),
    (
        "LiberationMono-BoldItalic",
        include_bytes!("../../../assets/fonts/liberation/LiberationMono-BoldItalic.ttf"),
    ),
];

/// Ergebnis der Umwandlung.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfaReport {
    /// Was geändert wurde (für den Nutzer).
    pub fixed: Vec<String>,
    /// Was nicht automatisch behoben werden konnte – die Datei ist dann evtl. nicht konform.
    pub problems: Vec<String>,
}

impl PdfaReport {
    pub fn summary(&self) -> String {
        let mut s = String::new();
        if self.fixed.is_empty() {
            s.push_str("Nichts zu ändern.\n");
        } else {
            s.push_str("Angepasst:\n");
            for f in &self.fixed {
                s.push_str(&format!("• {f}\n"));
            }
        }
        if !self.problems.is_empty() {
            s.push_str("\nNicht automatisch lösbar (Datei ist evtl. nicht PDF/A-konform):\n");
            for p in &self.problems {
                s.push_str(&format!("• {p}\n"));
            }
        }
        s
    }
}

/// Welche Liberation-Schrift ersetzt eine (nicht eingebettete) Schrift? `None` = keine
/// passende (z. B. Symbol). Liefert außerdem, ob die Breiten verlässlich passen.
fn replacement(base_font: &str) -> Option<(&'static str, &'static [u8], bool)> {
    // Teilmengen-Präfix "ABCDEF+" abtrennen, Schreibweisen vereinheitlichen.
    let name = base_font.rsplit('+').next().unwrap_or(base_font);
    let lower = name.to_ascii_lowercase().replace([' ', '-', ','], "");
    if lower.contains("symbol") || lower.contains("dingbat") {
        return None;
    }
    let family = if lower.starts_with("courier") {
        "Mono"
    } else if lower.starts_with("times") {
        "Serif"
    } else {
        "Sans"
    };
    // Standardschriften und ihre bekannten Entsprechungen: Breiten identisch.
    let known = ["helvetica", "arial", "times", "courier"]
        .iter()
        .any(|k| lower.starts_with(k));
    let bold = lower.contains("bold") || lower.contains("black") || lower.contains("heavy");
    let italic = lower.contains("italic") || lower.contains("oblique");
    let style = match (bold, italic) {
        (true, true) => "BoldItalic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (false, false) => "Regular",
    };
    let key = format!("Liberation{family}-{style}");
    LIBERATION
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(k, data)| (*k, *data, known))
}

pub(crate) fn has_font_file(pdf: &lopdf::Document, font: &Dictionary) -> bool {
    font.get(b"FontDescriptor")
        .ok()
        .and_then(|o| pdf.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok())
        .is_some_and(|fd| {
            [&b"FontFile"[..], b"FontFile2", b"FontFile3"]
                .iter()
                .any(|k| fd.has(k))
        })
}

/// Bettet nicht eingebettete Standardschriften als Liberation ein (auch für das
/// automatische Taggen: PDF/UA verlangt ebenfalls eingebettete Schriften).
pub(crate) fn embed_standard_fonts(
    pdf: &mut lopdf::Document,
    fixed: &mut Vec<String>,
    problems: &mut Vec<String>,
) -> Result<()> {
    let mut files: HashMap<String, ObjectId> = HashMap::new();
    let mut replaced: BTreeSet<String> = BTreeSet::new();
    let mut approximated: BTreeSet<String> = BTreeSet::new();
    let font_ids: Vec<ObjectId> = pdf
        .objects
        .iter()
        .filter_map(|(id, o)| {
            let d = o.as_dict().ok()?;
            (d.get(b"Type").and_then(Object::as_name).ok() == Some(b"Font")).then_some(*id)
        })
        .collect();
    for id in font_ids {
        let Ok(font) = pdf.get_dictionary(id).cloned() else {
            continue;
        };
        let subtype = font
            .get(b"Subtype")
            .and_then(Object::as_name)
            .unwrap_or_default()
            .to_vec();
        let base = String::from_utf8_lossy(
            font.get(b"BaseFont")
                .and_then(Object::as_name)
                .unwrap_or_default(),
        )
        .into_owned();
        match subtype.as_slice() {
            b"Type1" | b"MMType1" | b"TrueType" if !has_font_file(pdf, &font) => {
                match replacement(&base) {
                    Some((key, data, known)) => {
                        embed_simple_font(pdf, id, key, data, &mut files)?;
                        replaced.insert(format!("{base} (ersetzt durch {key})"));
                        if !known {
                            approximated.insert(base.clone());
                        }
                    }
                    None => problems.push(format!(
                        "Schrift „{base}“ ist nicht eingebettet und hat keinen Ersatz."
                    )),
                }
            }
            b"CIDFontType0" | b"CIDFontType2" if !has_font_file(pdf, &font) => problems.push(
                format!("CID-Schrift „{base}“ ist nicht eingebettet (kein automatischer Ersatz)."),
            ),
            _ => {}
        }
    }
    if !replaced.is_empty() {
        fixed.push(format!(
            "Schriften eingebettet: {}",
            replaced.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    if !approximated.is_empty() {
        fixed.push(format!(
            "Ersatzschrift mit evtl. abweichenden Breiten: {}",
            approximated.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    Ok(())
}

/// Bettet eine Liberation-Schrift als einfache TrueType-Schrift ein und baut das
/// Schrift-Dictionary um. Breiten kommen aus der Schrift selbst (PDF/A verlangt, dass sie
/// zum Schriftprogramm passen).
fn embed_simple_font(
    pdf: &mut lopdf::Document,
    font_id: ObjectId,
    key: &str,
    data: &[u8],
    files: &mut HashMap<String, ObjectId>,
) -> Result<()> {
    let face = ttf_parser::Face::parse(data, 0)
        .map_err(|e| CoreError::Other(format!("Schrift {key}: {e}")))?;
    let em = f32::from(face.units_per_em());
    let scale = |v: f32| (v * 1000.0 / em).round();

    let file_id = match files.get(key) {
        Some(id) => *id,
        None => {
            let mut s = Stream::new(
                dictionary! { "Length1" => data.len() as i64 },
                data.to_vec(),
            );
            let _ = s.compress();
            let id = pdf.add_object(s);
            files.insert(key.to_string(), id);
            id
        }
    };
    let bbox = face.global_bounding_box();
    let italic = key.contains("Italic");
    let mut flags = 32; // nicht-symbolisch
    if key.contains("Serif") {
        flags |= 2;
    }
    if key.contains("Mono") {
        flags |= 1;
    }
    if italic {
        flags |= 64;
    }
    let descriptor = pdf.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => Object::Name(key.as_bytes().to_vec()),
        "Flags" => flags,
        "FontBBox" => vec![
            scale(f32::from(bbox.x_min)).into(), scale(f32::from(bbox.y_min)).into(),
            scale(f32::from(bbox.x_max)).into(), scale(f32::from(bbox.y_max)).into(),
        ],
        "ItalicAngle" => if italic { -12 } else { 0 },
        "Ascent" => scale(f32::from(face.ascender())),
        "Descent" => scale(f32::from(face.descender())),
        "CapHeight" => scale(f32::from(face.capital_height().unwrap_or(face.ascender()))),
        "StemV" => if key.contains("Bold") { 140 } else { 80 },
        "FontFile2" => file_id,
    });

    // Breiten 32..=255 über WinAnsi -> Unicode -> Glyphe.
    let width = |code: u8| -> f32 {
        crate::font::winansi_char(code)
            .and_then(|c| face.glyph_index(c))
            .and_then(|g| face.glyph_hor_advance(g))
            .map_or(0.0, |a| scale(f32::from(a)))
    };
    let widths: Vec<Object> = (32u8..=255).map(|c| Object::Real(width(c))).collect();

    let font = pdf
        .get_object_mut(font_id)
        .and_then(Object::as_dict_mut)
        .map_err(|e| CoreError::Other(format!("Schrift: {e}")))?;
    font.set("Subtype", "TrueType");
    font.set("BaseFont", Object::Name(key.as_bytes().to_vec()));
    font.set("FirstChar", 32);
    font.set("LastChar", 255);
    font.set("Widths", widths);
    font.set("FontDescriptor", descriptor);
    // Nicht-symbolische TrueType-Schriften brauchen WinAnsi- oder MacRoman-Kodierung.
    let encoding_ok = match font.get(b"Encoding") {
        Ok(Object::Name(n)) => n == b"WinAnsiEncoding" || n == b"MacRomanEncoding",
        _ => false,
    };
    if !encoding_ok {
        font.set("Encoding", "WinAnsiEncoding");
    }
    font.remove(b"ToUnicode"); // passt evtl. nicht mehr; WinAnsi ist eindeutig
    Ok(())
}

/// Beschreibung des Namensraums `pdfuaid` für PDF/A (fremde XMP-Eigenschaften müssen in
/// PDF/A als Erweiterungsschema erklärt werden).
const PDFUA_EXTENSION_SCHEMA: &str = "<pdfaExtension:schemas><rdf:Bag><rdf:li rdf:parseType=\"Resource\">\
<pdfaSchema:schema>PDF/UA Universal Accessibility Schema</pdfaSchema:schema>\
<pdfaSchema:namespaceURI>http://www.aiim.org/pdfua/ns/id/</pdfaSchema:namespaceURI>\
<pdfaSchema:prefix>pdfuaid</pdfaSchema:prefix><pdfaSchema:property><rdf:Seq>\
<rdf:li rdf:parseType=\"Resource\"><pdfaProperty:name>part</pdfaProperty:name>\
<pdfaProperty:valueType>Integer</pdfaProperty:valueType><pdfaProperty:category>internal</pdfaProperty:category>\
<pdfaProperty:description>Indicates, which part of ISO 14289 standard is followed</pdfaProperty:description>\
</rdf:li></rdf:Seq></pdfaSchema:property></rdf:li></rdf:Bag></pdfaExtension:schemas>";

/// Schreibt XMP-Metadaten passend zu /Info und hängt sie an den Katalog.
/// `pdfa` = PDF/A-2b-Kennung eintragen (und /Info auf die gespiegelten Felder beschränken),
/// `pdfua` = PDF/UA-1-Kennung (bei PDF/A samt Erweiterungsschema, das PDF/A für fremde
/// Namensräume verlangt).
pub(crate) fn write_metadata(pdf: &mut lopdf::Document, pdfa: bool, pdfua: bool) -> Result<()> {
    let now = pdf_date_now();
    // /Info lesen (oder anlegen) und auf die Felder beschränken, die wir in XMP spiegeln.
    let info_id = pdf.trailer.get(b"Info").and_then(Object::as_reference).ok();
    let mut info = info_id
        .and_then(|id| pdf.get_dictionary(id).ok().cloned())
        .unwrap_or_default();
    let text = |d: &Dictionary, k: &[u8]| {
        let s = pdf_string::get(d, k);
        (!s.is_empty()).then_some(s)
    };
    let title = text(&info, b"Title");
    let author = text(&info, b"Author");
    let subject = text(&info, b"Subject");
    let keywords = text(&info, b"Keywords");
    let creator = text(&info, b"Creator");
    let created = text(&info, b"CreationDate").unwrap_or_else(|| now.clone());
    let producer = "PDF Tool";
    info.set("Producer", pdf_string::encode(producer));
    info.set("CreationDate", Object::string_literal(created.clone()));
    info.set("ModDate", Object::string_literal(now.clone()));
    // Nur Felder behalten, die auch in XMP stehen.
    let keep: [&[u8]; 8] = [
        b"Title",
        b"Author",
        b"Subject",
        b"Keywords",
        b"Creator",
        b"Producer",
        b"CreationDate",
        b"ModDate",
    ];
    let keys: Vec<Vec<u8>> = info.iter().map(|(k, _)| k.clone()).collect();
    for k in keys {
        if pdfa && !keep.contains(&k.as_slice()) {
            info.remove(&k);
        }
    }
    match info_id {
        Some(id) => {
            pdf.objects.insert(id, Object::Dictionary(info));
        }
        None => {
            let id = pdf.add_object(info);
            pdf.trailer.set("Info", id);
        }
    }

    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut fields = String::new();
    if let Some(t) = &title {
        fields.push_str(&format!(
            "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>",
            esc(t)
        ));
    }
    if let Some(a) = &author {
        fields.push_str(&format!(
            "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>",
            esc(a)
        ));
    }
    if let Some(s) = &subject {
        fields.push_str(&format!(
            "<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>",
            esc(s)
        ));
    }
    if let Some(k) = &keywords {
        fields.push_str(&format!("<pdf:Keywords>{}</pdf:Keywords>", esc(k)));
    }
    if let Some(c) = &creator {
        fields.push_str(&format!("<xmp:CreatorTool>{}</xmp:CreatorTool>", esc(c)));
    }
    let created_xmp = pdf_date_to_xmp(&created).unwrap_or_default();
    let modified_xmp = pdf_date_to_xmp(&now).unwrap_or_default();
    let mut pdfaid = String::new();
    if pdfa {
        pdfaid.push_str("<pdfaid:part>2</pdfaid:part><pdfaid:conformance>B</pdfaid:conformance>");
    }
    if pdfua {
        pdfaid.push_str("<pdfuaid:part>1</pdfuaid:part>");
    }
    if pdfa && pdfua {
        pdfaid.push_str(PDFUA_EXTENSION_SCHEMA);
    }
    let xmp = format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" \
xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" \
xmlns:pdfuaid=\"http://www.aiim.org/pdfua/ns/id/\" xmlns:pdfaExtension=\"http://www.aiim.org/pdfa/ns/extension/\" \
xmlns:pdfaSchema=\"http://www.aiim.org/pdfa/ns/schema#\" xmlns:pdfaProperty=\"http://www.aiim.org/pdfa/ns/property#\">\
{pdfaid}<pdf:Producer>{producer}</pdf:Producer><xmp:CreateDate>{created_xmp}</xmp:CreateDate>\
<xmp:ModifyDate>{modified_xmp}</xmp:ModifyDate><xmp:MetadataDate>{modified_xmp}</xmp:MetadataDate>{fields}\
</rdf:Description></rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>"
    );
    let mut stream = Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        xmp.into_bytes(),
    );
    // PDF/A: Metadaten unkomprimiert (lesbar ohne PDF-Kenntnis).
    stream.allows_compression = false;
    let id = pdf.add_object(stream);
    pdf.catalog_mut()?.set("Metadata", id);
    Ok(())
}

/// Erzeugt fehlende Erscheinungsbilder: für unterstützte Annotationen und ausfüllbare
/// Formularfelder (dieselben Bausteine wie beim Speichern). Liefert die Anzahl.
fn add_missing_appearances(doc: &crate::Document, pdf: &mut lopdf::Document) -> Result<usize> {
    use crate::form::appearance::WidgetAppearance;
    let lacks_ap = |pdf: &lopdf::Document, id: ObjectId| {
        pdf.get_dictionary(id)
            .map(|d| d.get(b"AP").is_err())
            .unwrap_or(false)
    };
    let mut count = 0;
    for a in doc.annotations().iter() {
        let Some(src) = a.source else { continue };
        if !lacks_ap(pdf, src.object_id) {
            continue;
        }
        let stream = crate::annotation::appearance::to_stream(
            crate::annotation::appearance::build(a),
            &mut |o| pdf.add_object(o),
        )?;
        let ap = pdf.add_object(stream);
        if let Ok(d) = pdf
            .get_object_mut(src.object_id)
            .and_then(Object::as_dict_mut)
        {
            d.set("AP", dictionary! { "N" => ap });
            count += 1;
        }
    }
    for field in doc.forms().iter() {
        for w in &field.widgets {
            let Some(wid) = w.object_id else { continue };
            if !lacks_ap(pdf, wid) {
                continue;
            }
            let (ap, state) = match crate::form::appearance::build(field, w) {
                Some(WidgetAppearance::Single(stream)) => {
                    (dictionary! { "N" => pdf.add_object(stream) }, None)
                }
                Some(WidgetAppearance::States { on_state, on, off }) => {
                    let mut n = Dictionary::new();
                    n.set(on_state.as_bytes(), pdf.add_object(on));
                    n.set("Off", pdf.add_object(off));
                    let state = if field.widget_is_on(w) {
                        on_state
                    } else {
                        "Off".into()
                    };
                    (dictionary! { "N" => n }, Some(state))
                }
                None => continue,
            };
            if let Ok(d) = pdf.get_object_mut(wid).and_then(Object::as_dict_mut) {
                d.set("AP", ap);
                if let Some(state) = state {
                    d.set("AS", Object::Name(state.into_bytes()));
                }
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Feldtyp eines Widgets (auch vom Elternfeld geerbt).
fn field_type(pdf: &lopdf::Document, widget: &Dictionary) -> Option<Vec<u8>> {
    let mut d = widget.clone();
    for _ in 0..16 {
        if let Ok(ft) = d.get(b"FT").and_then(Object::as_name) {
            return Some(ft.to_vec());
        }
        let parent = d.get(b"Parent").and_then(Object::as_reference).ok()?;
        d = pdf.get_dictionary(parent).ok()?.clone();
    }
    None
}

/// Nachbesserungen an Erscheinungsbildern, die PDF/A verlangt:
/// - Schaltflächen (/Btn) brauchen für /N ein Zustands-Dictionary statt eines Streams,
/// - jede Annotation (außer Popup/Link und solchen mit Fläche 0) braucht ein /AP –
///   wo keins erzeugt werden konnte (z. B. leeres Signaturfeld), ein leeres unsichtbares.
///
/// Liefert (umgebaute Schaltflächen, leere Erscheinungsbilder).
fn fix_appearance_dicts(pdf: &mut lopdf::Document) -> (usize, usize) {
    let ids: Vec<ObjectId> = pdf.objects.keys().copied().collect();
    let (mut buttons, mut empty) = (0, 0);
    for id in ids {
        let Ok(d) = pdf.get_dictionary(id).cloned() else {
            continue;
        };
        let subtype = d.get(b"Subtype").and_then(Object::as_name).ok();
        let is_annot = d.get(b"Type").and_then(Object::as_name).ok() == Some(b"Annot")
            || (subtype.is_some() && d.has(b"Rect") && d.has(b"P"));
        if !is_annot || matches!(subtype, Some(b"Popup" | b"Link")) {
            continue;
        }
        let rect: Vec<f32> = d
            .get(b"Rect")
            .and_then(Object::as_array)
            .map(|a| a.iter().filter_map(|o| o.as_float().ok()).collect())
            .unwrap_or_default();
        let (w, h) = match rect.as_slice() {
            [x0, y0, x1, y1] => ((x1 - x0).abs(), (y1 - y0).abs()),
            _ => (0.0, 0.0),
        };
        let is_button =
            subtype == Some(b"Widget") && field_type(pdf, &d).as_deref() == Some(b"Btn");
        let normal = d
            .get(b"AP")
            .and_then(Object::as_dict)
            .ok()
            .and_then(|ap| ap.get(b"N").ok().cloned());
        match normal {
            // Einzelner Stream bei einer Schaltfläche -> als Zustand "Default" einpacken.
            Some(n @ Object::Reference(_)) if is_button => {
                let is_stream = n
                    .as_reference()
                    .ok()
                    .and_then(|r| pdf.get_object(r).ok())
                    .is_some_and(|o| o.as_stream().is_ok());
                if is_stream && let Ok(d) = pdf.get_object_mut(id).and_then(Object::as_dict_mut) {
                    d.set("AP", dictionary! { "N" => dictionary! { "Default" => n } });
                    d.set("AS", "Default");
                    buttons += 1;
                }
            }
            Some(_) => {}
            None if w > 0.0 && h > 0.0 => {
                let stream = pdf.add_object(Stream::new(
                    dictionary! {
                        "Type" => "XObject", "Subtype" => "Form",
                        "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
                    },
                    Vec::new(),
                ));
                let n = if is_button {
                    Object::Dictionary(dictionary! { "Off" => stream })
                } else {
                    Object::Reference(stream)
                };
                if let Ok(d) = pdf.get_object_mut(id).and_then(Object::as_dict_mut) {
                    d.set("AP", dictionary! { "N" => n });
                    if is_button && !d.has(b"AS") {
                        d.set("AS", "Off");
                    }
                    empty += 1;
                }
            }
            None => {}
        }
    }
    (buttons, empty)
}

/// Macht direkte Schrift-Dictionaries in /Font-Ressourcen zu eigenen Objekten, damit der
/// Schrift-Schritt sie findet. Sucht rekursiv (Ressourcen von Erscheinungsbildern, /DR des
/// Formulars …) – Erscheinungsbilder enthalten Helvetica oft direkt.
pub(crate) fn promote_direct_fonts(pdf: &mut lopdf::Document) {
    /// Läuft durch ein Objekt; neue Schriftobjekte landen in `out` (Nummern ab `next`).
    fn walk(obj: &mut Object, next: &mut u32, out: &mut Vec<(ObjectId, Object)>, depth: usize) {
        if depth > 32 {
            return;
        }
        let dict = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            Object::Array(a) => {
                for o in a {
                    walk(o, next, out, depth + 1);
                }
                return;
            }
            _ => return,
        };
        for (key, value) in dict.iter_mut() {
            if key.as_slice() == b"Font"
                && let Object::Dictionary(fonts) = value
            {
                for (_, font) in fonts.iter_mut() {
                    let is_font = font.as_dict().is_ok_and(|d| {
                        d.get(b"Type").and_then(Object::as_name).ok() == Some(b"Font")
                    });
                    if is_font {
                        *next += 1;
                        let id = (*next, 0);
                        let direct = std::mem::replace(font, Object::Reference(id));
                        out.push((id, direct));
                    }
                }
            } else {
                walk(value, next, out, depth + 1);
            }
        }
    }
    let mut next = pdf.max_id;
    let mut created = Vec::new();
    for obj in pdf.objects.values_mut() {
        walk(obj, &mut next, &mut created, 0);
    }
    for (id, obj) in created {
        pdf.objects.insert(id, obj);
    }
    pdf.max_id = next;
}

/// Prüfsumme für die Datei-ID (FNV-1a, zweimal mit verschiedenem Startwert = 16 Byte).
fn file_id(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    for seed in [0xcbf2_9ce4_8422_2325u64, 0x8422_2325_cbf2_9ce4] {
        let mut h = seed;
        for &b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        out.extend_from_slice(&h.to_be_bytes());
    }
    out
}

/// Wandelt eine PDF nach bestem Vermögen in PDF/A-2b um.
pub fn convert_to_pdfa(bytes: &[u8]) -> Result<(Vec<u8>, PdfaReport)> {
    let mut report = PdfaReport::default();
    // 1) JavaScript und eingebettete Dateien entfernen (sonst nichts) – wie "Bereinigen".
    let opts = SanitizeOptions {
        metadata: false,
        javascript: true,
        embedded_files: true,
        hidden_layers: false,
        comments: false,
        form_data: false,
    };
    let (clean, removed) = sanitize_pdf(bytes, opts)?;
    report.fixed.extend(removed);
    let check = lopdf::Document::load_mem(&clean)?;
    if check.is_encrypted() {
        return Err(CoreError::Other(
            "Verschlüsselte PDFs können nicht nach PDF/A umgewandelt werden.".into(),
        ));
    }
    // Über das Dokumentmodell laden: So kennen wir Annotationen und Formularfelder.
    let doc = crate::Document::from_bytes(clean)?;
    let mut pdf = doc.pdf().clone();
    pdf.version = "1.7".into();
    let made = add_missing_appearances(&doc, &mut pdf)?;
    if made > 0 {
        report
            .fixed
            .push(format!("{made} fehlende(s) Erscheinungsbild(er) erzeugt"));
    }
    // Geerbte Seitenattribute ausdrücklich in die Seite (PDF/A: Inhalte brauchen ein
    // direkt zugeordnetes /Resources).
    let pages: Vec<ObjectId> = pdf.get_pages().into_values().collect();
    for page in pages {
        for key in [&b"Resources"[..], b"MediaBox", b"CropBox", b"Rotate"] {
            let has = pdf.get_dictionary(page).is_ok_and(|d| d.has(key));
            if !has
                && let Some(v) = crate::document::inherited(&pdf, page, key).cloned()
                && let Ok(d) = pdf.get_object_mut(page).and_then(Object::as_dict_mut)
            {
                d.set(key, v);
            }
        }
    }
    let (buttons, empty) = fix_appearance_dicts(&mut pdf);
    if buttons > 0 {
        report.fixed.push(format!(
            "{buttons} Schaltfläche(n): Erscheinungsbild als Zustand angelegt"
        ));
    }
    if empty > 0 {
        report.fixed.push(format!(
            "{empty} leere(s) Erscheinungsbild(er) für Felder ohne Darstellung (z. B. leere Signaturfelder)"
        ));
    }
    promote_direct_fonts(&mut pdf);

    // 2) Schriften.
    embed_standard_fonts(&mut pdf, &mut report.fixed, &mut report.problems)?;

    // 3) Objekte einzeln anpassen.
    let mut lzw = 0;
    let mut annots = 0;
    let mut missing_ap = 0;
    let mut cmyk = false;
    let ids: Vec<ObjectId> = pdf.objects.keys().copied().collect();
    for id in ids {
        let Some(obj) = pdf.objects.get_mut(&id) else {
            continue;
        };
        // LZW ist in PDF/A verboten: entpacken (wird unten mit Flate neu komprimiert).
        if let Object::Stream(s) = obj {
            let has_lzw = match s.dict.get(b"Filter") {
                Ok(Object::Name(n)) => n == b"LZWDecode",
                Ok(Object::Array(a)) => a.iter().any(|f| f.as_name().ok() == Some(b"LZWDecode")),
                _ => false,
            };
            if has_lzw && s.decompress().is_ok() {
                lzw += 1;
            }
        }
        let dict = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &mut s.dict,
            _ => continue,
        };
        let name = |k: &[u8]| {
            dict.get(k)
                .and_then(Object::as_name)
                .ok()
                .map(<[u8]>::to_vec)
        };
        let ty = name(b"Type");
        let subtype = name(b"Subtype");
        // Bilder: keine Interpolation, keine Alternativen; CMYK merken.
        if subtype.as_deref() == Some(b"Image") {
            dict.remove(b"Interpolate");
            dict.remove(b"Alternates");
            dict.remove(b"OPI");
            if dict.get(b"ColorSpace").and_then(Object::as_name).ok() == Some(b"DeviceCMYK") {
                cmyk = true;
            }
        }
        // Transferfunktionen sind verboten.
        if ty.as_deref() == Some(b"ExtGState") {
            dict.remove(b"TR");
            if dict.get(b"TR2").and_then(Object::as_name).ok() != Some(b"Default") {
                dict.remove(b"TR2");
            }
        }
        // Annotationen: drucken, nicht verstecken; Zusatzaktionen weg.
        if ty.as_deref() == Some(b"Annot")
            || (subtype.is_some() && dict.has(b"Rect") && dict.has(b"P"))
        {
            let f = dict.get(b"F").and_then(Object::as_i64).unwrap_or(0);
            dict.set("F", (f | 4) & !(1 | 2 | 32));
            dict.remove(b"AA");
            annots += 1;
            let needs_ap = !matches!(subtype.as_deref(), Some(b"Popup" | b"Link"));
            if needs_ap && !dict.has(b"AP") {
                missing_ap += 1;
            }
        }
        dict.remove(b"AA");
    }
    if lzw > 0 {
        report
            .fixed
            .push(format!("{lzw} LZW-komprimierte Stream(s) neu komprimiert"));
    }
    if annots > 0 {
        report
            .fixed
            .push(format!("{annots} Annotation(en) druckbar gemacht"));
    }
    if missing_ap > 0 {
        report.problems.push(format!(
            "{missing_ap} Annotation(en) ohne Erscheinungsbild (/AP)."
        ));
    }
    if cmyk {
        report.problems.push(
            "Bilder in DeviceCMYK – mit dem sRGB-Profil nicht zulässig (CMYK-Profil nötig).".into(),
        );
    }

    // 4) Katalog: Formular, Ebenen, OutputIntent, Metadaten.
    let catalog = pdf.catalog()?.clone();
    if let Some(acro_id) = catalog.get(b"AcroForm").and_then(Object::as_reference).ok()
        && let Ok(acro) = pdf.get_object_mut(acro_id).and_then(Object::as_dict_mut)
    {
        acro.remove(b"NeedAppearances");
        acro.remove(b"XFA");
    } else if let Ok(Object::Dictionary(_)) = catalog.get(b"AcroForm")
        && let Ok(Object::Dictionary(acro)) = pdf.catalog_mut()?.get_mut(b"AcroForm")
    {
        acro.remove(b"NeedAppearances");
        acro.remove(b"XFA");
    }
    let has_pdfa_intent = catalog
        .get(b"OutputIntents")
        .ok()
        .and_then(|o| pdf.dereference(o).ok())
        .and_then(|(_, o)| o.as_array().ok())
        .is_some_and(|a| {
            a.iter().any(|i| {
                pdf.dereference(i)
                    .ok()
                    .and_then(|(_, o)| o.as_dict().ok())
                    .and_then(|d| d.get(b"S").and_then(Object::as_name).ok())
                    == Some(b"GTS_PDFA1")
            })
        });
    if !has_pdfa_intent {
        let mut icc = Stream::new(dictionary! { "N" => 3 }, SRGB_ICC.to_vec());
        let _ = icc.compress();
        let icc = pdf.add_object(icc);
        let intent = pdf.add_object(dictionary! {
            "Type" => "OutputIntent",
            "S" => "GTS_PDFA1",
            "OutputConditionIdentifier" => Object::string_literal("sRGB IEC61966-2.1"),
            "Info" => Object::string_literal("sRGB IEC61966-2.1"),
            "DestOutputProfile" => icc,
        });
        pdf.catalog_mut()?
            .set("OutputIntents", vec![Object::from(intent)]);
        report
            .fixed
            .push("sRGB-Farbprofil als OutputIntent ergänzt".into());
    }
    // Ebenen: Standardkonfiguration braucht einen Namen, /AS ist nicht erlaubt.
    if let Ok(Object::Dictionary(oc)) = pdf.catalog_mut()?.get_mut(b"OCProperties")
        && let Ok(Object::Dictionary(d)) = oc.get_mut(b"D")
    {
        d.remove(b"AS");
        if !d.has(b"Name") {
            d.set("Name", Object::string_literal("Standard"));
        }
    }
    pdf.catalog_mut()?.remove(b"AA");
    write_metadata(&mut pdf, true, false)?;
    report
        .fixed
        .push("PDF/A-Metadaten (XMP, PDF/A-2b) geschrieben".into());

    // 5) Neu schreiben: alte Fassungen weg, komprimieren, Datei-ID.
    pdf.trailer.remove(b"Prev");
    pdf.trailer.remove(b"XRefStm");
    pdf.trailer.remove(b"Encrypt");
    pdf.prune_objects();
    pdf.compress();
    let id = Object::String(file_id(bytes), StringFormat::Hexadecimal);
    pdf.trailer.set("ID", vec![id.clone(), id]);
    let mut out = Vec::new();
    pdf.save_to(&mut out)
        .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_replacements() {
        assert_eq!(
            replacement("Helvetica").unwrap().0,
            "LiberationSans-Regular"
        );
        assert_eq!(
            replacement("ABCDEF+Arial-BoldMT").unwrap().0,
            "LiberationSans-Bold"
        );
        assert_eq!(
            replacement("Times-BoldItalic").unwrap().0,
            "LiberationSerif-BoldItalic"
        );
        assert_eq!(
            replacement("Courier-Oblique").unwrap().0,
            "LiberationMono-Italic"
        );
        assert!(
            replacement("Calibri").is_some_and(|r| !r.2),
            "unbekannt -> genähert"
        );
        assert!(replacement("Symbol").is_none());
        assert!(replacement("ZapfDingbats").is_none());
    }
}
