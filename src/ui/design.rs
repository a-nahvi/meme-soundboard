use super::icons::{self, Icon};
use crate::{audio::ClipProgress, language::Language, storage::Appearance};
use eframe::egui::{
    self, Align, Align2, Color32, FontDefinitions, FontId, Frame, Label, Layout, Margin, Rect,
    RichText, Sense, Stroke, Ui, Vec2, Widget,
};
use shadcn_egui::{
    Palette, Theme,
    components::{
        button::{Button, ButtonSize, ButtonVariant},
        card::{Card, CardSize, card_content},
        icon::{Glyph, Icon as KitIcon, IconShape},
        kbd::{Kbd, KbdGroup},
        slider::{Slider, SliderVariant},
    },
    style::{AppEnum, AppVariant, Paint, Prop, Radius, Recipe, Role, State, Weight, pick},
    theme::heading_family_for,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
};

pub const GUTTER: i8 = 28;
pub const ROUND_CONTROL: AppVariant<ButtonSize> = ButtonSize::app("round-control");
const PLAYBACK_SLIDER: AppVariant<SliderVariant> = SliderVariant::app("playback");

pub fn set_language(ctx: &egui::Context, language: Language) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("ui-language"), language));
    ctx.request_repaint();
}

fn current_language(ctx: &egui::Context) -> Language {
    ctx.data(|data| data.get_temp(egui::Id::unique("ui-language")))
        .unwrap_or_default()
}

pub fn language(ui: &Ui) -> Language {
    current_language(ui.ctx())
}

pub fn row(ui: &Ui) -> Layout {
    language(ui).text(
        Layout::left_to_right(Align::Center),
        Layout::right_to_left(Align::Center),
    )
}

pub fn end_row(ui: &Ui) -> Layout {
    language(ui).text(
        Layout::right_to_left(Align::Center),
        Layout::left_to_right(Align::Center),
    )
}

pub fn column(ui: &Ui) -> Layout {
    Layout::top_down(language(ui).text(Align::Min, Align::Max))
}

const fn rgb(red: u8, green: u8, blue: u8) -> Color32 {
    Color32::from_rgb(red, green, blue)
}

