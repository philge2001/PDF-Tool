//! Hintergrund-Jobs: lange Aufgaben (Teilen, später OCR, Komprimieren, Export) laufen in
//! einem eigenen Thread. Die Oberfläche zeigt Fortschritt und kann abbrechen.

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

/// Ergebnis eines Jobs: Erfolgsmeldung oder Fehlermeldung (beide für den Nutzer).
pub type JobResult = Result<String, String>;

#[derive(Default, Clone)]
struct Progress {
    fraction: f32,
    text: String,
}

/// Wird dem Job übergeben: Fortschritt melden, Abbruch abfragen.
pub struct JobCtx {
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    ui: egui::Context,
}

impl JobCtx {
    /// Meldet den Fortschritt (0.0..=1.0) mit einem kurzen Text.
    pub fn progress(&self, fraction: f32, text: impl Into<String>) {
        // `lock` kann nur scheitern, wenn ein anderer Thread beim Halten des Locks abgestürzt
        // ist ("vergiftet"); dann ist ein veralteter Fortschritt egal.
        if let Ok(mut p) = self.progress.lock() {
            p.fraction = fraction.clamp(0.0, 1.0);
            p.text = text.into();
        }
        self.ui.request_repaint();
    }

    /// Hat der Nutzer "Abbrechen" gedrückt? Jobs sollten das regelmäßig prüfen.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Ein laufender Job aus Sicht der Oberfläche.
pub struct Job {
    pub title: String,
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    result: Receiver<JobResult>,
}

impl Job {
    /// Startet `work` in einem neuen Thread.
    ///
    /// `work` muss `Send + 'static` sein: Es wandert in einen anderen Thread und darf daher
    /// nichts aus dem UI-Thread ausleihen – alle Daten werden hineinverschoben (`move`).
    pub fn spawn(
        title: impl Into<String>,
        ui: &egui::Context,
        work: impl FnOnce(&JobCtx) -> JobResult + Send + 'static,
    ) -> Self {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let ctx = JobCtx {
            progress: progress.clone(),
            cancel: cancel.clone(),
            ui: ui.clone(),
        };
        // Desktop: eigener Thread. Browser: läuft sofort (dort gibt es keine Threads).
        crate::platform::spawn(move || {
            let result = work(&ctx);
            let _ = tx.send(result);
            ctx.ui.request_repaint();
        });
        Self {
            title: title.into(),
            progress,
            cancel,
            result: rx,
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// `Some(Ergebnis)`, sobald der Job fertig ist.
    pub fn poll(&self) -> Option<JobResult> {
        match self.result.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("Der Hintergrund-Job ist abgestürzt.".into()))
            }
        }
    }

    fn snapshot(&self) -> Progress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// Modales Fortschrittsfenster.
    pub fn show(&self, ctx: &egui::Context) {
        let p = self.snapshot();
        egui::Modal::new(egui::Id::unique("job_progress")).show(ctx, |ui| {
            ui.set_min_width(320.0);
            ui.heading(&self.title);
            ui.add(egui::ProgressBar::new(p.fraction).show_percentage());
            ui.label(&p.text);
            ui.add_space(6.0);
            let cancelling = self.cancel.load(Ordering::Relaxed);
            if ui
                .add_enabled(!cancelling, egui::Button::new("Abbrechen"))
                .clicked()
            {
                self.cancel();
            }
            if cancelling {
                ui.label(egui::RichText::new("Wird abgebrochen …").weak());
            }
        });
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Zeitlimits in Desktop-Tests (nicht im Browser)
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait(job: &Job) -> JobResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(r) = job.poll() {
                return r;
            }
            assert!(Instant::now() < deadline, "Job hängt");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn job_reports_result_and_progress() {
        let ctx = egui::Context::default();
        let job = Job::spawn("Test", &ctx, |c| {
            c.progress(0.5, "Halbzeit");
            Ok("fertig".into())
        });
        assert_eq!(wait(&job), Ok("fertig".into()));
        assert_eq!(job.snapshot().text, "Halbzeit");
    }

    #[test]
    fn job_can_be_cancelled() {
        let ctx = egui::Context::default();
        let job = Job::spawn("Test", &ctx, |c| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !c.cancelled() {
                if Instant::now() > deadline {
                    return Ok("nicht abgebrochen".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err("abgebrochen".into())
        });
        job.cancel();
        assert_eq!(wait(&job), Err("abgebrochen".into()));
    }
}
