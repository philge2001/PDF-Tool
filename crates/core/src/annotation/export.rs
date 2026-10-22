//! Modell -> PDF-Annotations-Dictionary (ohne /AP – das hängt `save` an).

use super::{AnnotKind, Annotation};
use crate::pdf_string;
use lopdf::{Dictionary, Object, ObjectId};

fn num_array(values: impl IntoIterator<Item = f32>) -> Object {
    Object::Array(values.into_iter().map(Object::Real).collect())
}

/// Baut das Dictionary einer Annotation.
///
/// `base`: Bei importierten Annotationen das Original-Dictionary. Unbekannte Schlüssel
/// anderer Programme bleiben so erhalten; nur unsere Schlüssel werden überschrieben.
pub fn to_dict(a: &Annotation, page_id: ObjectId, base: Option<&Dictionary>) -> Dictionary {
    let mut d = base.cloned().unwrap_or_default();
    // Veraltete Darstellungen/Rich-Text entfernen – wir liefern eine neue /AP.
    for key in [
        &b"AP"[..],
        b"AS",
        b"RC",
        b"QuadPoints",
        b"InkList",
        b"L",
        b"LE",
        b"RD",
    ] {
        d.remove(key);
    }

    let r = a.bounds();
    let c = a.color;
    d.set("Type", "Annot");
    d.set("Subtype", a.kind.subtype());
    d.set("Rect", num_array([r.x0, r.y0, r.x1, r.y1]));
    d.set("P", page_id);
    d.set("F", 4); // Print: auch beim Drucken sichtbar
    d.set("Contents", pdf_string::encode(&a.contents));
    d.set("T", pdf_string::encode(&a.author));
    d.set("NM", pdf_string::encode(&a.name));
    d.set("M", Object::string_literal(a.modified.clone()));
    d.set("CA", a.opacity);

    // Bei FreeText ist /C die HINTERGRUNDfarbe; die Textfarbe steht in /DA.
    if !matches!(a.kind, AnnotKind::FreeText { .. }) {
        d.set("C", num_array([c.r, c.g, c.b]));
    }

    let border = |w: f32| -> Object { lopdf::dictionary! { "W" => w, "S" => "S" }.into() };
    match &a.kind {
        AnnotKind::Highlight { quads }
        | AnnotKind::Underline { quads }
        | AnnotKind::StrikeOut { quads } => {
            d.set(
                "QuadPoints",
                num_array(quads.iter().flat_map(|q| q.points).flat_map(|p| [p.x, p.y])),
            );
        }
        AnnotKind::Note { .. } => {
            d.set("Name", "Comment");
            d.set("Open", false);
        }
        AnnotKind::Ink { strokes, width } => {
            d.set(
                "InkList",
                Object::Array(
                    strokes
                        .iter()
                        .map(|s| num_array(s.iter().flat_map(|p| [p.x, p.y])))
                        .collect(),
                ),
            );
            d.set("BS", border(*width));
        }
        AnnotKind::Square { width, .. } | AnnotKind::Circle { width, .. } => {
            d.set("BS", border(*width));
        }
        AnnotKind::Line {
            start,
            end,
            width,
            arrow_end,
        } => {
            d.set("L", num_array([start.x, start.y, end.x, end.y]));
            let le = if *arrow_end { "OpenArrow" } else { "None" };
            d.set("LE", vec![Object::from("None"), Object::from(le)]);
            d.set("BS", border(*width));
        }
        AnnotKind::FreeText { font_size, .. } => {
            let da = format!("/Helv {} Tf {} {} {} rg", font_size, c.r, c.g, c.b);
            d.set("DA", Object::string_literal(da));
            d.set("Q", 0); // linksbündig
            d.set("BS", border(0.0));
            d.remove(b"C");
        }
    }
    d
}
