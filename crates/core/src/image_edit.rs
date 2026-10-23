//! Bilder im Seiteninhalt: auswählen, verschieben/skalieren, ersetzen, löschen, einfügen (Phase 6.3).
//!
//! Ein Bild wird mit `Do` (Bild-XObject) oder als Inline-Bild (`BI … EI`) gezeichnet; es füllt das
//! Einheitsquadrat unter der aktuellen Matrix (CTM). Verschieben/Skalieren ändert deshalb nur die
//! Matrix: Vor die Zeichenoperation kommt `q X cm … Q` mit X = M_neu × M_alt⁻¹. Steht das Bild
//! schon in genau so einer Klammer, wird nur deren `cm` angepasst (keine Verschachtelung).
//!
//! Bilder in Formular-XObjects (viele Programme verpacken Bilder so, oft mit Beschneidung)
//! zählen ebenfalls, wenn das Formular nur Bilder und Pfade enthält: Verschoben, skaliert,
//! gelöscht wird dann der Aufruf des Formulars (`Do`), ersetzt wird er durch das neue Bild.
//!
//! Neue Bilder (PNG/JPEG) liegen bis zum Speichern im Modell (`PageContent::images`).
//! JPEG wird unverändert übernommen (`DCTDecode`), PNG dekodiert und mit Flate komprimiert;
//! ein Alphakanal wird zur `/SMask`.

use crate::content::{Item, Matrix};
use crate::document::Document;
use crate::error::{CoreError, Result};
use crate::geometry::{Point, Rect};
use crate::text_edit::{PageContent, ReplacePageContent};
use lopdf::content::{Content, Operation};
use lopdf::{Object, Stream, dictionary};

/// Ein neues Bild-XObject (noch nicht in der Datei).
#[derive(Clone)]
pub struct NewImage {
    pub image: Stream,
    /// Transparenz (Graustufen-Bild), wird beim Speichern als /SMask verknüpft.
    pub smask: Option<Stream>,
    /// Größe in Pixeln.
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Debug for NewImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NewImage({}×{}, Maske: {})",
            self.width,
            self.height,
            self.smask.is_some()
        )
    }
}

/// Größte erlaubte Pixelzahl (schützt vor riesigen oder präparierten Dateien).
const MAX_PIXELS: u64 = 100_000_000;

impl NewImage {
    /// Liest eine PNG- oder JPEG-Datei.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let img = if bytes.starts_with(b"\x89PNG") {
            Self::from_png(bytes)
        } else if bytes.starts_with(&[0xFF, 0xD8]) {
            Self::from_jpeg(bytes)
        } else {
            Err("Nur PNG- und JPEG-Bilder werden unterstützt.".to_string())
        }
        .map_err(CoreError::Other)?;
        if u64::from(img.width) * u64::from(img.height) > MAX_PIXELS {
            return Err(CoreError::Other("Das Bild ist zu groß.".into()));
        }
        Ok(img)
    }

    fn from_jpeg(bytes: &[u8]) -> std::result::Result<Self, String> {
        use zune_jpeg::JpegDecoder;
        use zune_jpeg::zune_core::bytestream::ZCursor;
        let mut dec = JpegDecoder::new(ZCursor::new(bytes));
        dec.decode_headers()
            .map_err(|e| format!("JPEG unlesbar: {e:?}"))?;
        let info = dec.info().ok_or("JPEG ohne Kopfdaten")?;
        let comps = dec.input_colorspace().map_or(3, |c| c.num_components());
        let space = match comps {
            1 => "DeviceGray",
            4 => "DeviceCMYK",
            _ => "DeviceRGB",
        };
        let (width, height) = (u32::from(info.width), u32::from(info.height));
        let mut dict = image_dict(width, height, space);
        dict.set("Filter", "DCTDecode");
        // CMYK-JPEGs aus Adobe-Programmen sind invertiert gespeichert.
        if comps == 4 && bytes.windows(5).take(4096).any(|w| w == b"Adobe") {
            dict.set(
                "Decode",
                [1, 0, 1, 0, 1, 0, 1, 0]
                    .into_iter()
                    .map(Object::from)
                    .collect::<Vec<_>>(),
            );
        }
        Ok(Self {
            image: Stream::new(dict, bytes.to_vec()),
            smask: None,
            width,
            height,
        })
    }

    fn from_png(bytes: &[u8]) -> std::result::Result<Self, String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        // Paletten/kleine Bittiefen auf 8 Bit erweitern, 16 Bit auf 8 Bit kürzen.
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("PNG unlesbar: {e}"))?;
        let (width, height) = {
            let info = reader.info();
            (info.width, info.height)
        };
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err("Das Bild ist zu groß.".into());
        }
        let size = reader.output_buffer_size().ok_or("PNG zu groß")?;
        let mut buf = vec![0; size];
        let frame = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("PNG unlesbar: {e}"))?;
        let data = &buf[..frame.buffer_size()];
        let (comps, alpha, space) = match frame.color_type {
            png::ColorType::Grayscale => (1, false, "DeviceGray"),
            png::ColorType::GrayscaleAlpha => (1, true, "DeviceGray"),
            png::ColorType::Rgba => (3, true, "DeviceRGB"),
            _ => (3, false, "DeviceRGB"),
        };
        // Farbe und Alpha trennen: PDF speichert Transparenz als eigenes Graustufenbild.
        let (color, mask) = if alpha {
            let px = comps + 1;
            let color: Vec<u8> = data
                .chunks_exact(px)
                .flat_map(|p| p[..comps].iter().copied())
                .collect();
            let mask: Vec<u8> = data.chunks_exact(px).map(|p| p[comps]).collect();
            // Ganz deckend? Dann braucht es keine Maske.
            let opaque = mask.iter().all(|&a| a == 255);
            (color, (!opaque).then_some(mask))
        } else {
            (data.to_vec(), None)
        };
        let mut image = Stream::new(image_dict(width, height, space), color);
        let _ = image.compress();
        let smask = mask.map(|m| {
            let mut s = Stream::new(image_dict(width, height, "DeviceGray"), m);
            let _ = s.compress();
            s
        });
        Ok(Self {
            image,
            smask,
            width,
            height,
        })
    }
}

