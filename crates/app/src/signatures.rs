//! Signaturen anzeigen: Hinweisleiste, Signatur-Panel, Vertrauensspeicher.
//!
//! Geprüft werden immer die DATEIBYTES (`Document::bytes`) – ungespeicherte Änderungen
//! betreffen Signaturen nicht. Neu geprüft wird, wenn sich die Bytes (nach dem Speichern)
//! oder der Vertrauensspeicher ändern.

use crate::app::{OpenDocument, PdfToolApp};
use crate::viewer::transform;
use eframe::egui::{self, Color32, RichText};
use pdftool_sign::{Integrity, SignatureReport, Status, Trust, TrustStore, cert};
use std::sync::Arc;

pub const GREEN: Color32 = Color32::from_rgb(40, 160, 70);
pub const AMBER: Color32 = Color32::from_rgb(220, 150, 20);
pub const RED: Color32 = Color32::from_rgb(220, 50, 50);

/// Geprüfte Signaturen eines Dokuments.
#[derive(Default)]
pub struct SigState {
    pub reports: Vec<SignatureReport>,
    /// Für welche Bytes / welchen Vertrauensstand gilt `reports`?
    checked: Option<(Arc<Vec<u8>>, u64)>,
}

/// Farbiger Statuspunkt (gezeichnet – nicht jede Schrift hat ein Kreiszeichen).
pub fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

pub fn status_color(s: Status) -> Color32 {
    match s {
        Status::Valid => GREEN,
        Status::UnknownSigner => AMBER,
        Status::Invalid => RED,
    }
}

/// Was im Panel angeklickt wurde (wird nach dem Zeichnen ausgeführt, weil dafür
/// andere Teile von `PdfToolApp` veränderlich ausgeliehen werden müssen).
enum PanelAction {
    Show(usize),
    Trust(Vec<u8>),
    SaveSignedVersion(usize),
}

impl OpenDocument {
    /// Prüft neu, falls sich Dateibytes oder Vertrauensspeicher geändert haben.
    pub fn update_signatures(&mut self, trust: &TrustStore, trust_version: u64) {
        let bytes = self.doc.bytes();
        // `Arc::ptr_eq`: dieselben Bytes im Speicher? (Nach dem Speichern gibt es ein neues Arc.)
        let fresh = self
            .signatures
            .checked
            .as_ref()
            .is_some_and(|(b, v)| Arc::ptr_eq(b, bytes) && *v == trust_version);
        if fresh {
            return;
        }
        self.signatures.reports = pdftool_sign::verify_document(&self.doc, trust);
        self.signatures.checked = Some((bytes.clone(), trust_version));
    }
}

impl PdfToolApp {
    /// Vertrauensspeicher aus den Einstellungen neu aufbauen.
    pub(crate) fn rebuild_trust(&mut self) {
        self.trust =
            TrustStore::from_pem_list(self.settings.trusted_certs.iter().map(String::as_str));
        self.trust_version += 1;
    }

    /// Fügt ein Zertifikat (DER) dem Vertrauensspeicher hinzu.
    pub(crate) fn trust_certificate(&mut self, der: &[u8]) {
        let Some(pem) = cert::to_pem(der) else {
            self.error = Some("Zertifikat konnte nicht gelesen werden.".into());
            return;
        };
        if !self.settings.trusted_certs.contains(&pem) {
            self.settings.trusted_certs.push(pem);
            self.rebuild_trust();
        }
    }

    /// Leiste über dem Dokument, wenn es Signaturen enthält.
    pub(crate) fn signature_banner(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &self.document else { return };
        let reports = &d.signatures.reports;
        let worst = reports
            .iter()
            .map(SignatureReport::status)
            .max_by_key(|s| match s {
                Status::Valid => 0,
                Status::UnknownSigner => 1,
                Status::Invalid => 2,
            })
            .unwrap_or(Status::Valid);
        let text = match (reports.len(), worst) {
            (1, _) => format!(
                "Signiert von {}: {}",
                reports[0].signer,
                reports[0].summary()
            ),
            (n, Status::Valid) => format!("{n} Signaturen, alle gültig"),
            (n, Status::UnknownSigner) => {
                format!("{n} Signaturen – mindestens eine Identität nicht bestätigt")
            }
            (n, Status::Invalid) => format!("{n} Signaturen – mindestens eine ungültig"),
        };
        ui.horizontal(|ui| {
            status_dot(ui, status_color(worst));
            ui.label(text);
            let open = self.right_pane == Some(crate::shell::RightPane::Signatures);
            let label = if open {
                "Signaturen ausblenden"
            } else {
                "Signaturen anzeigen"
            };
            if ui.button(label).clicked() {
                self.toggle_pane(crate::shell::RightPane::Signatures);
            }
        });
    }

