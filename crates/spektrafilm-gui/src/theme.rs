//! Dark warm-accent theme for the SpektraFilm GUI.
//!
//! Applied once at startup via [`apply`]. All colours derive from the
//! prototype palette so the visual identity stays consistent.

use egui::{Color32, CornerRadius, Rgba, Shadow, Stroke, Visuals};

/// Warm gold accent used for active elements, sliders, and highlights.
pub const ACCENT: Color32 = Color32::from_rgb(0xd4, 0xa8, 0x53);
/// Dimmed accent for hover borders and inactive indicators.
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x8a, 0x6e, 0x35);
/// Very dark background for the main canvas area.
pub const BG: Color32 = Color32::from_rgb(0x0b, 0x0b, 0x0c);
/// Panel background (control panel, viewer chrome).
pub const SURFACE: Color32 = Color32::from_rgb(0x11, 0x11, 0x13);
/// Slightly lighter surface for cards and section bodies.
pub const SURFACE2: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x1d);
/// Tertiary surface for hover states and nested elements.
pub const SURFACE3: Color32 = Color32::from_rgb(0x22, 0x22, 0x26);
/// Default border colour.
pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2e);
/// Border colour for active/focused elements.
pub const BORDER_ACTIVE: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x44);
/// Primary text colour.
pub const FG: Color32 = Color32::from_rgb(0xe8, 0xe8, 0xe8);
/// Secondary text for labels and descriptions.
pub const FG_SECONDARY: Color32 = Color32::from_rgb(0xa0, 0xa0, 0xa8);
/// Muted text for hints and metadata.
pub const FG_MUTED: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x72);
/// Success green.
pub const SUCCESS: Color32 = Color32::from_rgb(0x6a, 0xbf, 0x69);
/// Error red.
pub const ERROR: Color32 = Color32::from_rgb(0xe0, 0x55, 0x55);
/// Info blue.
pub const INFO: Color32 = Color32::from_rgb(0x6a, 0x9a, 0xd4);

/// Apply the dark warm-accent theme to an egui context.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(FG);
    visuals.panel_fill = SURFACE;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = SURFACE2;
    visuals.widgets.noninteractive.bg_fill = SURFACE2;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, FG_SECONDARY);
    visuals.widgets.inactive.bg_fill = SURFACE3;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, FG_SECONDARY);
    visuals.widgets.hovered.bg_fill = SURFACE3;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT_DIM);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, FG);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::BLACK);
    visuals.widgets.open.bg_fill = SURFACE2;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, BORDER_ACTIVE);
    visuals.selection.bg_fill = Rgba::from(ACCENT).multiply(0.25).into();
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.window_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(80),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: Color32::from_black_alpha(60),
    };
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = egui::style::HandleShape::Circle;
    visuals.code_bg_color = SURFACE3;
    visuals.warn_fg_color = Color32::from_rgb(0xe0, 0xa0, 0x55);
    visuals.error_fg_color = ERROR;
    ctx.set_visuals(visuals);
}
