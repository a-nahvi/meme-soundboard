use super::{App, design};
use eframe::egui::{
    self, Align, Align2, FontId, Frame, Layout, Margin, Sense, Stroke, Ui, ViewportCommand, vec2,
};
use shadcn_egui::{components::button::ButtonVariant, icons::lucide};

impl App {
    pub(super) fn window_frame(&mut self, ui: &mut Ui) {
        let (maximized, focused) =
            ui.input(|input| (input.viewport().maximized == Some(true), input.focused));
        let p = design::palette(ui);
        let bar = egui::Panel::top("window-frame")
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.card)
                    .inner_margin(Margin::symmetric(6, 5)),
            )
            .show(ui, |ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let ctx = ui.ctx().clone();
                    if design::icon_button(ui, "Close", lucide::X, ButtonVariant::Ghost).clicked()
                        && !self.hide_to_tray(&ctx)
                    {
                        self.quitting = true;
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                    let (label, icon) = if maximized {
                        ("Restore", lucide::COPY)
                    } else {
                        ("Maximize", lucide::SQUARE)
                    };
                    if design::icon_button(ui, label, icon, ButtonVariant::Ghost).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                    }
                    if design::icon_button(ui, "Minimize", lucide::MINUS, ButtonVariant::Ghost)
                        .clicked()
                    {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                    }
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                    crate::window_frame::set_caption(rect, ctx.pixels_per_point());
                    ui.painter().text(
                        egui::pos2(rect.left() + 10.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        "Meme Soundboard",
                        FontId::proportional(13.0),
                        if focused {
                            p.foreground
                        } else {
                            p.muted_foreground
                        },
                    );
                });
            });
        let rect = bar.response.rect;
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.border));
    }
}
