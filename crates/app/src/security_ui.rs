//! Passwortschutz in der Oberfläche: Passwortabfrage beim Öffnen und Dialog
//! "Datei → Sicherheit…" (Status, Besitzerpasswort eingeben, Schutz setzen/entfernen).
//!
//! Passwörter stehen nur im Speicher (Dialog-Felder), nie in den Einstellungen.

use crate::app::PdfToolApp;
use eframe::egui;
use pdftool_core::CoreError;
use pdftool_core::security::{Permissions, Protection};
use std::path::PathBuf;

/// Offene Passwortabfrage für eine Datei.
pub struct PasswordPrompt {
    path: PathBuf,
    password: String,
    error: Option<String>,
    focus: bool,
}

/// Dialog "Sicherheit".
pub struct SecurityDialog {
    /// Besitzerpasswort zum Entsperren eines eingeschränkten Dokuments.
    unlock: String,
    use_user_password: bool,
    user: String,
    user_repeat: String,
    owner: String,
    owner_repeat: String,
    permissions: Permissions,
    error: Option<String>,
    /// Fokus beim ersten Bild ins Entsperren-Feld.
    focus: bool,
}

impl Default for SecurityDialog {
    fn default() -> Self {
        Self {
            unlock: String::new(),
            use_user_password: true,
            user: String::new(),
            user_repeat: String::new(),
            owner: String::new(),
            owner_repeat: String::new(),
            permissions: Permissions::ALL,
            error: None,
            focus: true,
        }
    }
}

/// Prüft die Eingaben und baut den gewünschten Schutz.
pub fn protection_from(dlg: &SecurityDialog) -> Result<Protection, String> {
    if dlg.use_user_password {
        if dlg.user.is_empty() {
            return Err("Bitte ein Passwort zum Öffnen eingeben.".into());
        }
        if dlg.user != dlg.user_repeat {
            return Err("Die Passwörter zum Öffnen stimmen nicht überein.".into());
        }
    }
    if dlg.owner.is_empty() {
        return Err("Bitte ein Besitzerpasswort eingeben.".into());
    }
    if dlg.owner != dlg.owner_repeat {
        return Err("Die Besitzerpasswörter stimmen nicht überein.".into());
    }
    if dlg.use_user_password && dlg.owner == dlg.user {
        return Err("Besitzer- und Öffnen-Passwort sollten verschieden sein.".into());
    }
    Ok(Protection {
        user_password: if dlg.use_user_password {
            dlg.user.clone()
        } else {
            String::new()
        },
        owner_password: dlg.owner.clone(),
        permissions: dlg.permissions,
    })
}

/// Passwortfeld (Eingabe verdeckt).
fn password_field(ui: &mut egui::Ui, value: &mut String) -> egui::Response {
    ui.add(
        egui::TextEdit::singleline(value)
            .password(true)
            .desired_width(200.0),
    )
}

impl PdfToolApp {
    /// Öffnen mit Passwortabfrage: Fehlt das Passwort, erscheint der Dialog.
    /// `true`, wenn der Fehler hier behandelt wurde.
    pub(crate) fn handle_open_error(&mut self, path: &std::path::Path, error: &CoreError) -> bool {
        if matches!(error, CoreError::PasswordRequired) {
            self.password_prompt = Some(PasswordPrompt {
                path: path.to_path_buf(),
                password: String::new(),
                error: None,
                focus: true,
            });
            return true;
        }
        false
    }

