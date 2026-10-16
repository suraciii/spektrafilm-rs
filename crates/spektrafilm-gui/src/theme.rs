//! Dark warm-accent theme for the SpektraFilm GUI.
//!
//! Applied once at startup via [`apply`]. All colours derive from the
//! prototype palette so the visual identity stays consistent.
//!
//! This module is the single token set: spacing, corner radius, type roles,
//! and colour roles. A module references a role, never an appearance.

use egui::{Color32, CornerRadius, FontId, Rgba, Shadow, Stroke, TextStyle, Visuals};

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
/// Error red.
pub const ERROR: Color32 = Color32::from_rgb(0xe0, 0x55, 0x55);
/// Warning amber, for capability notices that are not errors.
pub const WARNING: Color32 = Color32::from_rgb(0xe0, 0xa0, 0x55);

/// Chart series red, used by the tonal histogram's first band.
pub const SERIES1: Color32 = Color32::from_rgb(0xe0, 0x55, 0x55);
/// Chart series green, used by the tonal histogram's second band.
pub const SERIES2: Color32 = Color32::from_rgb(0x6a, 0xbf, 0x69);
/// Chart series blue, used by the tonal histogram's third band.
pub const SERIES3: Color32 = Color32::from_rgb(0x6a, 0x9a, 0xd4);

// ---------------------------------------------------------------------------
// Spacing
// ---------------------------------------------------------------------------

/// Gap between a section header and the section body.
pub const SPACE_SECTION: f32 = 4.0;
/// Gap between two parameter rows inside one section.
pub const SPACE_ROW: f32 = 2.0;
/// Gap between the page navigation and the search box.
pub const SPACE_BLOCK: f32 = 4.0;
/// Horizontal inset of a panel or a card.
pub const INSET_PANEL: i8 = 8;
/// Vertical inset of a panel or a card.
pub const INSET_PANEL_V: i8 = 4;
/// Gap between two action groups inside one region.
pub const SPACE_GROUP: f32 = 8.0;
/// Height reserved below the scrollable controls for the action bar.
pub const HEIGHT_PANEL_RESERVE: f32 = 95.0;
/// Height of the tonal histogram strip.
pub const HEIGHT_CHART_STRIP: f32 = 80.0;
/// Inset of the compare overlay's divider label.
pub const SPACE_OVERLAY_LABEL: f32 = 8.0;

// ---------------------------------------------------------------------------
// Corner radius
// ---------------------------------------------------------------------------

/// Corner radius of a window or a large panel.
pub const RADIUS_WINDOW: u8 = 8;
/// Corner radius of a card, a chip, or a switch track.
pub const RADIUS_CARD: u8 = 6;
/// Corner radius of a menu or a popup.
pub const RADIUS_MENU: u8 = 6;

// ---------------------------------------------------------------------------
// Type roles
// ---------------------------------------------------------------------------

/// Row label: the parameter's name.
pub const TEXT_LABEL: TextStyle = TextStyle::Button;
/// Value shown in a row's editor, at the parameter's declared decimals.
///
/// Monospace so equal-width digits align down the value column. egui 0.31
/// bundles Hack, whose digits share one advance width, so fixed decimals in
/// this style are tabular by construction.
pub const TEXT_VALUE: TextStyle = TextStyle::Monospace;
/// Hint or legend text beside a row or a section.
pub const TEXT_HINT: TextStyle = TextStyle::Small;
/// Section title in a section header.
pub const TEXT_SECTION: TextStyle = TextStyle::Button;
/// Metadata readout in the viewer footer and the histogram.
pub const TEXT_READOUT: TextStyle = TextStyle::Small;

/// Point size of the hint and readout role.
pub const SIZE_HINT: f32 = 9.0;
/// Point size of a value in a parameter row, at the parameter's declared
/// decimals. Monospace so the value column aligns.
pub const SIZE_VALUE: f32 = 12.0;
/// Point size of the preset-card name.
pub const SIZE_CARD_TITLE: f32 = 11.0;
/// Point size of the preset-card route metadata.
pub const SIZE_CARD_META: f32 = 9.0;
/// Point size of the histogram axis labels.
pub const SIZE_CHART_AXIS: f32 = 10.0;
/// Point size of the viewer's empty-state message.
pub const SIZE_VIEWER_MESSAGE: f32 = 16.0;
/// Point size of the viewer's pixel-inspection readout.
pub const SIZE_VIEWER_READOUT: f32 = 12.0;

