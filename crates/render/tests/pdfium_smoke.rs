//! Rendert eine Fixture-Seite mit echtem PDFium. Wird übersprungen (nicht fehlgeschlagen),
//! wenn die PDFium-Bibliothek nicht gefunden wird – z. B. in CI.
//!
//! Lokal: FOLIOCRAFT_PDFIUM_PATH=/pfad/zu/pdfium/lib cargo test -p foliocraft-render

use foliocraft_core::{AddAnnotation, AnnotKind, Annotation, Color, Document, Quad, Rect};
use foliocraft_render::{Event, Priority, RenderKey, RenderRequest, RenderService};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixture(name: &str) -> Vec<u8> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read(p).expect("Fixture lesen")
}

/// Wartet bis zu 10 s auf ein Ereignis, das `pred` erfüllt.
fn wait_for(svc: &RenderService, mut pred: impl FnMut(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        for ev in svc.poll() {
            if pred(&ev) {
                return ev;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("Zeitüberschreitung beim Warten auf Render-Ereignis");
}

#[test]
fn renders_page_and_tile() {
    let Ok(svc) = RenderService::start(|| {}) else {
        eprintln!("PDFium nicht gefunden – Test übersprungen");
        return;
    };
    svc.open(
        1,
        Arc::new(fixture("text_two_pages.pdf")),
        Default::default(),
    );
    match wait_for(&svc, |e| {
        matches!(e, Event::Opened { .. } | Event::OpenFailed { .. })
    }) {
        Event::Opened { page_count, .. } => assert_eq!(page_count, 2),
        _ => panic!("Öffnen fehlgeschlagen"),
    }

    let key = RenderKey {
        doc: 1,
        page: 0,
        scale_milli: 1000,
        tile: None,
    };
    svc.request(RenderRequest {
        key,
        page_size_pt: (595.0, 842.0),
        priority: Priority::Visible,
    });
    let Event::Rendered { image, .. } = wait_for(&svc, |e| matches!(e, Event::Rendered { .. }))
    else {
        unreachable!()
    };
    assert_eq!((image.width, image.height), (595, 842));
    assert_eq!(image.pixels.len(), 595 * 842 * 4);
    // Oben links ist weißer Rand, irgendwo muss dunkler Text sein.
    assert_eq!(&image.pixels[0..4], &[255, 255, 255, 255]);
    assert!(
        image
            .pixels
            .chunks(4)
            .any(|p| p[0] < 100 && p[1] < 100 && p[2] < 100)
    );

    // Kachel (1,0) bei 400 %: Seite 2380x3368 px -> Kachel x=1024..2048, y=0..1024
    let key = RenderKey {
        doc: 1,
        page: 0,
        scale_milli: 4000,
        tile: Some((1, 0)),
    };
    svc.request(RenderRequest {
        key,
        page_size_pt: (595.0, 842.0),
        priority: Priority::Visible,
    });
    let Event::Rendered { image, .. } = wait_for(&svc, |e| matches!(e, Event::Rendered { .. }))
    else {
        unreachable!()
    };
    assert_eq!((image.width, image.height), (1024, 1024));

    // Text mit Positionen: "Suchwort: Zebra. Noch einmal ZEBRA und zebra." steht bei y = 660.
    svc.extract_text(1, 0);
    let Event::Text { text, .. } = wait_for(&svc, |e| matches!(e, Event::Text { .. })) else {
        unreachable!()
    };
    let hits = text.find("zebra", false);
    assert_eq!(hits.len(), 3);
    let rects = text.range_rects(hits[0]);
    assert_eq!(rects.len(), 1);
    let r = rects[0];
    assert!(r.y0 > 650.0 && r.y1 < 680.0, "Zeilenposition: {r:?}");
    assert!(
        r.x0 > 72.0 && r.width() > 20.0 && r.width() < 60.0,
        "Wortbreite: {r:?}"
    );
    assert!(
        text.text_in(foliocraft_core::TextRange::new(0, 40))
            .contains("FolioCraft Testdokument")
    );

    // ---- Annotationen: von core gespeichert, von PDFium gezeichnet bzw. ausgeblendet ----
    let mut doc = Document::from_bytes(fixture("text_two_pages.pdf")).unwrap();
    let mut a = Annotation::new(
        0,
        AnnotKind::Highlight {
            quads: vec![Quad::from_rect(Rect::new(300.0, 300.0, 400.0, 350.0))],
        },
        Color::YELLOW,
    );
    a.id = doc.allocate_annotation_id();
    doc.execute(Box::new(AddAnnotation::new(a))).unwrap();
    let saved = Document::from_bytes(doc.to_bytes().unwrap()).unwrap();
    let pixel_in_highlight = |svc: &RenderService, doc_id| {
        let key = RenderKey {
            doc: doc_id,
            page: 0,
            scale_milli: 1000,
            tile: None,
        };
        svc.request(RenderRequest {
            key,
            page_size_pt: (595.0, 842.0),
            priority: Priority::Visible,
        });
        let Event::Rendered { image, .. } = wait_for(svc, |e| matches!(e, Event::Rendered { .. }))
        else {
            unreachable!()
        };
        // PDF (350, 325) -> Pixel (350, 842 - 325)
        let i = ((842 - 325) * 595 + 350) * 4;
        [image.pixels[i], image.pixels[i + 1], image.pixels[i + 2]]
    };

    svc.open(2, Arc::new(saved.bytes().to_vec()), Default::default());
    let [r, g, b] = pixel_in_highlight(&svc, 2);
    assert!(
        r > 200 && g > 180 && b < 200,
        "PDFium zeigt unsere Hervorhebung: {r},{g},{b}"
    );

    let hidden = [(0u32, saved.managed_annot_indices(0))]
        .into_iter()
        .collect();
    assert_eq!(saved.managed_annot_indices(0), vec![0]);
    svc.open(3, Arc::new(saved.bytes().to_vec()), hidden);
    assert_eq!(pixel_in_highlight(&svc, 3), [255, 255, 255], "ausgeblendet");
}