    pub(crate) fn password_prompt_ui(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.password_prompt else {
            return;
        };
        let mut open = true;
        let mut submit = false;
        let name = p
            .path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        egui::Window::new("Passwort erforderlich")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.label(format!("„{name}“ ist durch ein Passwort geschützt."));
                ui.horizontal(|ui| {
                    ui.label("Passwort:");
                    let r = password_field(ui, &mut p.password);
                    if std::mem::take(&mut p.focus) {
                        r.request_focus();
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit = true;
                    }
                });
                if let Some(e) = &p.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.add_space(4.0);
                if ui.button("Öffnen").clicked() {
                    submit = true;
                }
            });
        if !open {
            self.password_prompt = None;
            return;
        }
        if !submit {
            return;
        }
        let (path, password) = (p.path.clone(), p.password.clone());
        match crate::platform::open_document(&path, Some(&password)) {
            Ok(doc) => {
                self.password_prompt = None;
                self.open_in_new_tab(doc);
                self.settings.add_recent(&path);
            }
            Err(CoreError::WrongPassword) => {
                if let Some(p) = &mut self.password_prompt {
                    p.error = Some("Das Passwort ist falsch.".into());
                    p.password.clear();
                    p.focus = true;
                }
            }
            Err(e) => {
                self.password_prompt = None;
                self.error = Some(format!(
                    "„{}“ konnte nicht geöffnet werden:\n{e}",
                    path.display()
                ));
            }
        }
    }

    pub(crate) fn open_security_dialog(&mut self) {
        if self.document.is_some() {
            self.security_dialog = Some(SecurityDialog::default());
        }
    }

    pub(crate) fn security_dialog_ui(&mut self, ctx: &egui::Context) {
        let (Some(dlg), Some(d)) = (&mut self.security_dialog, &mut self.document) else {
            return;
        };
        let mut open = true;
        let mut action: Option<Option<Protection>> = None;
        egui::Window::new("Sicherheit")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                // ----- Status -----
                match d.doc.security() {
                    None => {
                        ui.label("Dieses Dokument ist nicht geschützt.");
                    }
                    Some(s) => {
                        ui.label(format!(
                            "Geschützt mit {} · {} · erlaubt: {}",
                            s.method,
                            if s.needs_password {
                                "Passwort zum Öffnen"
                            } else {
                                "ohne Passwort zu öffnen"
                            },
                            s.permissions.describe()
                        ));
                        if !s.owner_known {
                            ui.add_space(4.0);
                            ui.label("Zum Ändern von Schutz oder Inhalt (falls eingeschränkt) das Besitzerpasswort eingeben:");
                            ui.horizontal(|ui| {
                                let r = password_field(ui, &mut dlg.unlock);
                                if std::mem::take(&mut dlg.focus) {
                                    r.request_focus();
                                }
                                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if ui.button("Entsperren").clicked() || enter {
                                    if d.doc.unlock(&dlg.unlock) {
                                        dlg.error = None;
                                    } else {
                                        dlg.error = Some("Das Besitzerpasswort ist falsch.".into());
                                    }
                                    dlg.unlock.clear();
                                }
                            });
                            if let Some(e) = &dlg.error {
                                ui.colored_label(ui.visuals().error_fg_color, e);
                            }
                            return;
                        }
                    }
                }
                ui.separator();

                // ----- Neuer Schutz -----
                ui.heading("Schutz festlegen (AES-256)");
                ui.checkbox(&mut dlg.use_user_password, "Passwort zum Öffnen verlangen");
                ui.add_enabled_ui(dlg.use_user_password, |ui| {
                    egui::Grid::new("user_pw").num_columns(2).show(ui, |ui| {
                        ui.label("Passwort:");
                        password_field(ui, &mut dlg.user);
                        ui.end_row();
                        ui.label("Wiederholen:");
                        password_field(ui, &mut dlg.user_repeat);
                        ui.end_row();
                    });
                });
                ui.add_space(4.0);
                ui.label("Besitzerpasswort (für volle Rechte):");
                egui::Grid::new("owner_pw").num_columns(2).show(ui, |ui| {
                    ui.label("Passwort:");
                    password_field(ui, &mut dlg.owner);
                    ui.end_row();
                    ui.label("Wiederholen:");
                    password_field(ui, &mut dlg.owner_repeat);
                    ui.end_row();
                });
                ui.add_space(4.0);
                ui.label("Ohne Besitzerpasswort erlaubt:");
                let p = &mut dlg.permissions;
                ui.checkbox(&mut p.print, "Drucken");
                ui.checkbox(&mut p.copy, "Text und Bilder kopieren");
                ui.checkbox(&mut p.annotate, "Kommentieren und Formulare ausfüllen");
                ui.checkbox(&mut p.modify, "Inhalt und Seiten ändern");
                ui.weak("Berechtigungen sind eine Vorgabe für PDF-Programme, kein harter Schutz.");
                if let Some(e) = &dlg.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                if d.doc.has_signatures() {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Achtung: Die Datei wird beim Speichern komplett neu geschrieben – \
                         vorhandene Signaturen werden ungültig.",
                    );
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Schutz übernehmen").clicked() {
                        match protection_from(dlg) {
                            Ok(p) => action = Some(Some(p)),
                            Err(e) => dlg.error = Some(e),
                        }
                    }
                    if d.doc.security().is_some() && ui.button("Schutz entfernen").clicked() {
                        action = Some(None);
                    }
                });
                ui.weak("Wirksam beim nächsten Speichern (Strg+S).");
            });
        if !open {
            self.security_dialog = None;
            return;
        }
        if let Some(protection) = action {
            match d.doc.set_protection(protection.as_ref()) {
                Ok(()) => {
                    self.security_dialog = None;
                    self.info = Some(match protection {
                        Some(_) => {
                            "Schutz festgelegt – wirksam beim nächsten Speichern (Strg+S).".into()
                        }
                        None => {
                            "Schutz entfernt – wirksam beim nächsten Speichern (Strg+S).".into()
                        }
                    });
                }
                Err(e) => {
                    if let Some(dlg) = &mut self.security_dialog {
                        dlg.error = Some(e.to_string());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_input() {
        let mut d = SecurityDialog {
            user: "a".into(),
            user_repeat: "a".into(),
            owner: "b".into(),
            owner_repeat: "b".into(),
            ..Default::default()
        };
        assert_eq!(protection_from(&d).unwrap().user_password, "a");
        d.user_repeat = "x".into();
        assert!(protection_from(&d).is_err());
        d.use_user_password = false;
        assert_eq!(protection_from(&d).unwrap().user_password, "");
        d.owner_repeat = "c".into();
        assert!(protection_from(&d).is_err());
        d.owner_repeat = "b".into();
        d.owner.clear();
        d.owner_repeat.clear();
        assert!(protection_from(&d).is_err(), "Besitzerpasswort Pflicht");
    }
}
