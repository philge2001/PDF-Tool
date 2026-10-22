//! PDF-Seite des Signierens (ohne Kryptografie): eine neue Revision mit Signaturfeld,
//! Signatur-Dictionary und Platzhaltern.
//!
//! Ablauf beim Signieren:
//! 1. [`prepare_signature`] hängt eine Revision an. /Contents ist ein Platzhalter aus Nullen,
//!    /ByteRange wird so eingetragen, dass sie genau die Lücke um /Contents ausspart.
//! 2. Der Aufrufer (Crate `sign`) bildet den Hash über [`PreparedSignature::signed_parts`]
//!    und erzeugt daraus die CMS-Signatur.
//! 3. [`PreparedSignature::finish`] schreibt die Signatur (Hex) in die Lücke. Die Länge der
//!    Datei ändert sich dabei nicht – deshalb stimmen ByteRange und Querverweistabelle weiter.

use crate::annotation::appearance::{helvetica_dict, n, pdf_literal};
use crate::document::Document;
use crate::error::{CoreError, Result};
use crate::form::FieldKind;
use crate::form_save::{dict_mut, resolved_array, update_acroform};
use crate::geometry::Rect;
use crate::{font, pdf_string};
use lopdf::{Dictionary, IncrementalDocument, Object, ObjectId, Stream, StringFormat, dictionary};
use std::collections::HashSet;
use std::fmt::Write as _;

/// Platzhalterzahl in /ByteRange (zehnstellig, damit später jeder echte Wert hineinpasst).
const RANGE_PLACEHOLDER: i64 = 9_999_999_999;

/// Wo die Signatur hinkommt.
#[derive(Debug, Clone, PartialEq)]
pub enum SigPlacement {
    /// Vorhandenes, noch leeres Signaturfeld (voller Name).
    Existing(String),
    /// Neues Feld auf Seite `page` (Index). Ein leeres Rechteck ergibt eine unsichtbare Signatur.
    New {
        page: usize,
        rect: Rect,
        name: String,
    },
}

/// Alles, was ins Signatur-Dictionary und in die sichtbare Darstellung kommt.
#[derive(Debug, Clone)]
pub struct SignatureSpec {
    pub placement: SigPlacement,
    /// z. B. "ETSI.CAdES.detached" (PAdES).
    pub sub_filter: String,
    /// Name des Unterzeichners (/Name), Grund, Ort, Kontakt (dürfen leer sein).
    pub name: String,
    pub reason: String,
    pub location: String,
    pub contact: String,
    /// PDF-Datum der Signatur (/M).
    pub date: String,
    /// Textzeilen für die sichtbare Signatur.
    pub appearance_lines: Vec<String>,
    /// Reservierter Platz für die CMS-Signatur in Bytes.
    pub contents_len: usize,
}

/// Datei mit Platzhalter, bereit zum Signieren.
#[derive(Debug, Clone)]
pub struct PreparedSignature {
    bytes: Vec<u8>,
    /// Position von '<' und die Position direkt hinter '>' des /Contents-Platzhalters.
    gap: (usize, usize),
}

impl PreparedSignature {
    /// /ByteRange [a b c d]: signiert sind a..a+b und c..c+d.
    pub fn byte_range(&self) -> [usize; 4] {
        let (start, end) = self.gap;
        [0, start, end, self.bytes.len() - end]
    }

    /// Die beiden signierten Teile der Datei (alles außer der Lücke).
    pub fn signed_parts(&self) -> [&[u8]; 2] {
        let (start, end) = self.gap;
        [&self.bytes[..start], &self.bytes[end..]]
    }

    /// Setzt die fertige Signatur (DER) in die Lücke ein.
    pub fn finish(mut self, der: &[u8]) -> Result<Vec<u8>> {
        let (start, end) = self.gap;
        let room = end - start - 2; // ohne '<' und '>'
        if der.len() * 2 > room {
            return Err(CoreError::Other(format!(
                "Signatur zu groß ({} Bytes, Platz für {})",
                der.len(),
                room / 2
            )));
        }
        let mut hex = String::with_capacity(room);
        for b in der {
            let _ = write!(hex, "{b:02x}");
        }
        self.bytes[start + 1..start + 1 + hex.len()].copy_from_slice(hex.as_bytes());
        Ok(self.bytes)
    }
}