fn image_dict(width: u32, height: u32, space: &str) -> lopdf::Dictionary {
    dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => i64::from(width),
        "Height" => i64::from(height),
        "ColorSpace" => Object::Name(space.as_bytes().to_vec()),
        "BitsPerComponent" => 8,
    }
}

/// Ein Bild auf einer Seite (direkt im Seiteninhalt oder als Formular-XObject, das nur
/// Bilder enthält).
#[derive(Debug, Clone, PartialEq)]
pub struct PageImage {
    /// Stempel des Inhalts, aus dem es stammt.
    pub stamp: u64,
    /// Index der Zeichenoperation (`Do` oder `BI`).
    pub op: usize,
    /// Abbildung des Einheitsquadrats auf die Seite (bei Formularen: des größten Bilds darin).
    pub matrix: Matrix,
    /// Grafikmatrix (CTM) an der Zeichenoperation – dort setzt das Verschieben an. Bei Bildern
    /// gleich `matrix`, bei Formularen ohne deren /Matrix.
    pub ctm: Matrix,
    /// Bild steckt in einem Formular-XObject.
    pub in_form: bool,
    /// Größe in Pixeln (falls bekannt).
    pub pixels: Option<(u32, u32)>,
    /// Umfassendes Rechteck (PDF-Benutzerraum).
    pub bounds: Rect,
    pub inline: bool,
    /// Ressourcenname des Bild-XObjects (`None` bei Inline-Bildern).
    pub name: Option<Vec<u8>>,
}

fn matrix_operands(m: &Matrix) -> Vec<Object> {
    m.0.iter().map(|v| Object::Real(*v)).collect()
}

/// Setzt die Matrix des Bilds an `op` von `old` auf `new` (siehe Moduldoku).
fn set_matrix(ops: &mut Vec<Operation>, op: usize, old: &Matrix, new: &Matrix) -> Result<()> {
    let inv = old
        .invert()
        .ok_or_else(|| CoreError::Other("Bild mit ungültiger Matrix.".into()))?;
    let x = new.then(&inv);
    let wrapped = op >= 2
        && ops[op - 2].operator == "q"
        && ops[op - 1].operator == "cm"
        && ops.get(op + 1).is_some_and(|o| o.operator == "Q");
    if wrapped && let Some(cm) = Matrix::from_operands(&ops[op - 1].operands) {
        // Die `cm` wirkt nur auf dieses Bild: einfach ersetzen.
        ops[op - 1].operands = matrix_operands(&x.then(&cm));
    } else {
        let draw = ops[op].clone();
        ops.splice(
            op..=op,
            [
                Operation::new("q", vec![]),
                Operation::new("cm", matrix_operands(&x)),
                draw,
                Operation::new("Q", vec![]),
            ],
        );
    }
    Ok(())
}

