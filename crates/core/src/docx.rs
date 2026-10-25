//! Einfacher Word-Export (DOCX): Absätze und Bilder in Lesereihenfolge (Phase 8).
//!
//! DOCX ist ein ZIP mit XML-Dateien (WordprocessingML). Wir schreiben das Nötigste:
//! - `[Content_Types].xml`, `_rels/.rels` – Pflichtteile des Containers,
//! - `word/document.xml` – Absätze (Schriftgröße, Fett), Bilder, Seitenumbrüche,
//! - `word/_rels/document.xml.rels` + `word/media/…` – die Bilder.
//!
//! Die Absätze kommen aus `core::paragraph` (Zeilen zusammengefasst, Trennstriche entfernt),
//! die Bilder aus `core::image_edit`. Pro Seite wird nach der Höhe (oben zuerst) sortiert.
//! *Grenze:* kein originalgetreues Layout (Spalten, Positionen, Tabellen).

use crate::content::{self, page_resources};
use crate::document::Document;
use crate::error::{CoreError, Result};
use crate::image_codec::{self, PixelFormat};
use crate::raster::Raster;
use std::fmt::Write as _;
use std::io::Write as _;

/// 1 pt = 12 700 EMU (Maßeinheit von DrawingML).
const EMU_PER_PT: f32 = 12_700.0;

/// Ein Element einer Seite in Lesereihenfolge.
enum Block {
    Text {
        text: String,
        size: f32,
        bold: bool,
    },
    Image {
        media: usize,
        width: f32,
        height: f32,
    },
}

/// Ein Bild für `word/media/`.
struct Media {
    data: Vec<u8>,
    ext: &'static str,
}

/// XML-Sonderzeichen ersetzen (und Steuerzeichen weglassen, die XML nicht erlaubt).
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// Bild-XObject für Word aufbereiten: JPEG unverändert, sonst (Grau/RGB, 8 Bit) als PNG.
/// Andere Formate (CMYK, Indexed, 1 Bit …) werden übersprungen.
fn media_for(pdf: &lopdf::Document, stream: &lopdf::Stream) -> Option<Media> {
    let filter = stream.dict.get(b"Filter").ok().and_then(|f| match f {
        lopdf::Object::Name(n) => Some(n.clone()),
        lopdf::Object::Array(a) if a.len() == 1 => a[0].as_name().ok().map(<[u8]>::to_vec),
        _ => None,
    });
    let space = stream
        .dict
        .get(b"ColorSpace")
        .ok()
        .and_then(|o| o.as_name().ok())
        .unwrap_or_default();
    let simple_space = space == b"DeviceRGB" || space == b"DeviceGray";
    if filter.as_deref() == Some(b"DCTDecode") && simple_space {
        return Some(Media {
            data: stream.content.clone(),
            ext: "jpeg",
        });
    }
    if !simple_space {
        return None;
    }
    let r = Raster::decode(pdf, stream).ok()?;
    let format = match (r.comps, r.bpc) {
        (1, 8) => PixelFormat::Gray,
        (3, 8) => PixelFormat::Rgb,
        _ => return None,
    };
    let data = image_codec::encode_png(&r.data, r.width as u32, r.height as u32, format).ok()?;
    Some(Media { data, ext: "png" })
}

