//! Zeigt die gefundenen Systemschriften und die Zuordnung eines PDF-Schriftnamens.
//!
//! `cargo run -p pdftool-core --example font_list -- [PDF-Schriftname …]`

fn main() {
    let fonts = pdftool_core::system_fonts::system_fonts();
    println!("{} einbettbare TrueType-Schriften:", fonts.len());
    for f in fonts {
        println!("  {} | {} | {}", f.family, f.style, f.postscript);
    }
    for name in std::env::args().skip(1) {
        match pdftool_core::system_fonts::find_for_base_font(&name) {
            Some(f) => println!("{name} -> {} ({})", f.full_name, f.path.display()),
            None => println!("{name} -> keine passende Systemschrift"),
        }
    }
}
