use super::{design, icons::Icon};
use crate::{
    releases::{self, AvailableUpdate},
    storage::Store,
};
use anyhow::Result;
use eframe::egui::{self, RichText, Ui};
use shadcn_egui::components::button::ButtonVariant;
use std::{sync::mpsc, thread};

#[derive(Default)]
pub(super) struct Updates {
    status: Status,
    pending: Option<mpsc::Receiver<Result<Status>>>,
}

#[derive(Default)]
enum Status {
    #[default]
    Idle,
    Checking,
    Current,
    Available(AvailableUpdate),
    Downloading,
    InstallerOpened,
    Error(String),
}

impl Updates {
    pub fn ui(&mut self, ui: &mut Ui, store: &Store) {
        self.receive(store);
        let language = design::language(ui);
        let version = releases::INSTALLED_VERSION;
        design::settings_card(ui, |ui| {
            design::section_heading(
                ui,
                language.text("Updates", "به‌روزرسانی"),
                &language.text(
                    format!("Installed version {version}"),
                    format!("نسخهٔ نصب‌شده {version}"),
                ),
            );
            let repository = releases::REPOSITORY;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.pending.is_none(),
                        design::toolbar_button(
                            language.text("Check for updates", "بررسی به‌روزرسانی"),
                            Icon::REFRESH,
                            ButtonVariant::Outline,
                        ),
                    )
                    .clicked()
                {
                    self.start(ui.ctx(), Status::Checking, move || {
                        Ok(match releases::check(repository)? {
                            Some(update) => Status::Available(update),
                            None => Status::Current,
                        })
                    });
                }
                if ui
                    .add(design::toolbar_button(
                        language.text("Open GitHub releases", "مشاهدهٔ نسخه‌ها در GitHub"),
                        Icon::EXTERNAL_LINK,
                        ButtonVariant::Ghost,
                    ))
                    .clicked()
                {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                        "https://github.com/{repository}/releases"
                    )));
                }
            });
            match &self.status {
                Status::Idle => {}
                Status::Checking => {
                    design::muted(
                        ui,
                        language.text("Checking GitHub…", "در حال بررسی GitHub…"),
                    );
                }
                Status::Current => {
                    design::muted(
                        ui,
                        language.text("You're up to date.", "آخرین نسخه نصب است."),
                    );
                }
                Status::Available(update) => {
                    let update = update.clone();
                    let version = &update.version;
                    ui.label(
                        RichText::new(language.text(
                            format!("Version {version} is available"),
                            format!("نسخهٔ {version} در دسترس است"),
                        ))
                        .strong(),
                    );
                    if ui
                        .add(design::toolbar_button(
                            language.text("Download and install", "دانلود و نصب"),
                            Icon::DOWNLOAD,
                            ButtonVariant::Default,
                        ))
                        .clicked()
                    {
                        let data = store.root.clone();
                        self.start(ui.ctx(), Status::Downloading, move || {
                            releases::download_and_install(repository, &update, &data)?;
                            Ok(Status::InstallerOpened)
                        });
                    }
                }
                Status::Downloading => {
                    design::muted(
                        ui,
                        language.text(
                            "Downloading and verifying the installer…",
                            "در حال دانلود و بررسی نصب‌کننده…",
                        ),
                    );
                }
                Status::InstallerOpened => {
                    design::muted(
                        ui,
                        language.text(
                            "Installer opened. Follow Setup to apply the update.",
                            "نصب‌کننده باز شد. مراحل نصب را برای به‌روزرسانی ادامه دهید.",
                        ),
                    );
                }
                Status::Error(error) => {
                    let message = language.text(
                        error.as_str(),
                        "به‌روزرسانی انجام نشد. دوباره تلاش کنید یا گزارش عیب‌یابی را ببینید.",
                    );
                    ui.label(RichText::new(message).color(design::palette(ui).destructive));
                }
            }
        });
    }

    fn start(
        &mut self,
        ctx: &egui::Context,
        status: Status,
        work: impl FnOnce() -> Result<Status> + Send + 'static,
    ) {
        let (send, receive) = mpsc::channel();
        self.status = status;
        self.pending = Some(receive);
        let wake = ctx.clone();
        thread::spawn(move || {
            let _ = send.send(work());
            wake.request_repaint();
        });
    }

    fn receive(&mut self, store: &Store) {
        let Some(pending) = &self.pending else {
            return;
        };
        if let Status::Error(error) = &self.status {
            store.log(error);
        }
        self.status = match pending.try_recv() {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => Status::Error(error.to_string()),
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Status::Error("The update stopped. Check again to retry.".into())
            }
        };
        self.pending = None;
    }
}