    /// Rechtes Panel mit allen Signaturen.
    pub(crate) fn signatures_panel(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &self.document else { return };
        if d.signatures.reports.is_empty() {
            ui.label("Dieses Dokument enthält keine Signaturen.");
            return;
        }
        let mut action = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, r) in d.signatures.reports.iter().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    report_ui(ui, i, r, &mut action);
                });
                ui.add_space(6.0);
            }
        });
        match action {
            Some(PanelAction::Show(i)) => self.show_signature_field(i),
            Some(PanelAction::Trust(der)) => self.trust_certificate(&der),
            Some(PanelAction::SaveSignedVersion(i)) => self.save_signed_version(i),
            None => {}
        }
    }

    fn show_signature_field(&mut self, i: usize) {
        let Some(d) = &mut self.document else { return };
        let Some(r) = d.signatures.reports.get(i) else {
            return;
        };
        let Some(page) = r.page else { return };
        match (r.rect, d.doc.page(page)) {
            (Some(rect), Ok(info)) if rect.width() > 0.0 && rect.height() > 0.0 => {
                let unit = transform::pdf_rect_to_unit(info, rect);
                d.viewer.reveal(page, unit);
            }
            _ => d.viewer.scroll_to_page(page),
        }
    }

    /// Speichert die Datei so, wie sie zum Zeitpunkt der Signatur war.
    fn save_signed_version(&mut self, i: usize) {
        let Some(d) = &self.document else { return };
        let Some(r) = d.signatures.reports.get(i) else {
            return;
        };
        let bytes = d.doc.bytes();
        let Some(signed) = bytes.get(..r.signed_len) else {
            return;
        };
        let name = format!(
            "{}-signierte-Fassung.pdf",
            d.file_name().trim_end_matches(".pdf")
        );
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(name)
            .save_file()
        else {
            return;
        };
        if let Err(e) = std::fs::write(&path, signed) {
            self.error = Some(format!(
                "„{}“ konnte nicht gespeichert werden:\n{e}",
                path.display()
            ));
        } else {
            self.info = Some(format!(
                "Signierte Fassung gespeichert:\n{}",
                path.display()
            ));
        }
    }

    /// Abschnitt "Vertrauenswürdige Zertifikate" im Einstellungsfenster.
    pub(crate) fn trust_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Vertrauenswürdige Zertifikate").strong());
        ui.label(
            RichText::new(
                "Signaturen gelten als gültig, wenn ihre Zertifikatskette bei einem dieser \
                 Zertifikate endet.",
            )
            .weak(),
        );
        let mut remove = None;
        if self.trust.anchors().is_empty() {
            ui.label("(keine)");
        }
        for (i, c) in self.trust.anchors().iter().enumerate() {
            let info = cert::CertInfo::of(c);
            ui.horizontal(|ui| {
                ui.label(&info.name).on_hover_text(format!(
                    "{}\nGültig bis {}\nSHA-256: {}",
                    info.subject, info.not_after, info.fingerprint
                ));
                if ui.small_button("Entfernen").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            // Reihenfolge in den Einstellungen = Reihenfolge im Speicher (unlesbare fehlen dort).
            let der = der::Encode::to_der(&self.trust.anchors()[i]).unwrap_or_default();
            if let Some(pem) = cert::to_pem(&der) {
                self.settings.trusted_certs.retain(|p| p != &pem);
            }
            self.rebuild_trust();
        }
        if ui.button("Zertifikat importieren…").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Zertifikat", &["cer", "crt", "der", "pem"])
                .pick_file()
        {
            match std::fs::read(&path)
                .ok()
                .and_then(|b| cert::parse_certificate(&b))
            {
                Some(c) => {
                    let der = der::Encode::to_der(&c).unwrap_or_default();
                    self.trust_certificate(&der);
                }
                None => {
                    self.error = Some(format!(
                        "„{}“ ist kein lesbares Zertifikat (erwartet: DER oder PEM).",
                        path.display()
                    ));
                }
            }
        }
    }
}

