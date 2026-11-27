use super::design;
use crate::releases::{self, AvailableUpdate};
use anyhow::Result;
use eframe::egui::{self, RichText, Ui};
use shadcn_egui::{
    components::{
        button::ButtonVariant,
        card::{Card, CardSize, card_content, card_header},
    },
    icons::lucide,
};
use std::{path::Path, sync::mpsc, thread};

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
    pub fn ui(&mut self, ui: &mut Ui, data: &Path) {
        self.receive();
        Card::new().size(CardSize::Sm).show(ui, |ui| {
            card_header(
                ui,
                "Updates",
                &format!("Installed version {}", releases::INSTALLED_VERSION),
            );
            card_content(ui, |ui| {
                let Some(repository) = releases::REPOSITORY else {
                    design::muted(ui, "Configure a GitHub repository to check for updates.");
                    return;
                };
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            self.pending.is_none(),
                            design::toolbar_button(
                                "Check for updates",
                                lucide::REFRESH_CW,
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
                            "Open GitHub releases",
                            lucide::ARROW_UP_RIGHT,
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
                        design::muted(ui, "Checking GitHub…");
                    }
                    Status::Current => {
                        design::muted(ui, "You're up to date.");
                    }
                    Status::Available(update) => {
                        let update = update.clone();
                        ui.label(
                            RichText::new(format!("Version {} is available", update.version))
                                .strong(),
                        );
                        if ui
                            .add(design::toolbar_button(
                                "Download and install",
                                lucide::DOWNLOAD,
                                ButtonVariant::Default,
                            ))
                            .clicked()
                        {
                            let data = data.to_path_buf();
                            self.start(ui.ctx(), Status::Downloading, move || {
                                releases::download_and_install(repository, &update, &data)?;
                                Ok(Status::InstallerOpened)
                            });
                        }
                    }
                    Status::Downloading => {
                        design::muted(ui, "Downloading and verifying the installer…");
                    }
                    Status::InstallerOpened => {
                        design::muted(ui, "Installer opened. Follow Setup to apply the update.");
                    }
                    Status::Error(error) => {
                        ui.label(RichText::new(error).color(design::palette(ui).destructive));
                    }
                }
            });
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

    fn receive(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
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