pub fn install(ctx: &egui::Context, appearance: Appearance) -> bool {
    let mut kit = Theme::vega();
    kit.radius = 9.0;
    kit.dark = Palette {
        background: rgb(20, 28, 37),
        foreground: rgb(237, 245, 247),
        card: rgb(27, 38, 50),
        card_foreground: rgb(237, 245, 247),
        popover: rgb(31, 44, 56),
        popover_foreground: rgb(237, 245, 247),
        primary: rgb(169, 191, 255),
        primary_foreground: rgb(22, 36, 77),
        secondary: rgb(39, 53, 80),
        secondary_foreground: rgb(219, 228, 255),
        muted: rgb(33, 48, 60),
        muted_foreground: rgb(163, 184, 195),
        accent: rgb(36, 51, 75),
        accent_foreground: rgb(227, 235, 255),
        border: rgb(48, 67, 80),
        input: rgb(55, 77, 89),
        ring: rgb(153, 182, 248),
        destructive: rgb(255, 143, 150),
        destructive_foreground: rgb(58, 16, 23),
        ..Palette::dark()
    };
    kit.light = Palette {
        background: rgb(241, 247, 249),
        foreground: rgb(23, 42, 54),
        card: Color32::WHITE,
        card_foreground: rgb(23, 42, 54),
        popover: Color32::WHITE,
        popover_foreground: rgb(23, 42, 54),
        primary: rgb(48, 78, 170),
        primary_foreground: Color32::WHITE,
        secondary: rgb(229, 236, 253),
        secondary_foreground: rgb(43, 65, 127),
        muted: rgb(231, 240, 243),
        muted_foreground: rgb(75, 102, 115),
        accent: rgb(233, 239, 253),
        accent_foreground: rgb(37, 60, 122),
        border: rgb(207, 225, 231),
        input: rgb(193, 218, 224),
        ring: rgb(53, 86, 175),
        destructive: rgb(177, 39, 51),
        destructive_foreground: Color32::WHITE,
        ..Palette::light()
    };
    let style = std::sync::Arc::make_mut(&mut kit.style);
    let dialog = &mut style.dialog_content;
    dialog.ring_width.set(0.0);
    dialog.border_width.set(1.0);
    dialog.border_color.set(Paint::Role(Role::Border, 1.0));
    kit = kit.size(ROUND_CONTROL, ButtonSize::IconLg, |recipes, _| {
        recipes.button.radius.set(Radius::Px(99.0));
    });
    kit = kit.variant(PLAYBACK_SLIDER, SliderVariant::Default, |recipes, _| {
        recipes.slider_track.height = Prop::fixed(5.0);
        recipes.slider_thumb.width.set(12.0);
        recipes.slider_thumb.height.set(12.0);
        recipes.slider_thumb.bg.set(Paint::Role(Role::Primary, 1.0));
    });

    // Inter supplies the kit's typography. Windows' Segoe UI covers Persian and other scripts.
    let mut fonts = FontDefinitions::default();
    if let Some(windows) = std::env::var_os("WINDIR") {
        let file = std::path::PathBuf::from(windows)
            .join("Fonts")
            .join("segoeui.ttf");
        if let Ok(bytes) = std::fs::read(file) {
            let name = "Segoe UI fallback".to_owned();
            fonts
                .font_data
                .insert(name.clone(), egui::FontData::from_owned(bytes).into());
            for family in fonts.families.values_mut() {
                family.push(name.clone());
            }
        }
    }
    kit.apply_with_fonts(ctx, fonts);
    #[cfg(feature = "local-iconly")]
    icons::install(ctx);
    set_mode(ctx, appearance);
    let animate = system_animations_enabled();
    for mode in [egui::Theme::Dark, egui::Theme::Light] {
        let border = if mode == egui::Theme::Dark {
            kit.dark.border
        } else {
            kit.light.border
        };
        ctx.style_mut_of(mode, |style| {
            style.animation_time = if animate { 0.15 } else { 0.0 };
            style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
            style.spacing.item_spacing = Vec2::new(8.0, 8.0);
            style.spacing.scroll = egui::style::ScrollStyle {
                bar_width: 5.0,
                dormant_handle_opacity: 0.75,
                ..egui::style::ScrollStyle::solid()
            };
        });
    }
    animate
}

pub fn set_mode(ctx: &egui::Context, appearance: Appearance) {
    ctx.set_theme(match appearance {
        Appearance::System => egui::ThemePreference::System,
        Appearance::Light => egui::ThemePreference::Light,
        Appearance::Dark => egui::ThemePreference::Dark,
    });
}

fn system_animations_enabled() -> bool {
    let mut enabled: i32 = 1;
    // SPI_GETCLIENTAREAANIMATION writes a Win32 BOOL into the provided pointer.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    ok == 0 || enabled != 0
}

pub fn palette(ui: &Ui) -> Palette {
    *Theme::current(ui.ctx()).palette()
}

pub fn mic_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        rgb(116, 217, 195)
    } else {
        rgb(15, 111, 94)
    }
}

pub fn favorite_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        rgb(246, 196, 105)
    } else {
        rgb(133, 86, 14)
    }
}

pub fn title(ui: &mut Ui, label: &str) {
    let family = heading_family_for(ui.ctx(), Weight::Semibold);
    ui.label(RichText::new(label).font(FontId::new(32.0, family)));
}

pub fn muted(ui: &mut Ui, label: impl Into<String>) -> egui::Response {
    ui.label(RichText::new(label.into()).color(palette(ui).muted_foreground))
}

pub fn column_label(ui: &mut Ui, width: f32, height: f32, label: Label) -> egui::Response {
    ui.allocate_ui_with_layout(Vec2::new(width, height), row(ui), |ui| {
        ui.set_min_width(width);
        ui.add(label.truncate())
    })
    .inner
}

pub fn settings_card(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Card::new().size(CardSize::Sm).show(ui, |ui| {
        card_content(ui, |ui| {
            ui.with_layout(column(ui), content);
        });
    });
}

pub fn section_heading(ui: &mut Ui, title: &str, description: &str) {
    ui.label(RichText::new(title).strong().size(16.0));
    muted(ui, description);
    ui.add_space(4.0);
}

