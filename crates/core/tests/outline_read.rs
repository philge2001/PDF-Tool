//! Lesezeichen lesen: verschachtelt, /Dest, /GoTo, benannte Ziele, Farbe/Stil, Weblink.

use pdftool_core::Document;
use pdftool_core::outline::Destination;

fn fixture(name: &str) -> Document {
    Document::open(format!(
        "{}/../../tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn reads_bookmarks() {
    let doc = fixture("bookmarks.pdf");
    let o = doc.outline();
    let titles: Vec<&str> = o.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["Kapitel 1", "Kapitel 2", "Anhang", "Projektseite"]);

    let k1 = &o[0];
    assert!(k1.open);
    assert_eq!(
        k1.dest,
        Some(Destination {
            page: 0,
            top: Some(800.0),
            left: Some(0.0)
        })
    );
    assert_eq!(k1.children.len(), 1);
    let s = &k1.children[0];
    assert_eq!(s.title, "Abschnitt 1.1");
    assert_eq!(
        s.dest.as_ref().map(|d| (d.page, d.top)),
        Some((0, Some(520.0)))
    );

    // Benanntes Ziel über den Namensbaum.
    assert_eq!(o[1].dest.as_ref().map(|d| d.page), Some(1));
    // FitH, rot, fett.
    assert_eq!(
        o[2].dest.as_ref().map(|d| (d.page, d.top)),
        Some((2, Some(700.0)))
    );
    assert!(o[2].bold && !o[2].italic);
    assert_eq!(o[2].color, Some([0.8, 0.0, 0.0]));
    // Weblink ohne Seite.
    assert_eq!(o[3].dest, None);
    assert_eq!(o[3].uri.as_deref(), Some("https://example.org"));

    assert!(!fixture("text_two_pages.pdf").has_outline());
}
