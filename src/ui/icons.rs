use eframe::egui::{self, Color32, Rect, Response, Ui, Vec2};
use shadcn_egui::{
    components::icon::{Icon as VectorIcon, paint_icon},
    icons::lucide,
};
use std::{
    f32::consts::{PI, TAU},
    time::Duration,
};

#[derive(Clone, Copy, PartialEq)]
pub(super) struct Icon {
    vector: VectorIcon,
    filled_star: bool,
}

impl Icon {
    const fn new(vector: VectorIcon) -> Self {
        Self {
            vector,
            filled_star: false,
        }
    }

    pub const LIBRARY: Self = Self::new(lucide::LIBRARY);
    pub const AUDIO_DEVICES: Self = Self::new(lucide::PLUG);
    pub const MIX: Self = Self::new(lucide::SLIDERS_HORIZONTAL);
    pub const KEYBOARD: Self = Self::new(lucide::KEYBOARD);
    pub const APPEARANCE: Self = Self::new(lucide::PALETTE);
    pub const DOWNLOAD: Self = Self::new(lucide::DOWNLOAD);
    pub const DOCUMENT: Self = Self::new(lucide::FILE_TEXT);
    pub const STOP: Self = Self::new(lucide::SQUARE);
    pub const MIC: Self = Self::new(lucide::MIC);
    pub const MIC_OFF: Self = Self::new(lucide::MIC_OFF);
    pub const WAVEFORM: Self = Self::new(lucide::AUDIO_WAVEFORM);
    pub const HEADPHONES: Self = Self::new(lucide::HEADPHONES);
    pub const WARNING: Self = Self::new(lucide::TRIANGLE_ALERT);
    pub const SUCCESS: Self = Self::new(lucide::CIRCLE_CHECK);
    pub const CLOSE: Self = Self::new(lucide::X);
    pub const PLUS: Self = Self::new(lucide::PLUS);
    pub const FOLDER: Self = Self::new(lucide::FOLDER_OPEN);
    pub const SEARCH: Self = Self::new(lucide::SEARCH);
    pub const STAR: Self = Self::new(lucide::STAR);
    pub const STAR_FILLED: Self = Self {
        filled_star: true,
        ..Self::STAR
    };
    pub const DELETE: Self = Self::new(lucide::TRASH_2);
    pub const PLAY: Self = Self::new(lucide::PLAY);
    pub const RENAME: Self = Self::new(lucide::PENCIL);
    pub const TRIM: Self = Self::new(lucide::SCISSORS);
    pub const MUSIC: Self = Self::new(lucide::MUSIC_2);
    pub const REFRESH: Self = Self::new(lucide::REFRESH_CW);
    pub const EXTERNAL_LINK: Self = Self::new(lucide::ARROW_UP_RIGHT);
    pub const CHECK: Self = Self::new(lucide::CHECK);
}

#[derive(Clone, Copy)]
struct Hover {
    icon: Icon,
    since: f64,
}

pub(super) fn paint(ui: &Ui, rect: Rect, icon: Icon, color: Color32) {
    if icon.filled_star {
        let mut mesh = egui::Mesh::default();
        let center = rect.center();
        mesh.colored_vertex(center, color);
        for point in 0..10 {
            let angle = point as f32 * TAU / 10.0 - PI / 2.0;
            let radius = rect.width() * if point % 2 == 0 { 0.43 } else { 0.205 };
            mesh.colored_vertex(center + Vec2::new(angle.cos(), angle.sin()) * radius, color);
            mesh.add_triangle(0, point + 1, (point + 1) % 10 + 1);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
    } else {
        paint_icon(ui.painter(), rect, &icon.vector, color, 1.8);
    }
}

pub(super) fn paint_hover(ui: &Ui, response: &Response, rect: Rect, icon: Icon, color: Color32) {
    const DURATION: f64 = 0.32;
    let ctx = ui.ctx();
    let id = response.id.with("icon-hover");
    let hovered = ui.style().animation_time > 0.0
        && response.hovered()
        && response.enabled()
        && ui.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|pos| response.interact_rect.contains(pos))
        });
    if !hovered {
        ctx.data_mut(|data| data.remove::<Hover>(id));
        paint(ui, rect, icon, color);
        return;
    }

    let now = ui.input(|input| input.time);
    let since = ctx.data_mut(|data| {
        let hover = data.get_temp_mut_or_insert_with(id, || Hover { icon, since: now });
        if hover.icon != icon {
            *hover = Hover { icon, since: now };
        }
        hover.since
    });
    let progress = ((now - since) / DURATION).clamp(0.0, 1.0) as f32;
    let lift = if progress < 1.0 {
        ctx.request_repaint_after(Duration::from_millis(16));
        (progress * PI).sin()
    } else {
        0.0
    };
    paint(
        ui,
        rect.expand(lift * 0.75).translate(egui::vec2(0.0, -lift)),
        icon,
        color,
    );
}