pub fn text_input(
    ui: &mut Ui,
    value: &mut String,
    hint: &'static str,
    width: f32,
) -> egui::Response {
    let align = language(ui).text(Align2::LEFT_CENTER, Align2::RIGHT_CENTER);
    let p = palette(ui);
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        visuals.text_edit_bg_color = Some(p.card);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.input);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.ring);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.ring);
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(width.min(ui.available_width()))
                .margin(Margin::symmetric(10, 8))
                .font(FontId::proportional(14.0))
                .hint_text(hint)
                .align(align),
        )
    })
    .inner
}

pub fn select(
    ui: &mut Ui,
    id: &str,
    selected: &mut usize,
    options: &[&str],
    width: f32,
) -> egui::Response {
    let p = palette(ui);
    let rtl = language(ui).is_rtl();
    ui.scope(|ui| {
        control_colors(ui, p.foreground, p.card, p.accent, p.input);
        ui.spacing_mut().interact_size.y = 36.0;
        let combobox = egui::ComboBox::from_id_salt(id)
            .width(width)
            .wrap_mode(egui::TextWrapMode::Truncate);
        let current = options[*selected];
        let combobox = if rtl {
            combobox.selected_text((egui::Atom::grow(), current))
        } else {
            combobox.selected_text(current)
        };
        let result = combobox.show_ui(ui, |ui| {
            ui.with_layout(column(ui), |ui| {
                options.iter().enumerate().find_map(|(index, option)| {
                    ui.selectable_label(index == *selected, *option)
                        .clicked()
                        .then_some(index)
                })
            })
            .inner
        });
        let mut response = result.response;
        if let Some(Some(index)) = result.inner {
            response.mark_changed();
            *selected = index;
        }
        response
    })
    .inner
}

fn control_colors(
    ui: &mut Ui,
    foreground: Color32,
    fill: Color32,
    hover: Color32,
    border: Color32,
) {
    ui.spacing_mut().button_padding = egui::vec2(10.0, 4.0);
    let widgets = &mut ui.visuals_mut().widgets;
    for (state, background) in [
        (&mut widgets.inactive, fill),
        (&mut widgets.hovered, hover),
        (&mut widgets.active, hover),
        (&mut widgets.open, hover),
    ] {
        state.weak_bg_fill = background;
        state.bg_fill = background;
        state.bg_stroke = Stroke::new(1.0, border);
        state.fg_stroke.color = foreground;
        state.corner_radius = 8.into();
        state.expansion = 0.0;
    }
}

pub fn dialog(
    ctx: &egui::Context,
    id: &'static str,
    title: &str,
    width: f32,
    open: &mut bool,
    content: impl FnOnce(&mut Ui),
) {
    let language = current_language(ctx);
    let p = *Theme::current(ctx).palette();
    let frame = Frame::new()
        .fill(p.popover)
        .corner_radius(12)
        .stroke(Stroke::new(1.0, p.border))
        .inner_margin(Margin::same(20))
        .shadow(ctx.global_style().visuals.popup_shadow);
    let response = egui::Modal::new(egui::Id::unique(id))
        .frame(frame)
        .show(ctx, |ui| {
            let width = width.min(ctx.content_rect().width() - 32.0) - 40.0;
            ui.set_width(width);
            ui.with_layout(column(ui), |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(title).size(18.0).strong());
                    ui.with_layout(end_row(ui), |ui| {
                        if icon_button(
                            ui,
                            language.text("Close", "بستن"),
                            Icon::CLOSE,
                            ButtonVariant::Ghost,
                        )
                        .clicked()
                        {
                            *open = false;
                        }
                    });
                });
                ui.add_space(8.0);
                content(ui);
            });
        });
    *open &= !response.should_close();
}

pub fn panel(ui: &Ui) -> Frame {
    let p = palette(ui);
    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.border))
        .corner_radius(12)
}

const ICON_SLOT: KitIcon = KitIcon {
    name: "iconly-slot",
    size: 0.0,
    shape: IconShape::Segs(&[]),
};

pub fn toolbar_button(label: &str, icon: Icon, variant: ButtonVariant) -> IconButton<'_> {
    IconButton {
        icon: Some(icon),
        ..action_button(label, variant)
    }
}

pub fn action_button(label: &str, variant: ButtonVariant) -> IconButton<'_> {
    IconButton {
        label,
        icon: None,
        variant,
        size: ButtonSize::Default,
        busy: false,
        color: None,
    }
}

