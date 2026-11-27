use super::{App, design};
use eframe::egui::{
    self, Align, Align2, CursorIcon, FontId, Frame, Layout, Margin, PointerButton, Pos2, Rect,
    ResizeDirection, Sense, Stroke, Ui, ViewportCommand, vec2,
};
use shadcn_egui::{components::button::ButtonVariant, icons::lucide};

impl App {
    pub(super) fn window_frame(&mut self, ui: &mut Ui) {
        let (maximized, focused) =
            ui.input(|input| (input.viewport().maximized == Some(true), input.focused));
        if !maximized {
            resize_window(ui);
        }
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
                    let (rect, caption) = ui.allocate_exact_size(
                        vec2(ui.available_width(), 28.0),
                        Sense::click_and_drag(),
                    );
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
                    if caption.double_clicked_by(PointerButton::Primary) {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                    } else if caption.drag_started_by(PointerButton::Primary) {
                        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                    }
                });
            });
        let rect = bar.response.rect;
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.border));
    }
}

fn resize_window(ui: &Ui) {
    let (position, pressed) = ui.input(|input| {
        (
            input.pointer.hover_pos(),
            input.pointer.button_pressed(PointerButton::Primary),
        )
    });
    if let Some((direction, cursor)) =
        position.and_then(|position| resize_direction(ui.max_rect(), position))
    {
        ui.ctx().set_cursor_icon(cursor);
        if pressed {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
}

fn resize_direction(rect: Rect, pointer: Pos2) -> Option<(ResizeDirection, CursorIcon)> {
    const BORDER: f32 = 5.0;
    if !rect.contains(pointer) {
        return None;
    }
    use CursorIcon as Cursor;
    use ResizeDirection as Edge;
    let inner = rect.shrink(BORDER);
    let horizontal = if pointer.x < inner.left() {
        -1
    } else if pointer.x > inner.right() {
        1
    } else {
        0
    };
    let vertical = if pointer.y < inner.top() {
        -1
    } else if pointer.y > inner.bottom() {
        1
    } else {
        0
    };
    Some(match (horizontal, vertical) {
        (-1, -1) => (Edge::NorthWest, Cursor::ResizeNwSe),
        (1, -1) => (Edge::NorthEast, Cursor::ResizeNeSw),
        (-1, 1) => (Edge::SouthWest, Cursor::ResizeNeSw),
        (1, 1) => (Edge::SouthEast, Cursor::ResizeNwSe),
        (0, -1) => (Edge::North, Cursor::ResizeVertical),
        (0, 1) => (Edge::South, Cursor::ResizeVertical),
        (-1, 0) => (Edge::West, Cursor::ResizeHorizontal),
        (1, 0) => (Edge::East, Cursor::ResizeHorizontal),
        _ => return None,
    })
}
