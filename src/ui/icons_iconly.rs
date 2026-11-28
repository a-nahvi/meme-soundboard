use eframe::egui::{
    self, Color32, Context, Id, Rect, Response, TextureHandle, TextureOptions, Ui, Vec2, pos2,
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Icon {
    start: usize,
    frames: usize,
}

impl Icon {
    const fn new(start: usize, frames: usize) -> Self {
        Self { start, frames }
    }
}

include!("../../assets/iconly/frames.rs");

#[derive(Clone)]
struct Atlas {
    texture: TextureHandle,
    size: Vec2,
}

#[derive(Clone, Copy)]
struct Hover {
    icon: Icon,
    since: f64,
}

const ATLAS: &str = "iconly-atlas";

pub(super) fn install(ctx: &Context) {
    let png = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/iconly/atlas.png"))
        .expect("The embedded Iconly atlas is a PNG");
    let size = [png.width as usize, png.height as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, &png.rgba);
    let texture = ctx.load_texture("Iconly", pixels, TextureOptions::LINEAR);
    ctx.data_mut(|data| {
        data.insert_temp(
            Id::unique(ATLAS),
            Atlas {
                texture,
                size: egui::vec2(png.width as f32, png.height as f32),
            },
        );
    });
}

pub(super) fn paint(ui: &Ui, rect: Rect, icon: Icon, color: Color32) {
    paint_frame(ui, rect, icon.start, color);
}

pub(super) fn paint_hover(ui: &Ui, response: &Response, rect: Rect, icon: Icon, color: Color32) {
    if ui.style().animation_time == 0.0 {
        paint(ui, rect, icon, color);
        return;
    }

    let ctx = ui.ctx();
    let id = response.id.with("iconly-hover");
    let hovered = response.hovered()
        && response.enabled()
        && ui.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|pos| response.interact_rect.contains(pos))
        });
    if icon.frames == 1 {
        let progress = ctx.animate_bool_with_time(id, hovered, 0.15);
        paint(ui, rect.expand(progress * 0.65), icon, color);
        return;
    }

    if !hovered {
        ctx.data_mut(|data| data.remove::<Hover>(id));
        paint(ui, rect, icon, color);
        return;
    }

    let now = ui.input(|input| input.time);
    let since = ctx.data_mut(|data| {
        let state = data.get_temp_mut_or_insert_with(id, || Hover { icon, since: now });
        if state.icon != icon {
            *state = Hover { icon, since: now };
        }
        state.since
    });
    let frame = ((now - since) * 1000.0 / FRAME_MS as f64) as usize;
    if frame + 1 < icon.frames {
        ctx.request_repaint_after(Duration::from_millis(FRAME_MS as u64));
    }
    paint_frame(ui, rect, icon.start + frame.min(icon.frames - 1), color);
}

fn paint_frame(ui: &Ui, rect: Rect, frame: usize, color: Color32) {
    let Some(atlas) = ui
        .ctx()
        .data(|data| data.get_temp::<Atlas>(Id::unique(ATLAS)))
    else {
        return;
    };
    let x = (frame % TILES_PER_ROW * TILE_STRIDE + TILE_PADDING) as f32;
    let y = (frame / TILES_PER_ROW * TILE_STRIDE + TILE_PADDING) as f32;
    let uv = Rect::from_min_max(
        pos2(x / atlas.size.x, y / atlas.size.y),
        pos2(
            (x + TILE_SIDE as f32) / atlas.size.x,
            (y + TILE_SIDE as f32) / atlas.size.y,
        ),
    );
    ui.painter().image(atlas.texture.id(), rect, uv, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_advances_on_hover_and_returns_to_rest() {
        let ctx = Context::default();
        install(&ctx);
        ctx.all_styles_mut(|style| style.animation_time = 0.15);
        let texture = ctx
            .data(|data| data.get_temp::<Atlas>(Id::unique(ATLAS)))
            .unwrap()
            .texture
            .id();
        let draw = |time, events, target: &mut Rect| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(100.0))),
                    events,
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
                    *target = rect;
                    paint_hover(ui, &response, rect, Icon::PLAY, Color32::WHITE);
                },
            );
            output.textures_delta.clear();
            output
                .shapes
                .into_iter()
                .find_map(|shape| match shape.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == texture => {
                        Some(mesh.vertices[0].uv)
                    }
                    _ => None,
                })
                .unwrap()
        };
        let mut target = Rect::NOTHING;
        let rest = draw(0.0, vec![], &mut target);
        let enter = egui::Event::PointerMoved(target.center());
        assert_eq!(draw(0.1, vec![enter.clone()], &mut target), rest);
        assert_ne!(draw(0.3, vec![], &mut target), rest);
        let end = draw(2.0, vec![], &mut target);
        assert_eq!(draw(3.0, vec![], &mut target), end);
        assert_eq!(draw(3.1, vec![egui::Event::PointerGone], &mut target), rest);
        assert_eq!(draw(3.2, vec![enter], &mut target), rest);

        ctx.all_styles_mut(|style| style.animation_time = 0.0);
        assert_eq!(draw(3.4, vec![], &mut target), rest);
    }
}
