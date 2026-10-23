//! Signieren in der App: Dialog, Bereich auf der Seite wählen, Testzertifikat erstellen.
//!
//! Das Passwort und der geöffnete Schlüssel leben nur im Dialog (im Arbeitsspeicher) und
//! verschwinden, sobald er geschlossen wird. Gespeichert wird nur der Pfad zur Zertifikatsdatei.

use crate::app::PdfToolApp;
use crate::viewer::{ViewOutput, transform};
use eframe::egui::{self, RichText, Stroke, StrokeKind};
use pdftool_core::signature_prep::{self, SigPlacement, SignatureSpec};
use pdftool_core::{Point, Rect as PdfRect, date};
use pdftool_sign::{KeyKind, Signer, sign_pdf};
use std::path::PathBuf;

/// Öffentliche, kostenlose Zeitstempeldienste (RFC 3161) als Vorschlag.
const TSA_SUGGESTIONS: [&str; 3] = [
    "http://timestamp.digicert.com",
    "http://timestamp.sectigo.com",
    "https://freetsa.org/tsr",
];

/// Standardgröße eines sichtbaren Signaturfelds bei einfachem Klick (Punkt).
const DEFAULT_SIZE: (f32, f32) = (200.0, 60.0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SignMode {
    Visible,
    Invisible,
    Existing,
}

pub struct SignDialog {
    password: String,
    signer: Option<Signer>,
    load_error: Option<String>,
    mode: SignMode,
    /// Noch leere Signaturfelder im Dokument und das gewählte.
    existing: Vec<String>,
    existing_choice: usize,
    /// Gewählter Bereich: (Seitenindex, Rechteck im PDF-Raum).
    area: Option<(usize, PdfRect)>,
    /// Wartet darauf, dass der Nutzer den Bereich aufzieht (Dialog ist solange ausgeblendet).
    placing: bool,
    /// Laufender Zug: Seite und Startpunkt.
    drag: Option<(usize, Point, PdfRect)>,
}

pub struct CertDialog {
    name: String,
    organization: String,
    email: String,
    years: u32,
    kind: KeyKind,
    password: String,
    repeat: String,
    trust_it: bool,
    error: Option<String>,
}

impl PdfToolApp {
    pub(crate) fn open_sign_dialog(&mut self) {
        let Some(d) = &self.document else { return };
        let existing = signature_prep::empty_signature_fields(&d.doc);
        self.sign_dialog = Some(SignDialog {
            password: String::new(),
            signer: None,
            load_error: None,
            mode: if existing.is_empty() {
                SignMode::Visible
            } else {
                SignMode::Existing
            },
            existing,
            existing_choice: 0,
            area: None,
            placing: false,
            drag: None,
        });
    }

    pub(crate) fn open_cert_dialog(&mut self) {
        self.cert_dialog = Some(CertDialog {
            name: self.settings.author.clone(),
            organization: String::new(),
            email: String::new(),
            years: 3,
            kind: KeyKind::Rsa2048,
            password: String::new(),
            repeat: String::new(),
            trust_it: true,
            error: None,
        });
    }

    /// Ist gerade das Aufziehen des Signaturbereichs aktiv? (Dann ruhen die Werkzeuge.)
    pub(crate) fn is_placing_signature(&self) -> bool {
        self.sign_dialog.as_ref().is_some_and(|d| d.placing)
    }

    /// Bereich für die sichtbare Signatur aufziehen (oder klicken = Standardgröße).
    pub(crate) fn handle_sign_placement(&mut self, ui: &egui::Ui, view: &ViewOutput) {
        let (Some(dlg), Some(d)) = (&mut self.sign_dialog, &self.document) else {
            return;
        };
        let resp = &view.response;
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            dlg.placing = false;
            dlg.drag = None;
            return;
        }
        let to_pdf = |pos| {
            let (page, rect) = view.page_at(pos)?;
            Some((
                page,
                transform::screen_to_pdf(d.doc.page(page).ok()?, rect, pos),
            ))
        };
        if resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && let Some((page, p)) = to_pdf(origin)
        {
            dlg.drag = Some((page, p, PdfRect::new(p.x, p.y, p.x, p.y)));
        }
        if resp.dragged()
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some((page, start, _)) = dlg.drag
            && let Some((_, rect)) = view.pages.iter().find(|(i, _)| *i == page)
            && let Ok(info) = d.doc.page(page)
        {
            let p = transform::screen_to_pdf(info, *rect, pos);
            dlg.drag = Some((page, start, PdfRect::from_points(start, p)));
        }
        let finished = if resp.drag_stopped() {
            dlg.drag.take().map(|(page, _, r)| (page, r.normalized()))
        } else if resp.clicked() {
            resp.interact_pointer_pos()
                .and_then(to_pdf)
                .map(|(page, p)| {
                    let (w, h) = DEFAULT_SIZE;
                    (page, PdfRect::new(p.x, p.y - h, p.x + w, p.y))
                })
        } else {
            None
        };
        if let Some((page, mut r)) = finished {
            // Zu kleine Züge auf die Standardgröße bringen.
            if r.width() < 40.0 || r.height() < 15.0 {
                r = PdfRect::new(r.x0, r.y1 - DEFAULT_SIZE.1, r.x0 + DEFAULT_SIZE.0, r.y1);
            }
            dlg.area = Some((page, r));
            dlg.placing = false;
        }
    }

    /// Gewählten bzw. gerade aufgezogenen Bereich anzeigen.
    pub(crate) fn paint_sign_area(&self, ui: &egui::Ui, view: &ViewOutput) {
        let (Some(dlg), Some(d)) = (&self.sign_dialog, &self.document) else {
            return;
        };
        let current = dlg
            .drag
            .map(|(p, _, r)| (p, r))
            .or(dlg.area.filter(|_| dlg.mode == SignMode::Visible));
        let Some((page, r)) = current else { return };
        let Some((_, page_rect)) = view.pages.iter().find(|(i, _)| *i == page) else {
            return;
        };
        let Ok(info) = d.doc.page(page) else { return };
        let sr = transform::pdf_rect_to_screen(info, *page_rect, r.normalized());
        let painter = ui.painter().with_clip_rect(view.clip);
        painter.rect_filled(sr, 0.0, crate::form_draw::HIGHLIGHT);
        painter.rect_stroke(
            sr,
            0.0,
            Stroke::new(1.5, crate::form_draw::FOCUS),
            StrokeKind::Middle,
        );
        painter.text(
            sr.center(),
            egui::Align2::CENTER_CENTER,
            "Signatur",
            egui::FontId::proportional(13.0),
            crate::form_draw::FOCUS,
        );
    }

    pub(crate) fn sign_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.sign_dialog else {
            return;
        };
        if dlg.placing {
            // Hinweis statt Dialog, solange der Bereich gewählt wird.
            egui::Area::new(egui::Id::unique("sign_place_hint"))
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 110.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(
                            "Bereich für die Signatur aufziehen (oder klicken). Esc bricht ab.",
                        );
                    });
                });
            return;
        }
        let mut open = true;
        let mut action = None;
        egui::Window::new("Dokument signieren")
            .open(&mut open)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                action = sign_dialog_body(ui, dlg, &mut self.settings);
            });
        if !open {
            self.sign_dialog = None;
            return;
        }
        match action {
            Some(DialogAction::Sign) => self.sign_now(ctx),
            Some(DialogAction::CreateCert) => self.open_cert_dialog(),
            Some(DialogAction::Browse) => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Digitale ID (PKCS#12)", &["p12", "pfx"])
                    .pick_file()
                {
                    self.settings.signing_cert_path = Some(path);
                    if let Some(dlg) = &mut self.sign_dialog {
                        dlg.signer = None;
                        dlg.load_error = None;
                    }
                }
            }
            None => {}
        }
    }

    fn sign_now(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&self.sign_dialog, &self.document) else {
            return;
        };
        let Some(signer) = &dlg.signer else { return };
        if self.job.is_some() {
            return; // es läuft schon etwas
        }
        let name = signature_prep::free_signature_name(&d.doc);
        let placement = match dlg.mode {
            SignMode::Visible => match dlg.area {
                Some((page, rect)) => SigPlacement::New { page, rect, name },
                None => return,
            },
            SignMode::Invisible => SigPlacement::New {
                page: d
                    .viewer
                    .current_page
                    .min(d.doc.page_count().saturating_sub(1)),
                rect: PdfRect::new(0.0, 0.0, 0.0, 0.0),
                name,
            },
            SignMode::Existing => match dlg.existing.get(dlg.existing_choice) {
                Some(n) => SigPlacement::Existing(n.clone()),
                None => return,
            },
        };
        let now = date::pdf_date_now();
        let who = signer.info().name;
        let s = &self.settings;
        let mut lines = vec![
            format!("Digital signiert von {who}"),
            format!("Datum: {}", date::display_pdf_date(&now)),
        ];
        if !s.sign_reason.is_empty() {
            lines.push(format!("Grund: {}", s.sign_reason));
        }
        if !s.sign_location.is_empty() {
            lines.push(format!("Ort: {}", s.sign_location));
        }
        let spec = SignatureSpec {
            placement,
            sub_filter: String::new(),
            name: who,
            reason: s.sign_reason.clone(),
            location: s.sign_location.clone(),
            contact: s.sign_contact.clone(),
            date: now,
            appearance_lines: lines,
            contents_len: 0,
        };
        // Ungespeicherte Änderungen kommen mit in die signierte Fassung.
        let base = if d.doc.is_dirty() {
            match d.doc.to_bytes() {
                Ok(b) => b,
                Err(e) => {
                    self.error = Some(format!("Dokument konnte nicht vorbereitet werden:\n{e}"));
                    return;
                }
            }
        } else {
            d.doc.bytes().to_vec()
        };
        // Erst das Ziel wählen, dann signieren (die Signatur deckt die fertigen Bytes ab).
        let stem = d.file_name().trim_end_matches(".pdf").to_string();
        let mut dialog = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(format!("{stem}-signiert.pdf"));
        if let Some(dir) = d.doc.path().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return; // abgebrochen – nichts geschrieben
        };
        let tsa_url = (self.settings.tsa_enabled && !self.settings.tsa_url.trim().is_empty())
            .then(|| self.settings.tsa_url.trim().to_string());

        // Mit Zeitstempel braucht es das Netzwerk -> Hintergrund-Job, die Oberfläche bleibt bedienbar.
        // Der Job bekommt eigene Kopien von allem (`move`), weil er in einem anderen Thread läuft.
        let signer = signer.clone();
        let target = path.clone();
        let work = move |ctx: &crate::jobs::JobCtx| -> crate::jobs::JobResult {
            let transport = tsa_url.as_ref().map(|url| {
                ctx.progress(0.3, format!("Zeitstempel von {url} …"));
                move |req: &[u8]| http_tsa(url, req)
            });
            let tsa: Option<pdftool_sign::timestamp::TsaTransport> =
                transport.as_ref().map(|f| f as _);
            let signed = sign_pdf(&base, spec, &signer, tsa)
                .map_err(|e| format!("Signieren fehlgeschlagen:\n{e}"))?;
            std::fs::write(&target, signed).map_err(|e| {
                format!(
                    "„{}“ konnte nicht gespeichert werden:\n{e}",
                    target.display()
                )
            })?;
            Ok(format!("Signiert und gespeichert:\n{}", target.display()))
        };
        self.sign_dialog = None;
        self.open_after_job = Some(path);
        self.job = Some(crate::jobs::Job::spawn("Signieren", ctx, work));
    }

    pub(crate) fn cert_dialog_ui(&mut self, ctx: &egui::Context) {
        let Some(dlg) = &mut self.cert_dialog else {
            return;
        };
        let mut open = true;
        let mut create = false;
        egui::Window::new("Testzertifikat erstellen")
            .open(&mut open)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Erstellt eine digitale ID mit selbst ausgestelltem Zertifikat. Andere \
                         Programme zeigen damit signierte Dokumente als „Identität unbekannt“, \
                         bis man dem Zertifikat dort vertraut. Für amtliche oder geschäftliche \
                         Signaturen brauchen Sie ein Zertifikat einer Zertifizierungsstelle.",
                    )
                    .weak(),
                );
                egui::Grid::new("cert_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut dlg.name);
                        ui.end_row();
                        ui.label("Organisation");
                        ui.text_edit_singleline(&mut dlg.organization);
                        ui.end_row();
                        ui.label("E-Mail");
                        ui.text_edit_singleline(&mut dlg.email);
                        ui.end_row();
                        ui.label("Gültig (Jahre)");
                        ui.add(egui::DragValue::new(&mut dlg.years).range(1..=10));
                        ui.end_row();
                        ui.label("Schlüssel");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut dlg.kind, KeyKind::Rsa2048, "RSA 2048");
                            ui.radio_value(&mut dlg.kind, KeyKind::P256, "ECDSA P-256");
                        });
                        ui.end_row();
                        ui.label("Passwort");
                        ui.add(egui::TextEdit::singleline(&mut dlg.password).password(true));
                        ui.end_row();
                        ui.label("Wiederholen");
                        ui.add(egui::TextEdit::singleline(&mut dlg.repeat).password(true));
                        ui.end_row();
                    });
                ui.checkbox(&mut dlg.trust_it, "Diesem Zertifikat in PDF Tool vertrauen");
                if let Some(e) = &dlg.error {
                    ui.colored_label(crate::signatures::RED, e);
                }
                if ui.button("Erstellen und speichern…").clicked() {
                    create = true;
                }
            });
        if !open {
            self.cert_dialog = None;
            return;
        }
        if create {
            self.create_test_certificate();
        }
    }

    fn create_test_certificate(&mut self) {
        let Some(dlg) = &mut self.cert_dialog else {
            return;
        };
        dlg.error = if dlg.name.trim().is_empty() {
            Some("Bitte einen Namen eingeben.".into())
        } else if dlg.password.len() < 4 {
            Some("Das Passwort muss mindestens 4 Zeichen haben.".into())
        } else if dlg.password != dlg.repeat {
            Some("Die Passwörter stimmen nicht überein.".into())
        } else {
            None
        };
        if dlg.error.is_some() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Digitale ID (PKCS#12)", &["p12"])
            .set_file_name(format!("{}.p12", dlg.name.trim()))
            .save_file()
        else {
            return;
        };
        // RSA-Schlüsselerzeugung dauert einen Moment (unter einer Sekunde im Release-Build).
        let result = Signer::self_signed(
            &dlg.name,
            &dlg.organization,
            &dlg.email,
            dlg.kind,
            dlg.years,
        )
        .and_then(|s| Ok((s.to_pkcs12(&dlg.password)?, s)));
        let (p12, signer) = match result {
            Ok(r) => r,
            Err(e) => {
                dlg.error = Some(e.to_string());
                return;
            }
        };
        if let Err(e) = std::fs::write(&path, p12) {
            dlg.error = Some(format!("Speichern fehlgeschlagen: {e}"));
            return;
        }
        let trust_it = dlg.trust_it;
        let password = std::mem::take(&mut dlg.password);
        self.cert_dialog = None;
        if trust_it {
            self.trust_certificate(&signer.info().der);
        }
        self.settings.signing_cert_path = Some(path.clone());
        // Steht der Signieren-Dialog offen, ist die neue ID gleich einsatzbereit.
        if let Some(sd) = &mut self.sign_dialog {
            sd.signer = Some(signer);
            sd.password = password;
            sd.load_error = None;
        }
        self.info = Some(format!(
            "Digitale ID gespeichert:\n{}\n\nBewahren Sie Datei und Passwort sicher auf.",
            path.display()
        ));
    }
}