pub struct IconButton<'a> {
    label: &'a str,
    icon: Option<Icon>,
    variant: ButtonVariant,
    size: ButtonSize,
    busy: bool,
    color: Option<Color32>,
}

impl IconButton<'_> {
    pub fn size(mut self, size: impl Into<ButtonSize>) -> Self {
        self.size = size.into();
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    // The kit lays out each character separately. Keep Persian words in a single shaping run.
    fn rtl_button(self, ui: &mut Ui) -> egui::Response {
        let p = palette(ui);
        let (fg, bg, hover, border) = match self.variant {
            ButtonVariant::Default => (
                p.primary_foreground,
                p.primary,
                p.primary.gamma_multiply(0.84),
                p.primary,
            ),
            ButtonVariant::Secondary => {
                (p.secondary_foreground, p.secondary, p.accent, p.secondary)
            }
            ButtonVariant::Ghost => (
                p.foreground,
                Color32::TRANSPARENT,
                p.accent,
                Color32::TRANSPARENT,
            ),
            ButtonVariant::Destructive => (
                p.destructive_foreground,
                p.destructive,
                p.destructive.gamma_multiply(0.84),
                p.destructive,
            ),
            _ => (p.foreground, p.card, p.accent, p.input),
        };
        let height = match self.size {
            ButtonSize::Lg => 40.0,
            ButtonSize::Sm => 30.0,
            _ => 36.0,
        };
        ui.scope(|ui| {
            control_colors(ui, fg, bg, hover, border);
            let text = RichText::new(self.label).size(14.0).color(fg);
            let slot = egui::IdSalt::new("soundboard-button-icon");
            let button = if self.icon.is_some() {
                egui::Button::new((text, egui::Atom::custom(slot, Vec2::splat(16.0))))
            } else {
                egui::Button::new(text)
            };
            let output = button.min_size(egui::vec2(0.0, height)).atom_ui(ui);
            if let (Some(icon), Some(rect)) = (self.icon, output.rect(slot)) {
                if self.busy {
                    egui::Spinner::new().color(fg).paint_at(ui, rect);
                } else {
                    let mut color = self.color.unwrap_or(fg);
                    if !output.enabled() {
                        color = color.gamma_multiply(0.48);
                    }
                    icons::paint_hover(ui, &output, rect.expand(1.0), icon, color);
                }
            }
            output.response
        })
        .inner
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let rtl = language(ui).is_rtl();
        let icon_only = matches!(
            self.size,
            ButtonSize::Icon
                | ButtonSize::IconXs
                | ButtonSize::IconSm
                | ButtonSize::IconLg
                | ButtonSize::App(_)
        );
        if rtl && !icon_only {
            return self.rtl_button(ui);
        }
        let button = Button::new(self.label)
            .variant(self.variant)
            .size(self.size);
        let Some(icon) = self.icon else {
            return ui.add(button);
        };
        let glyph = if self.busy {
            Glyph::Spinner
        } else {
            ICON_SLOT.into()
        };
        let response = ui.add(button.icon(glyph));
        if self.busy || !ui.is_rect_visible(response.rect) {
            return response;
        }

        // Let the kit lay out the hit target, label and icon slot, then paint the atlas in the slot.
        let theme = Theme::current(ui.ctx());
        let style = &theme.style;
        let variant = match self.variant {
            ButtonVariant::Outline => &style.button_variant_outline,
            ButtonVariant::Secondary => &style.button_variant_secondary,
            ButtonVariant::Ghost => &style.button_variant_ghost,
            ButtonVariant::Destructive => &style.button_variant_destructive,
            ButtonVariant::Link => &style.button_variant_link,
            _ => &style.button_variant_default,
        };
        let size = match self.size {
            ButtonSize::Xs => &style.button_size_xs,
            ButtonSize::Sm => &style.button_size_sm,
            ButtonSize::Lg => &style.button_size_lg,
            ButtonSize::Icon => &style.button_size_icon,
            ButtonSize::IconXs => &style.button_size_icon_xs,
            ButtonSize::IconSm => &style.button_size_icon_sm,
            ButtonSize::IconLg | ButtonSize::App(_) => &style.button_size_icon_lg,
            _ => &style.button_size_default,
        };
        let layers: [&Recipe; 3] = [&style.button, variant, size];
        let state = theme.state(
            ui.ctx(),
            self.variant.bits() | self.size.bits() | State::ICON_START,
        );
        let slot = pick(&layers, |recipe| &recipe.icon_size, state).unwrap_or(theme.metrics.icon);
        let mut center = response.rect.center();
        if !icon_only {
            let inset = pick(&layers, |recipe| &recipe.pad_left, state).unwrap_or_default();
            let border = pick(&layers, |recipe| &recipe.border_width, state).unwrap_or_default();
            center.x = response.rect.left() + inset + border + slot / 2.0;
        }
        let p = theme.palette();
        let mut color = self.color.unwrap_or(match self.variant {
            ButtonVariant::Default => p.primary_foreground,
            ButtonVariant::Secondary => p.secondary_foreground,
            ButtonVariant::Destructive => p.destructive,
            ButtonVariant::Link => p.primary,
            ButtonVariant::Outline | ButtonVariant::Ghost if response.hovered() => {
                p.accent_foreground
            }
            _ => p.foreground,
        });
        if !response.enabled() {
            color = color.gamma_multiply(0.48);
        }
        let rect = Rect::from_center_size(center, Vec2::splat(slot + 2.0));
        icons::paint_hover(ui, &response, rect, icon, color);
        response
    }
}