/// Font id for a role's point size, keeping the role's family.
pub fn text(role: TextStyle, size: f32) -> FontId {
    FontId::new(size, font_family(role))
}

fn font_family(role: TextStyle) -> egui::FontFamily {
    match role {
        TextStyle::Monospace => egui::FontFamily::Monospace,
        _ => egui::FontFamily::Proportional,
    }
}

/// Width of the value column in a parameter row.
pub const WIDTH_VALUE: f32 = 72.0;
/// Fixed width of the label column, so every row on a page aligns to one
/// label column and one value column.
pub const WIDTH_LABEL: f32 = 150.0;
/// Width of the right control panel.
pub const WIDTH_PANEL: f32 = 320.0;
/// Expanded width of the left preset browser.
pub const WIDTH_PRESET_PANEL: f32 = 180.0;
/// Collapsed width of the left preset browser.
pub const WIDTH_PRESET_PANEL_COLLAPSED: f32 = 38.0;
/// Largest width a selector or a text field may take.
pub const WIDTH_COMBO_MAX: f32 = 280.0;
/// Default size of the application window.
pub const WINDOW_DEFAULT_SIZE: [f32; 2] = [1460.0, 980.0];

/// Width of a default border stroke.
pub const STROKE_DEFAULT: f32 = 1.0;
/// Width of the invalid-state ring around a value field.
pub const STROKE_INVALID: f32 = 1.5;
/// Width of the compare divider.
pub const STROKE_DIVIDER: f32 = 2.0;
/// How far the invalid ring expands past the field it marks.
pub const INVALID_RING_EXPAND: f32 = 1.5;

/// Text colour that stays legible on the accent fill.
pub const CONTRAST_ON_ACCENT: Color32 = Color32::BLACK;
/// Shadow under a window or a large panel.
pub const SHADOW_WINDOW: Shadow = Shadow {
    offset: [0, 4],
    blur: 16,
    spread: 0,
    color: Color32::from_black_alpha(80),
};
/// Shadow under a menu or a popup.
pub const SHADOW_POPUP: Shadow = Shadow {
    offset: [0, 2],
    blur: 8,
    spread: 0,
    color: Color32::from_black_alpha(60),
};
/// Horizontal and vertical padding inside a button.
pub const BUTTON_PADDING: [f32; 2] = [6.0, 3.0];

/// A default-width stroke in `color`.
pub fn stroke(color: Color32) -> Stroke {
    Stroke::new(STROKE_DEFAULT, color)
}

/// Apply the dark warm-accent theme to an egui context.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(FG);
    visuals.panel_fill = SURFACE;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = SURFACE2;
    visuals.widgets.noninteractive.bg_fill = SURFACE2;
    visuals.widgets.noninteractive.bg_stroke = stroke(BORDER);
    visuals.widgets.noninteractive.fg_stroke = stroke(FG_SECONDARY);
    visuals.widgets.inactive.bg_fill = SURFACE3;
    visuals.widgets.inactive.bg_stroke = stroke(BORDER);
    visuals.widgets.inactive.fg_stroke = stroke(FG_SECONDARY);
    visuals.widgets.hovered.bg_fill = SURFACE3;
    visuals.widgets.hovered.bg_stroke = stroke(ACCENT_DIM);
    visuals.widgets.hovered.fg_stroke = stroke(FG);
    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = stroke(ACCENT);
    visuals.widgets.active.fg_stroke = stroke(CONTRAST_ON_ACCENT);
    visuals.widgets.open.bg_fill = SURFACE2;
    visuals.widgets.open.bg_stroke = stroke(BORDER_ACTIVE);
    visuals.selection.bg_fill = Rgba::from(ACCENT).multiply(0.25_f32).into();
    visuals.selection.stroke = stroke(ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.window_stroke = stroke(BORDER);
    visuals.window_corner_radius = CornerRadius::same(RADIUS_WINDOW);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS_MENU);
    visuals.window_shadow = SHADOW_WINDOW;
    visuals.popup_shadow = SHADOW_POPUP;
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = egui::style::HandleShape::Circle;
    visuals.code_bg_color = SURFACE3;
    visuals.warn_fg_color = WARNING;
    visuals.error_fg_color = ERROR;
    ctx.set_visuals(visuals);

    // Row density: the control panel is a dense list of parameter rows, so
    // its vertical rhythm is tighter than egui's default.
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(style.spacing.item_spacing.x, SPACE_ROW);
    style.spacing.button_padding = egui::vec2(BUTTON_PADDING[0], BUTTON_PADDING[1]);
    ctx.set_style(style);
}