/// Freier Ressourcenname für ein neues Bild.
fn free_image_name(doc: &Document, pc: &PageContent, page: lopdf::ObjectId) -> Vec<u8> {
    let resources = crate::content::page_resources(doc.pdf(), page);
    let taken = |n: &[u8]| {
        pc.images.contains_key(n)
            || resources
                .and_then(|r| r.get(b"XObject").ok())
                .and_then(|o| doc.pdf().dereference(o).ok())
                .and_then(|(_, o)| o.as_dict().ok())
                .is_some_and(|d| d.has(n))
    };
    (1..)
        .map(|i| format!("FcImg{i}").into_bytes())
        .find(|n| !taken(n))
        .expect("unendlich viele Namen")
}

impl Document {
    /// Bilder einer Seite (in Zeichenreihenfolge), auch Formular-XObjects mit nur Bildern.
    pub fn page_images(&self, page_idx: usize) -> Result<Vec<PageImage>> {
        let pc = self.page_content(page_idx)?;
        let page = self.page(page_idx)?.object_id;
        let it = self.interpret_content(page, &pc)?;
        let pdf = self.pdf();
        let resources = crate::content::page_resources(pdf, page);
        // Pixelgröße eines Bild-XObjects aus Ressourcen.
        let pixels_of = |res: Option<&lopdf::Dictionary>, name: &[u8]| -> Option<(u32, u32)> {
            if let Some(new) = pc.images.get(name) {
                return Some((new.width, new.height));
            }
            let xo = res?.get(b"XObject").ok()?;
            let xo = pdf.dereference(xo).ok()?.1.as_dict().ok()?;
            let s = pdf
                .dereference(xo.get(name).ok()?)
                .ok()?
                .1
                .as_stream()
                .ok()?;
            let w = s.dict.get(b"Width").and_then(Object::as_i64).ok()?;
            let h = s.dict.get(b"Height").and_then(Object::as_i64).ok()?;
            Some((w.max(1) as u32, h.max(1) as u32))
        };
        let mut out = Vec::new();
        for item in &it.items {
            match item {
                Item::Image {
                    op,
                    name,
                    matrix,
                    bounds,
                } => out.push(PageImage {
                    stamp: pc.stamp,
                    op: *op,
                    matrix: *matrix,
                    ctm: *matrix,
                    in_form: false,
                    pixels: name.as_ref().and_then(|n| pixels_of(resources, n)),
                    bounds: *bounds,
                    inline: name.is_none(),
                    name: name.clone(),
                }),
                Item::Form {
                    op,
                    name,
                    id: Some(id),
                    matrix,
                    ..
                } => {
                    let Ok(stream) = pdf.get_object(*id).and_then(Object::as_stream) else {
                        continue;
                    };
                    let Ok(inner) = crate::content::interpret_form(pdf, stream, resources, *matrix)
                    else {
                        continue;
                    };
                    let has_text = inner.items.iter().any(|i| {
                        matches!(i, Item::Text(t) if t.glyphs.iter().any(|g| !g.text.trim().is_empty()))
                    });
                    let largest = inner
                        .items
                        .iter()
                        .filter_map(|i| match i {
                            Item::Image {
                                matrix,
                                bounds,
                                name,
                                ..
                            } => Some((matrix, bounds, name)),
                            _ => None,
                        })
                        .max_by(|a, b| {
                            let area = |r: &Rect| r.width() * r.height();
                            area(a.1).total_cmp(&area(b.1))
                        });
                    let (Some((m, b, img_name)), false) = (largest, has_text) else {
                        continue;
                    };
                    // CTM am `Do` = /Matrix⁻¹ × (CTM inkl. /Matrix).
                    let form_matrix = stream
                        .dict
                        .get(b"Matrix")
                        .ok()
                        .and_then(|o| o.as_array().ok())
                        .and_then(|a| Matrix::from_operands(a))
                        .unwrap_or(Matrix::IDENTITY);
                    let Some(ctm) = form_matrix.invert().map(|inv| inv.then(matrix)) else {
                        continue;
                    };
                    let form_res = stream
                        .dict
                        .get(b"Resources")
                        .ok()
                        .and_then(|o| pdf.dereference(o).ok())
                        .and_then(|(_, o)| o.as_dict().ok());
                    out.push(PageImage {
                        stamp: pc.stamp,
                        op: *op,
                        matrix: *m,
                        ctm,
                        in_form: true,
                        pixels: img_name.as_ref().and_then(|n| pixels_of(form_res, n)),
                        bounds: *b,
                        inline: false,
                        name: Some(name.clone()),
                    });
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Eine kleine PDF mit nur diesem Bild (eine Seite, Bildgröße in Originalauflösung bei
    /// 1 pt = 1 Pixel, höchstens 25 Megapixel). Zum Rendern mit PDFium – so stimmen
    /// Farbraum, Masken und Beschneidung wie in der Anzeige. Liefert Bytes und Seitengröße.
    pub fn image_snapshot(
        &self,
        page_idx: usize,
        img: &PageImage,
    ) -> Result<(Vec<u8>, (f32, f32))> {
        let info = self.page(page_idx)?.clone();
        let pc = self.page_content(page_idx)?;
        let ops = Content::decode(&pc.content)?.operations;
        let draw = ops
            .get(img.op)
            .filter(|o| o.operator == "Do" || o.operator == "BI")
            .cloned()
            .ok_or_else(|| CoreError::Other("Bild passt nicht zum Seiteninhalt.".into()))?;
        let mut pdf = self.snapshot_pdf()?;
        let resources = crate::content::page_resources(&pdf, info.object_id)
            .cloned()
            .unwrap_or_default();

        // Maßstab: Pixelbreite des Bilds pro Punkt der Darstellung (sonst 300 dpi).
        let (bw, bh) = (img.bounds.width().max(1.0), img.bounds.height().max(1.0));
        let mut k = match img.pixels {
            Some((pw, ph)) => {
                let m = img.matrix.0;
                let (sw, sh) = (m[0].hypot(m[1]).max(1.0), m[2].hypot(m[3]).max(1.0));
                (pw as f32 / sw).max(ph as f32 / sh)
            }
            None => 300.0 / 72.0,
        }
        .clamp(0.25, 8.0);
        let max_px = 25_000_000.0;
        if bw * bh * k * k > max_px {
            k = (max_px / (bw * bh)).sqrt();
        }
        let (w, h) = (bw * k, bh * k);
        let place = Matrix([k, 0.0, 0.0, k, -img.bounds.x0 * k, -img.bounds.y0 * k]);
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new("cm", matrix_operands(&place)),
                Operation::new("cm", matrix_operands(&img.ctm)),
                draw,
                Operation::new("Q", vec![]),
            ],
        }
        .encode()?;
        let mut res = resources;
        // Neue, noch ungespeicherte Bilder stehen schon in der Momentaufnahme (gleicher Name).
        let content_id = pdf.add_object(Stream::new(dictionary! {}, content));
        let pages_id = pdf.new_object_id();
        let new_page = pdf.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), w.into(), h.into()],
            "Resources" => std::mem::take(&mut res),
            "Contents" => content_id,
            "Rotate" => i64::from(info.rotation),
        });
        pdf.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![new_page.into()], "Count" => 1,
            }),
        );
        let catalog = pdf.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        pdf.trailer.set("Root", catalog);
        pdf.trailer.remove(b"Info");
        pdf.prune_objects();
        let mut out = Vec::new();
        pdf.save_to(&mut out)
            .map_err(|e| CoreError::Other(format!("PDF schreiben: {e}")))?;
        let size = if info.rotation % 180 == 90 {
            (h, w)
        } else {
            (w, h)
        };
        Ok((out, size))
    }

    /// Prüft, ob `img` zum aktuellen Inhalt passt, und liefert Inhalt + Operationen.
    fn image_context(
        &self,
        page_idx: usize,
        img: &PageImage,
    ) -> Result<(lopdf::ObjectId, PageContent, Vec<Operation>)> {
        let page = self.page(page_idx)?.object_id;
        let pc = self.page_content(page_idx)?;
        if pc.stamp != img.stamp {
            return Err(CoreError::Other(
                "Der Seiteninhalt hat sich inzwischen geändert.".into(),
            ));
        }
        let ops = Content::decode(&pc.content)?.operations;
        let fits = ops
            .get(img.op)
            .is_some_and(|o| o.operator == "Do" || o.operator == "BI");
        if !fits {
            return Err(CoreError::Other(
                "Bild passt nicht zum Seiteninhalt.".into(),
            ));
        }
        Ok((page, pc, ops))
    }

    fn commit_ops(
        &mut self,
        page: lopdf::ObjectId,
        before: PageContent,
        mut after: PageContent,
        ops: Vec<Operation>,
        label: &str,
    ) -> Result<()> {
        after.content = Content { operations: ops }.encode()?;
        self.execute(Box::new(ReplacePageContent::new(
            page, before, after, label,
        )))
    }

    /// Verschiebt/skaliert ein Bild: neue Matrix = alte Matrix, danach `by` (im PDF-Raum).
    pub fn transform_image(&mut self, page_idx: usize, img: &PageImage, by: Matrix) -> Result<()> {
        let (page, pc, mut ops) = self.image_context(page_idx, img)?;
        set_matrix(&mut ops, img.op, &img.ctm, &img.ctm.then(&by))?;
        self.commit_ops(page, pc.clone(), pc, ops, "Bild verschieben")
    }

    /// Entfernt ein Bild aus dem Seiteninhalt.
    pub fn delete_image(&mut self, page_idx: usize, img: &PageImage) -> Result<()> {
        let (page, pc, mut ops) = self.image_context(page_idx, img)?;
        ops.remove(img.op);
        self.commit_ops(page, pc.clone(), pc, ops, "Bild löschen")
    }

    /// Ersetzt ein Bild. Das neue füllt den alten Bereich so weit wie möglich, ohne verzerrt
    /// zu werden (zentriert, gleiche Ausrichtung).
    pub fn replace_image(&mut self, page_idx: usize, img: &PageImage, new: NewImage) -> Result<()> {
        let (page, before, mut ops) = self.image_context(page_idx, img)?;
        let mut after = before.clone();
        let name = free_image_name(self, &after, page);
        let aspect = new.width as f32 / new.height.max(1) as f32;
        after.images.insert(name.clone(), new);
        ops[img.op] = Operation::new("Do", vec![Object::Name(name)]);

        // Seitenlängen des alten Bilds auf der Seite (Länge der Achsen der Matrix).
        let m = img.matrix.0;
        let (ow, oh) = (m[0].hypot(m[1]), m[2].hypot(m[3]));
        let (fw, fh) = if oh > 0.0 && ow / oh > aspect {
            ((oh * aspect) / ow, 1.0)
        } else if ow > 0.0 {
            (1.0, (ow / aspect.max(f32::EPSILON)) / oh.max(f32::EPSILON))
        } else {
            (1.0, 1.0)
        };
        let fit = Matrix([fw, 0.0, 0.0, fh, (1.0 - fw) / 2.0, (1.0 - fh) / 2.0]);
        // Bei Formularen gilt an der Operation die CTM (das neue Bild ersetzt das Formular).
        set_matrix(&mut ops, img.op, &img.ctm, &fit.then(&img.matrix))?;
        self.commit_ops(page, before, after, ops, "Bild ersetzen")
    }

    /// Fügt ein Bild ein: Mitte `center` (PDF-Raum), Breite `width` in Punkt (in der Anzeige),
    /// Höhe nach Seitenverhältnis; aufrecht in der Anzeige, auch auf gedrehten Seiten.
    pub fn insert_image(
        &mut self,
        page_idx: usize,
        new: NewImage,
        center: Point,
        width: f32,
    ) -> Result<()> {
        let info = self.page(page_idx)?;
        let (page, rotation) = (info.object_id, info.rotation);
        let before = self.page_content(page_idx)?;
        let mut after = before.clone();
        crate::text_edit::isolate(&mut after);
        let name = free_image_name(self, &after, page);
        let height = width * new.height as f32 / new.width.max(1) as f32;
        after.images.insert(name.clone(), new);

        let (sin, cos) = f32::from(rotation).to_radians().sin_cos();
        let m = Matrix([width, 0.0, 0.0, height, -width / 2.0, -height / 2.0])
            .then(&Matrix([cos, sin, -sin, cos, 0.0, 0.0]))
            .then(&Matrix::translate(center.x, center.y));
        let ops = vec![
            Operation::new("q", vec![]),
            Operation::new("cm", matrix_operands(&m)),
            Operation::new("Do", vec![Object::Name(name)]),
            Operation::new("Q", vec![]),
        ];
        crate::text_edit::append_ops(&mut after, ops)?;
        self.execute(Box::new(ReplacePageContent::new(
            page,
            before,
            after,
            "Bild einfügen",
        )))
    }
}