pub fn icon_button(ui: &mut Ui, label: &str, icon: Icon, variant: ButtonVariant) -> egui::Response {
    ui.add(toolbar_button(label, icon, variant).size(ButtonSize::IconSm))
        .on_hover_text(label)
}

pub fn icon(ui: &mut Ui, glyph: Icon, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        icons::paint(ui, rect.expand(1.0), glyph, color);
    }
}

pub fn keycaps(ui: &mut Ui, key: &str) {
    KbdGroup::new().show(ui, |ui| {
        for part in key.split('+') {
            ui.add(Kbd::new(part));
        }
    });
}

pub fn timeline(ui: &mut Ui, progress: ClipProgress, width: f32) -> Option<u32> {
    let language = language(ui);
    let time_width = if progress.duration_ms < 1000 {
        69.0
    } else {
        62.0
    };
    let track_width = (width - time_width - ui.spacing().item_spacing.x).max(1.0);
    let mut elapsed = progress.elapsed_ms as f32;
    ui.allocate_ui_with_layout(
        Vec2::new(width, 15.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            let response = ui
                .allocate_ui_with_layout(
                    Vec2::new(track_width, 15.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.add(
                            Slider::new(&mut elapsed, 0.0..=progress.duration_ms as f32)
                                .variant(PLAYBACK_SLIDER)
                                .label(language.text("Playback position", "موقعیت پخش")),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(language.text(
                            "Click or drag to seek",
                            "برای تغییر موقعیت پخش کلیک کنید یا نوار را بکشید",
                        ))
                    },
                )
                .inner;
            let shown = elapsed.round() as u32;
            let last_seek = response.id.with("last-pointer-seek");
            let seek = ui.data_mut(|data| {
                let previous = data.get_temp::<u32>(last_seek);
                let holding = response.is_pointer_button_down_on() || response.dragged();
                if holding {
                    data.insert_temp(last_seek, shown);
                } else {
                    data.remove::<u32>(last_seek);
                }
                (response.changed() && Some(shown) != previous).then_some(shown)
            });
            let p = palette(ui);
            ui.allocate_ui_with_layout(
                Vec2::new(time_width, 15.0),
                Layout::right_to_left(Align::Center),
                |ui| {
                    ui.label(
                        RichText::new(playback_time(shown, progress.duration_ms))
                            .size(11.0)
                            .color(p.muted_foreground),
                    );
                },
            );
            seek
        },
    )
    .inner
}

fn playback_time(elapsed_ms: u32, duration_ms: u32) -> String {
    if duration_ms < 1000 {
        format!("{elapsed_ms} / {duration_ms} ms")
    } else if duration_ms < 10_000 {
        format!(
            "{:.1} / {:.1} s",
            f64::from(elapsed_ms) / 1000.0,
            f64::from(duration_ms) / 1000.0
        )
    } else {
        let total = duration_ms.div_ceil(1000);
        let elapsed = if elapsed_ms == duration_ms {
            total
        } else {
            elapsed_ms / 1000
        };
        format!(
            "{}:{:02} / {}:{:02}",
            elapsed / 60,
            elapsed % 60,
            total / 60,
            total % 60
        )
    }
}

pub fn seconds_label(duration_ms: u32) -> String {
    format!("{:.3} s", f64::from(duration_ms) / 1000.0)
}

pub fn mic_meter(ui: &mut Ui, level: f32, animate: bool) {
    const BARS: usize = 14;
    const BAR_WIDTH: f32 = 4.0;
    const BAR_HEIGHT: f32 = 7.0;
    const STEP: f32 = 6.5;
    const WIDTH: f32 = BAR_WIDTH + STEP * (BARS - 1) as f32;
    let p = palette(ui);
    let signal = mic_color(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(WIDTH, BAR_HEIGHT), egui::Sense::hover());
    let fill = ((20.0 * level.max(0.0001).log10() + 58.0) / 58.0).clamp(0.0, 1.0);
    let fill = if animate {
        ui.ctx()
            .animate_value_with_time(ui.scope_id().with("mic-signal"), fill, 0.10)
    } else {
        fill
    };
    for band in 0..BARS {
        let blend = (fill * BARS as f32 - band as f32).clamp(0.0, 1.0);
        let channel =
            |from: u8, to: u8| egui::lerp(f32::from(from)..=f32::from(to), blend).round() as u8;
        let color = Color32::from_rgb(
            channel(p.border.r(), signal.r()),
            channel(p.border.g(), signal.g()),
            channel(p.border.b(), signal.b()),
        );
        let center = egui::pos2(
            rect.left() + BAR_WIDTH * 0.5 + band as f32 * STEP,
            rect.center().y,
        );
        ui.painter().rect_filled(
            egui::Rect::from_center_size(center, egui::vec2(BAR_WIDTH, BAR_HEIGHT)),
            2.0_f32,
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persian_mirrors_controls_and_keeps_mixed_text_in_reading_order() {
        let ctx = egui::Context::default();
        install(&ctx, Appearance::Light);
        set_language(&ctx, Language::Persian);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.with_layout(row(ui), |ui| {
                let first = ui.label("اول");
                let second = ui.label("دوم");
                assert!(first.rect.center().x > second.rect.center().x);
            });
            let galley = ui.painter().layout_no_wrap(
                "کلیپ MEME 12".to_owned(),
                FontId::proportional(16.0),
                Color32::WHITE,
            );
            let glyphs = &galley.rows[0].glyphs;
            let x = |character| {
                glyphs
                    .iter()
                    .find(|glyph| glyph.chr == character)
                    .unwrap()
                    .pos
                    .x
            };
            assert!(x('M') < x('1') && x('1') < x('2'));
            assert!(x('2') < x('پ') && x('پ') < x('ک'));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn playback_timeline_seeks_both_directions_with_the_mouse() {
        let context = egui::Context::default();
        install(&context, Appearance::Light);
        let mut track_origin = egui::Pos2::ZERO;
        let mut progress = ClipProgress {
            elapsed_ms: 5_000,
            duration_ms: 60_000,
            seek_revision: 0,
        };
        let draw = |events, progress, track_origin: &mut egui::Pos2| {
            let mut seek = None;
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 180.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        *track_origin = ui.cursor().min;
                        seek = timeline(ui, progress, 320.0);
                    });
                },
            );
            output.textures_delta.clear();
            seek
        };
        assert_eq!(draw(vec![], progress, &mut track_origin), None);
        let forward = track_origin + egui::vec2(200.0, 7.5);
        let backward = track_origin + egui::vec2(40.0, 7.5);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let seek = draw(
            vec![egui::Event::PointerMoved(forward), button(forward, true)],
            progress,
            &mut track_origin,
        )
        .expect("pressing the track should seek forward");
        assert!((40_000..55_000).contains(&seek));
        progress.elapsed_ms = seek;
        let seek = draw(
            vec![egui::Event::PointerMoved(backward)],
            progress,
            &mut track_origin,
        )
        .expect("dragging should seek backward");
        assert!((5_000..15_000).contains(&seek));
        progress.elapsed_ms = seek;
        assert_eq!(
            draw(vec![button(backward, false)], progress, &mut track_origin),
            None
        );
    }
}