/// Hängt an `base` (fertige PDF-Bytes) eine Revision mit Signaturfeld und Platzhalter an.
pub fn prepare_signature(base: &[u8], spec: &SignatureSpec) -> Result<PreparedSignature> {
    let doc = Document::from_bytes(base.to_vec())?;
    let pdf = doc.pdf();
    let mut inc = IncrementalDocument::create_from(base.to_vec(), pdf.clone());

    // Signatur-Dictionary mit Platzhaltern.
    let mut sig = dictionary! {
        "Type" => "Sig",
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => Object::Name(spec.sub_filter.as_bytes().to_vec()),
        "ByteRange" => vec![Object::from(0), RANGE_PLACEHOLDER.into(), RANGE_PLACEHOLDER.into(), RANGE_PLACEHOLDER.into()],
        "Contents" => Object::String(vec![0; spec.contents_len], StringFormat::Hexadecimal),
        "M" => Object::string_literal(spec.date.clone()),
    };
    for (key, value) in [
        ("Name", &spec.name),
        ("Reason", &spec.reason),
        ("Location", &spec.location),
        ("ContactInfo", &spec.contact),
    ] {
        if !value.is_empty() {
            sig.set(key, pdf_string::encode(value));
        }
    }
    let sig_id = inc.new_document.add_object(sig);

    match &spec.placement {
        SigPlacement::Existing(name) => {
            let field = doc
                .forms()
                .by_name(name)
                .filter(|f| f.kind == FieldKind::Signature)
                .ok_or_else(|| CoreError::Other(format!("Signaturfeld „{name}“ fehlt")))?;
            let fid = field
                .source
                .ok_or_else(|| CoreError::Other("Signaturfeld ohne Objekt".into()))?;
            if pdf.get_dictionary(fid).is_ok_and(|d| d.has(b"V")) {
                return Err(CoreError::Other(format!(
                    "Feld „{name}“ ist bereits signiert"
                )));
            }
            inc.opt_clone_object_to_new_document(fid)?;
            dict_mut(&mut inc, fid)?.set("V", sig_id);
            if let Some(w) = field.widgets.first()
                && let Some(wid) = w.object_id
            {
                let ap = appearance(&mut inc, w.rect, &spec.appearance_lines);
                inc.opt_clone_object_to_new_document(wid)?;
                let wd = dict_mut(&mut inc, wid)?;
                if let Some(ap) = ap {
                    wd.set("AP", dictionary! { "N" => ap });
                }
            }
            update_acroform(pdf, &mut inc, &[], &HashSet::new(), Some(3))?;
        }
        SigPlacement::New { page, rect, name } => {
            let page_id = doc.page(*page)?.object_id;
            let rect = rect.normalized();
            let ap = appearance(&mut inc, rect, &spec.appearance_lines);
            // Feld und Widget in einem Objekt. /F 132 = Drucken + Gesperrt.
            let mut w = dictionary! {
                "Type" => "Annot",
                "Subtype" => "Widget",
                "FT" => "Sig",
                "T" => pdf_string::encode(name),
                "V" => sig_id,
                "Rect" => vec![rect.x0.into(), rect.y0.into(), rect.x1.into(), rect.y1.into()],
                "P" => page_id,
                "F" => 132,
            };
            if let Some(ap) = ap {
                w.set("AP", dictionary! { "N" => ap });
            }
            let wid = inc.new_document.add_object(w);
            let mut annots = resolved_array(&inc, page_id, b"Annots");
            annots.push(wid.into());
            inc.opt_clone_object_to_new_document(page_id)?;
            dict_mut(&mut inc, page_id)?.set("Annots", annots);
            update_acroform(pdf, &mut inc, &[wid], &HashSet::new(), Some(3))?;
        }
    }

    let mut bytes = Vec::new();
    inc.save_to(&mut bytes)
        .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
    let gap = find_gap(&bytes, spec.contents_len)?;
    patch_byte_range(&mut bytes, gap)?;
    Ok(PreparedSignature { bytes, gap })
}