fn report_ui(ui: &mut egui::Ui, i: usize, r: &SignatureReport, action: &mut Option<PanelAction>) {
    let color = status_color(r.status());
    ui.horizontal(|ui| {
        status_dot(ui, color);
        ui.label(RichText::new(r.summary()).strong().color(color));
    });
    if r.document_timestamp {
        ui.label(format!("Dokument-Zeitstempel von: {}", r.signer));
    } else {
        ui.label(format!("Unterzeichnet von: {}", r.signer));
    }
    match &r.timestamp {
        Some(ts) if ts.integrity.is_ok() => {
            let who = match ts.trust {
                Trust::Trusted => ts.tsa.clone(),
                _ => format!("{} (Dienst nicht als vertrauenswürdig hinterlegt)", ts.tsa),
            };
            ui.label(format!("Zeitstempel: {}", ts.time))
                .on_hover_text(format!("Unabhängig bestätigt von {who}"));
        }
        Some(ts) => {
            let e = ts.integrity.as_ref().err().cloned().unwrap_or_default();
            ui.colored_label(RED, format!("Zeitstempel ungültig: {e}"));
        }
        None => {
            if let Some(t) = &r.time {
                ui.label(format!("Zeit: {t}"))
                    .on_hover_text("Vom Unterzeichner angegeben, nicht unabhängig bestätigt");
            }
        }
    }
    if !r.reason.is_empty() {
        ui.label(format!("Grund: {}", r.reason));
    }
    if !r.location.is_empty() {
        ui.label(format!("Ort: {}", r.location));
    }
    if let Some(p) = r.certification {
        let what = match p {
            1 => "keine Änderungen erlaubt",
            3 => "Formulare ausfüllen, signieren und kommentieren erlaubt",
            _ => "Formulare ausfüllen und signieren erlaubt",
        };
        ui.label(format!("Zertifizierungssignatur ({what})"));
    }
    ui.horizontal(|ui| {
        let place = match r.page {
            Some(p) => format!("Feld „{}“, Seite {}", r.field_name, p + 1),
            None => format!("Feld „{}“", r.field_name),
        };
        ui.label(RichText::new(place).weak());
        if r.page.is_some() && ui.small_button("Anzeigen").clicked() {
            *action = Some(PanelAction::Show(i));
        }
    });

    egui::CollapsingHeader::new("Details")
        .id_salt(("sig_details", i))
        .show(ui, |ui| {
            let integrity = match &r.integrity {
                Integrity::Intact => "Der signierte Inhalt ist unverändert.".to_string(),
                Integrity::Broken(e) => format!("UNGÜLTIG: {e}"),
                Integrity::Unsupported(e) => format!("Nicht prüfbar: {e}"),
            };
            ui.label(integrity);
            let trust = match &r.trust {
                Trust::Trusted => {
                    "Die Zertifikatskette endet bei einem vertrauenswürdigen Zertifikat."
                        .to_string()
                }
                Trust::Unknown(e) | Trust::Invalid(e) => e.clone(),
            };
            if r.integrity == Integrity::Intact {
                ui.label(trust);
            }
            if r.later_revisions > 0 {
                ui.label(
                    "Nach dem Signieren wurde die Datei ergänzt (z. B. Formular ausgefüllt, \
                     Kommentare, weitere Signatur). Der signierte Stand selbst ist davon nicht \
                     betroffen.",
                );
                if ui.button("Signierte Fassung speichern…").clicked() {
                    *action = Some(PanelAction::SaveSignedVersion(i));
                }
            }
            ui.label(
                RichText::new(format!(
                    "Verfahren: {} · {}",
                    r.sub_filter,
                    r.hash.map_or("?", |h| h.name())
                ))
                .weak(),
            );
        });

    if !r.chain.is_empty() {
        egui::CollapsingHeader::new("Zertifikate")
            .id_salt(("sig_certs", i))
            .show(ui, |ui| {
                for c in &r.chain {
                    ui.label(RichText::new(&c.name).strong());
                    ui.label(format!("Ausgestellt von: {}", c.issuer));
                    ui.label(format!("Gültig: {} – {}", c.not_before, c.not_after));
                    ui.label(
                        RichText::new(format!("SHA-256: {}", c.fingerprint))
                            .small()
                            .weak(),
                    );
                    ui.add_space(4.0);
                }
            });
    }
    // Vertrauen aussprechen: für das oberste Zertifikat der Kette (meist das Stammzertifikat).
    if matches!(r.trust, Trust::Unknown(_))
        && r.integrity == Integrity::Intact
        && let Some(top) = r.chain.last()
        && ui
            .button(format!("„{}“ vertrauen", top.name))
            .on_hover_text(
                "Nur, wenn Sie sicher sind, dass das Zertifikat echt ist (z. B. Fingerabdruck \
                 mit dem Aussteller abgeglichen).",
            )
            .clicked()
    {
        *action = Some(PanelAction::Trust(top.der.clone()));
    }
}