/// Erzeugt die DOCX-Datei für die angegebenen Seiten. `progress(Anteil, Text)` liefert
/// `false`, wenn abgebrochen werden soll.
pub fn export_docx(
    doc: &Document,
    pages: &[usize],
    progress: &dyn Fn(f32, String) -> bool,
) -> Result<Vec<u8>> {
    let pdf = doc.pdf();
    let mut media: Vec<Media> = Vec::new();
    let mut body = String::new();
    let total = pages.len();
    for (i, &page_idx) in pages.iter().enumerate() {
        if !progress(
            i as f32 / total.max(1) as f32,
            format!("Seite {} …", page_idx + 1),
        ) {
            return Err(CoreError::Other("Export abgebrochen.".into()));
        }
        let info = doc.page(page_idx)?.clone();
        let resources = page_resources(pdf, info.object_id);
        // (Oberkante, Block) – sortiert nach Höhe auf der Seite.
        let mut blocks: Vec<(f32, Block)> = Vec::new();
        let paragraphs = doc.paragraphs(page_idx).unwrap_or_default();
        for p in &paragraphs {
            let base = content::load_font(pdf, resources, None, &p.font).base_font;
            let bold = base.contains("Bold") || base.contains("Black") || base.contains("Heavy");
            blocks.push((
                p.bounds.y1,
                Block::Text {
                    text: p.text.clone(),
                    size: p.size_on_page,
                    bold,
                },
            ));
        }
        // Bilder; ein seitenfüllendes Bild (Scan) nur, wenn die Seite keinen Text hat –
        // sonst stünde der erkannte Text doppelt im Dokument.
        let page_area = info.crop_box.width() * info.crop_box.height();
        let xobjects = resources
            .and_then(|r| r.get(b"XObject").ok())
            .and_then(|o| pdf.dereference(o).ok())
            .and_then(|(_, o)| o.as_dict().ok());
        for img in doc.page_images(page_idx).unwrap_or_default() {
            let area = img.bounds.width() * img.bounds.height();
            if area > 0.8 * page_area && !paragraphs.is_empty() {
                continue;
            }
            let Some(stream) = img
                .name
                .as_ref()
                .and_then(|n| xobjects?.get(n).ok())
                .and_then(|o| pdf.dereference(o).ok())
                .and_then(|(_, o)| o.as_stream().ok())
            else {
                continue;
            };
            let Some(m) = media_for(pdf, stream) else {
                continue;
            };
            media.push(m);
            blocks.push((
                img.bounds.y1,
                Block::Image {
                    media: media.len(),
                    width: img.bounds.width(),
                    height: img.bounds.height(),
                },
            ));
        }
        // Oben zuerst (PDF-y wächst nach oben). Gedrehte Seiten: grob, aber lesbar.
        blocks.sort_by(|a, b| b.0.total_cmp(&a.0));

        if i > 0 {
            body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
        }
        for (_, block) in blocks {
            match block {
                Block::Text { text, size, bold } => {
                    // w:sz in halben Punkt.
                    let half_points = (size * 2.0).round().clamp(2.0, 3276.0) as u32;
                    // Harte Zeilenumbrüche im Absatz als Zeilenwechsel in Word.
                    let text = text
                        .split('\n')
                        .map(xml_escape)
                        .collect::<Vec<_>>()
                        .join(r#"</w:t><w:br/><w:t xml:space="preserve">"#);
                    let _ = write!(
                        body,
                        r#"<w:p><w:r><w:rPr>{}<w:sz w:val="{half_points}"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                        if bold { "<w:b/>" } else { "" },
                        text
                    );
                }
                Block::Image {
                    media: n,
                    width,
                    height,
                } => {
                    let (cx, cy) = (
                        (width * EMU_PER_PT).round() as i64,
                        (height * EMU_PER_PT).round() as i64,
                    );
                    let _ = write!(
                        body,
                        r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{n}" name="Bild {n}"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="{n}" name="Bild {n}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImg{n}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
                    );
                }
            }
        }
    }
    if body.is_empty() {
        body.push_str("<w:p/>");
    }
    write_package(&body, &media)
}

/// Packt alles in den ZIP-Container.
fn write_package(body: &str, media: &[Media]) -> Result<Vec<u8>> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let mut doc_rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (i, m) in media.iter().enumerate() {
        let n = i + 1;
        let _ = write!(
            doc_rels,
            r#"<Relationship Id="rIdImg{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/bild{n}.{}"/>"#,
            m.ext
        );
    }
    doc_rels.push_str("</Relationships>");
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="709" w:footer="709" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let deflate = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let zerr = |e: zip::result::ZipError| CoreError::Other(format!("DOCX schreiben: {e}"));
    let ioerr = |e: std::io::Error| CoreError::Other(format!("DOCX schreiben: {e}"));
    let mut add = |name: &str, data: &[u8]| -> Result<()> {
        zip.start_file(name, deflate).map_err(zerr)?;
        zip.write_all(data).map_err(ioerr)
    };
    add("[Content_Types].xml", content_types.as_bytes())?;
    add("_rels/.rels", rels.as_bytes())?;
    add("word/document.xml", document.as_bytes())?;
    add("word/_rels/document.xml.rels", doc_rels.as_bytes())?;
    for (i, m) in media.iter().enumerate() {
        add(&format!("word/media/bild{}.{}", i + 1, m.ext), &m.data)?;
    }
    Ok(zip.finish().map_err(zerr)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml() {
        assert_eq!(xml_escape("a<b & \"c\"\u{1}"), "a&lt;b &amp; &quot;c&quot;");
    }
}