/// Findet den /Contents-Platzhalter (letztes Vorkommen = in der neuen Revision).
fn find_gap(bytes: &[u8], len: usize) -> Result<(usize, usize)> {
    let mut needle = Vec::with_capacity(len * 2 + 2);
    needle.push(b'<');
    needle.resize(len * 2 + 1, b'0');
    needle.push(b'>');
    let start = rfind(bytes, &needle)
        .ok_or_else(|| CoreError::Other("Signatur-Platzhalter nicht gefunden".into()))?;
    Ok((start, start + needle.len()))
}

/// Ersetzt den /ByteRange-Platzhalter durch die echten Werte (mit Leerzeichen aufgefüllt,
/// damit sich keine Position in der Datei verschiebt).
fn patch_byte_range(bytes: &mut [u8], (start, end): (usize, usize)) -> Result<()> {
    let ph = RANGE_PLACEHOLDER.to_string();
    let needle = format!("{ph} {ph} {ph}");
    let at = rfind(bytes, needle.as_bytes())
        .ok_or_else(|| CoreError::Other("ByteRange-Platzhalter nicht gefunden".into()))?;
    let open = bytes[..at]
        .iter()
        .rposition(|&b| b == b'[')
        .ok_or_else(|| CoreError::Other("ByteRange-Platzhalter beschädigt".into()))?;
    let close = at + needle.len();
    if bytes.get(close) != Some(&b']') {
        return Err(CoreError::Other("ByteRange-Platzhalter beschädigt".into()));
    }
    let text = format!("[0 {start} {end} {}", bytes.len() - end);
    let slot = &mut bytes[open..close];
    if text.len() > slot.len() {
        return Err(CoreError::Other(
            "ByteRange passt nicht in den Platzhalter".into(),
        ));
    }
    slot.fill(b' ');
    slot[..text.len()].copy_from_slice(text.as_bytes());
    Ok(())
}

fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// Sichtbare Darstellung: Rahmen und Textzeilen, Schrift so groß, dass alles passt.
/// `None` bei unsichtbarer Signatur (leeres Rechteck).
fn appearance(inc: &mut IncrementalDocument, rect: Rect, lines: &[String]) -> Option<ObjectId> {
    let (w, h) = (rect.width(), rect.height());
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let pad = 4.0;
    let widest = |size: f32| {
        lines
            .iter()
            .map(|l| font::text_width(l, size))
            .fold(0.0_f32, f32::max)
    };
    // Größte Schrift (max. 12 pt), bei der Breite und Höhe reichen.
    let mut size = 12.0_f32;
    while size > 4.0
        && (widest(size) > w - 2.0 * pad || lines.len() as f32 * size * 1.2 > h - 2.0 * pad)
    {
        size -= 0.5;
    }
    let mut s = String::new();
    let _ = writeln!(
        s,
        "q 0.2 0.3 0.6 RG 1 w 0.5 0.5 {} {} re S",
        n(w - 1.0),
        n(h - 1.0)
    );
    let _ = writeln!(s, "BT /Helv {} Tf 0.1 0.15 0.35 rg", n(size));
    let lh = size * 1.2;
    // Textblock vertikal mittig.
    let block = lines.len() as f32 * lh;
    let first = (h + block) / 2.0 - size;
    let _ = writeln!(s, "{} {} Td", n(pad), n(first));
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            let _ = writeln!(s, "0 {} Td", n(-lh));
        }
        let _ = writeln!(s, "{} Tj", pdf_literal(&font::winansi_encode(l)));
    }
    s.push_str("ET Q\n");
    let dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "Helv" => helvetica_dict() } },
    };
    Some(
        inc.new_document
            .add_object(Stream::new(dict, s.into_bytes())),
    )
}

/// Ein freier Name für ein neues Signaturfeld ("Signatur1", "Signatur2" …).
pub fn free_signature_name(doc: &Document) -> String {
    doc.forms().unique_name("Signatur")
}

/// Vorhandene, noch nicht signierte Signaturfelder (volle Namen).
pub fn empty_signature_fields(doc: &Document) -> Vec<String> {
    let pdf = doc.pdf();
    doc.forms()
        .iter()
        .filter(|f| f.kind == FieldKind::Signature)
        .filter(|f| {
            f.source
                .and_then(|id| pdf.get_dictionary(id).ok())
                .is_some_and(|d: &Dictionary| !d.has(b"V"))
        })
        .map(|f| f.name.clone())
        .collect()
}
