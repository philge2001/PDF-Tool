//! Formular auf der Kommandozeile ausfüllen – zur Gegenprobe mit anderen Readern.
//!
//! `cargo run -p foliocraft-core --example form_fill -- in.pdf` listet alle Felder.
//! `cargo run -p foliocraft-core --example form_fill -- in.pdf out.pdf name=Wert …` setzt Werte:
//! Text direkt, Kontrollkästchen `an`/`aus`, Optionsfeld Zustandsname, Auswahl mit `|` getrennt.

use foliocraft_core::{Document, FieldKind, FieldValue, UpdateField};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(input) = args.first() else {
        eprintln!("Aufruf: form_fill in.pdf [out.pdf name=Wert …]");
        std::process::exit(2);
    };
    let mut doc = Document::open(input).expect("PDF öffnen");
    if args.len() < 2 {
        for f in doc.forms().iter() {
            println!(
                "{:<20} {:<16} {:?}{}{}",
                f.name,
                f.kind.label(),
                f.value,
                if f.read_only {
                    " [schreibgeschützt]"
                } else {
                    ""
                },
                if f.required { " [Pflicht]" } else { "" }
            );
        }
        return;
    }
    for pair in &args[2..] {
        let (name, raw) = pair.split_once('=').expect("Format name=Wert");
        let field = doc
            .forms()
            .by_name(name)
            .unwrap_or_else(|| panic!("Feld {name} gibt es nicht"))
            .clone();
        let value = match field.kind {
            FieldKind::CheckBox => FieldValue::Toggle(match raw {
                "an" => field.widgets[0].on_state.clone(),
                _ => None,
            }),
            FieldKind::Radio => FieldValue::Toggle(Some(raw.to_string())),
            FieldKind::Combo { .. } | FieldKind::List { .. } => {
                FieldValue::Choice(raw.split('|').map(str::to_string).collect())
            }
            _ => FieldValue::Text(raw.replace("\\n", "\n")),
        };
        doc.execute(Box::new(UpdateField::value(&field, value)))
            .expect("Wert setzen");
    }
    doc.save_as(&args[1]).expect("speichern");
    println!("geschrieben: {}", args[1]);
}