enum DialogAction {
    Sign,
    CreateCert,
    Browse,
}

fn sign_dialog_body(
    ui: &mut egui::Ui,
    dlg: &mut SignDialog,
    settings: &mut crate::settings::Settings,
) -> Option<DialogAction> {
    let mut action = None;
    ui.label(RichText::new("Digitale ID").strong());
    ui.horizontal(|ui| {
        let name = settings
            .signing_cert_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("(keine Datei gewählt)".into(), |n| {
                n.to_string_lossy().into_owned()
            });
        ui.label(name);
        if ui.button("Datei wählen…").clicked() {
            action = Some(DialogAction::Browse);
        }
    });
    if ui
        .small_button("Noch keine? Testzertifikat erstellen…")
        .clicked()
    {
        action = Some(DialogAction::CreateCert);
    }
    match &dlg.signer {
        Some(s) => {
            let info = s.info();
            ui.colored_label(crate::signatures::GREEN, format!("Geöffnet: {}", info.name));
            ui.label(RichText::new(format!("Gültig bis {}", info.not_after)).weak());
        }
        None => {
            ui.horizontal(|ui| {
                ui.label("Passwort");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut dlg.password)
                        .password(true)
                        .desired_width(160.0),
                );
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Öffnen").clicked() || enter)
                    && let Some(path) = &settings.signing_cert_path
                {
                    load_signer(dlg, path);
                }
            });
            if let Some(e) = &dlg.load_error {
                ui.colored_label(crate::signatures::RED, e);
            }
        }
    }

    ui.separator();
    egui::Grid::new("sign_details")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Grund");
            ui.text_edit_singleline(&mut settings.sign_reason);
            ui.end_row();
            ui.label("Ort");
            ui.text_edit_singleline(&mut settings.sign_location);
            ui.end_row();
            ui.label("Kontakt");
            ui.text_edit_singleline(&mut settings.sign_contact);
            ui.end_row();
        });

    ui.checkbox(
        &mut settings.tsa_enabled,
        "Zeitstempel hinzufügen (empfohlen)",
    )
    .on_hover_text(
        "Ein Zeitstempeldienst bestätigt unabhängig, wann signiert wurde. Dabei wird nur \
             ein Hashwert übertragen, nicht das Dokument.",
    );
    if settings.tsa_enabled {
        ui.horizontal(|ui| {
            ui.label("Dienst");
            ui.add(
                egui::TextEdit::singleline(&mut settings.tsa_url)
                    .hint_text("http://…")
                    .desired_width(220.0),
            );
            ui.menu_button("Vorschläge", |ui| {
                for url in TSA_SUGGESTIONS {
                    if ui.button(url).clicked() {
                        settings.tsa_url = url.to_string();
                        ui.close();
                    }
                }
            });
        });
    }

    ui.separator();
    ui.label(RichText::new("Darstellung").strong());
    if !dlg.existing.is_empty() {
        ui.horizontal(|ui| {
            ui.radio_value(&mut dlg.mode, SignMode::Existing, "In vorhandenes Feld:");
            egui::ComboBox::from_id_salt("sig_field")
                .selected_text(
                    dlg.existing
                        .get(dlg.existing_choice)
                        .cloned()
                        .unwrap_or_default(),
                )
                .show_ui(ui, |ui| {
                    for (i, n) in dlg.existing.iter().enumerate() {
                        ui.selectable_value(&mut dlg.existing_choice, i, n);
                    }
                });
        });
    }
    ui.horizontal(|ui| {
        ui.radio_value(&mut dlg.mode, SignMode::Visible, "Sichtbar");
        if dlg.mode == SignMode::Visible {
            let label = match dlg.area {
                Some((p, _)) => format!("Bereich auf Seite {} gewählt – ändern", p + 1),
                None => "Bereich auf der Seite wählen".into(),
            };
            if ui.button(label).clicked() {
                dlg.placing = true;
            }
        }
    });
    ui.radio_value(
        &mut dlg.mode,
        SignMode::Invisible,
        "Unsichtbar (nur im Signatur-Panel)",
    );

    ui.separator();
    let ready = dlg.signer.is_some()
        && match dlg.mode {
            SignMode::Visible => dlg.area.is_some(),
            SignMode::Invisible => true,
            SignMode::Existing => !dlg.existing.is_empty(),
        };
    ui.label(
        RichText::new(
            "Die signierte Fassung wird als neue Datei gespeichert. Ungespeicherte Änderungen \
             kommen mit hinein.",
        )
        .weak(),
    );
    if ui
        .add_enabled(ready, egui::Button::new("Signieren und speichern unter…"))
        .clicked()
    {
        action = Some(DialogAction::Sign);
    }
    action
}

fn load_signer(dlg: &mut SignDialog, path: &PathBuf) {
    match std::fs::read(path) {
        Ok(data) => match Signer::from_pkcs12(&data, &dlg.password) {
            Ok(s) => {
                dlg.signer = Some(s);
                dlg.load_error = None;
            }
            Err(e) => dlg.load_error = Some(e.to_string()),
        },
        Err(e) => dlg.load_error = Some(format!("Datei nicht lesbar: {e}")),
    }
}

/// Schickt eine Zeitstempel-Anfrage per HTTP(S) an den Dienst (RFC 3161, Abschnitt 3.4).
fn http_tsa(url: &str, request: &[u8]) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent
        .post(url)
        .header("Content-Type", "application/timestamp-query")
        .send(request)
        .map_err(|e| format!("Zeitstempeldienst nicht erreichbar ({url}): {e}"))?;
    response
        .body_mut()
        .read_to_vec()
        .map_err(|e| format!("Antwort des Zeitstempeldienstes unvollständig: {e}"))
}
