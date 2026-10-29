//! Dark warm-accent theme for the SpektraFilm GUI.
//!
//! Applied once at startup via [`apply`]. All colours derive from the
//! prototype palette so the visual identity stays consistent.
//!
//! This module is the single token set: spacing, corner radius, type roles,
//! and colour roles. A module references a role, never an appearance.

use egui::{Color32, CornerRadius, FontId, Rgba, Shadow, Stroke, TextStyle, Visuals};

/// Warm gold accent used for active elements, sliders, and highlights.
pub const ACCENT: Color32 = Color32::from_rgb(0xd6, 0xb3, 0x6a);
/// Dimmed accent for hover borders and inactive indicators.
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x8d, 0x73, 0x43);
/// Deepest surface: the well behind editable text (`extreme_bg_color`).
///
/// This is not the viewer's canvas. The 18% gray/black canvas behaviour is
/// viewer state outside the palette, so no canvas role lives here.
pub const BG: Color32 = Color32::from_rgb(0x0d, 0x0e, 0x0f);
/// Panel background (control panel, viewer chrome).
pub const SURFACE: Color32 = Color32::from_rgb(0x13, 0x14, 0x15);
/// Slightly lighter surface for cards and section bodies.
pub const SURFACE2: Color32 = Color32::from_rgb(0x19, 0x1a, 0x1c);
/// Tertiary surface for hover states and nested elements.
pub const SURFACE3: Color32 = Color32::from_rgb(0x22, 0x22, 0x26);
/// Default border colour.
pub const BORDER: Color32 = Color32::from_rgb(0x2b, 0x2c, 0x2f);
/// Border colour for active/focused elements.
pub const BORDER_ACTIVE: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x44);
/// Primary text colour.
pub const FG: Color32 = Color32::from_rgb(0xf0, 0xee, 0xe9);
/// Secondary text for parameter labels and descriptions.
pub const FG_SECONDARY: Color32 = Color32::from_rgb(0xb8, 0xb5, 0xae);
/// Muted text for hints, units, and metadata. The darkest content role that
/// stays readable at `SIZE_HINT`, the panel's smallest text.
pub const FG_MUTED: Color32 = Color32::from_rgb(0x91, 0x8e, 0x87);
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

/// Spacing scale, in points. Every vertical gap in the panel is one of these
/// values, so the panel's rhythm comes from one scale rather than from a
/// literal at each use. Steps beyond the fourth are named when a use appears.
pub const SPACE_1: f32 = 4.0;
/// Second step of the spacing scale.
pub const SPACE_2: f32 = 8.0;
/// Third step of the spacing scale: the structural gap around a module or
/// sub-section header, where the panel changes subject.
pub const SPACE_3: f32 = 12.0;
/// Fourth step of the spacing scale: the structural gap between two regions
/// inside one surface.
pub const SPACE_4: f32 = 16.0;
/// Inset inside an expanded parameter module.
pub const SPACE_MODULE: f32 = SPACE_3;
/// Horizontal inset in dialog headers, bodies, and action rows.
pub const SPACE_DIALOG: f32 = SPACE_4;

/// Vertical gap between two items in the control panel.
///
/// The panel is a dense list of parameter rows, so its rhythm is the scale's
/// first step rather than egui's default.
pub const SPACE_ROW: f32 = SPACE_1;
/// Gap between the page navigation and the search box.
pub const SPACE_BLOCK: f32 = SPACE_1;
/// Height reserved below the scrollable controls for the action bar.
///
/// Includes the two-line 64 pt buttons and spacing around the separator.
/// The same reservation applies in both interface languages.
pub const HEIGHT_ACTION_BAR_RESERVE: f32 = 64.0 + SPACE_2 * 2.0;
/// Gap between two action groups inside one region.
pub const SPACE_GROUP: f32 = SPACE_2;
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

/// Value shown in a row's editor, at the parameter's declared decimals.
///
/// Monospace so equal-width digits align down the value column. egui 0.31
/// bundles Hack, whose digits share one advance width, so fixed decimals in
/// this style are tabular by construction.
pub const TEXT_VALUE: TextStyle = TextStyle::Monospace;
/// Metadata readout in the viewer footer and the histogram.
pub const TEXT_READOUT: TextStyle = TextStyle::Small;

/// Point size of the hint and readout role: the smallest text the panel
/// renders. Muted text at this size must stay readable.
pub const SIZE_HINT: f32 = 11.0;
/// Point size of a unit suffix and the experimental marker.
/// Matches the prototype's `--fs-unit`.
pub const SIZE_UNIT: f32 = 11.0;
/// Point size of a parameter row's label. Matches the prototype's `--fs-label`.
pub const SIZE_LABEL: f32 = 13.0;
/// Point size of a value in a parameter row, at the parameter's declared
/// decimals. Monospace so the value column aligns.
pub const SIZE_VALUE: f32 = 13.0;
/// Point size of a module or sub-section title. Matches the prototype's
/// `--fs-title`.
pub const SIZE_TITLE: f32 = 13.5;
/// Point size of a stage heading: the inspector's strongest text, one clear
/// step above the module titles it groups.
pub const SIZE_STAGE: f32 = 15.0;
/// Point size of the histogram axis labels.
pub const SIZE_CHART_AXIS: f32 = 11.0;
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
/// Width reserved for a unit suffix beside a numeric value.
pub const WIDTH_UNIT: f32 = 60.0;
/// Width of the right-hand control panel. The panel is fixed-width, so its
/// geometry is a layout token rather than a literal at each use.
pub const WIDTH_CONTROL_PANEL: f32 = 344.0;
/// Narrowest a choice row's selector may be, so a long label still leaves a
/// usable selector.
pub const WIDTH_COMBO_MIN: f32 = 60.0;
/// Widest a choice row's selector may be. Narrow enough that a selector
/// sharing a row with another control leaves room for it inside the control
/// panel.
pub const WIDTH_COMBO_MAX: f32 = 160.0;
/// Width of a default border stroke.
pub const STROKE_DEFAULT: f32 = 1.0;
/// Width of the invalid-state ring around a value field.
pub const STROKE_INVALID: f32 = 1.5;
/// Width of the keyboard-focus and locate ring.
///
/// The ring is an outer edge cue in [`ACCENT`], distinct from the accent fill
/// of a pressed or action control and from the tint of a selected one. It sits
/// one stroke width outside the control, so it never covers the control itself.
pub const STROKE_FOCUS: f32 = 2.0;
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
/// Shared control height and minimum photographic work area.
pub const HEIGHT_CONTROL: f32 = 28.0;
/// Window width below which the preset browser defaults to its reopen rail.
pub const WIDTH_COMPACT_WORKSPACE: f32 = 1250.0;
/// Compact global toolbar, including its outer spacing.
pub const HEIGHT_HEADER: f32 = 64.0;
/// Viewer-only tools below the photograph.
pub const HEIGHT_VIEWER_FOOTER: f32 = 50.0;
/// Stable retained-result identity strip above the photograph.
pub const HEIGHT_VIEWER_STATUS: f32 = 34.0;
/// Dialog title and close action, including their vertical spacing.
pub const HEIGHT_DIALOG_HEADER: f32 = 56.0;
/// Dialog actions, including their vertical spacing.
pub const HEIGHT_DIALOG_FOOTER: f32 = HEIGHT_CONTROL + 2.0 * SPACE_DIALOG;

/// A default-width stroke in `color`.
pub fn stroke(color: Color32) -> Stroke {
    Stroke::new(STROKE_DEFAULT, color)
}
/// Shared sheet surface; each region owns its inset and bounded height.
pub fn dialog_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(SURFACE)
        .stroke(stroke(BORDER))
        .corner_radius(RADIUS_WINDOW)
        .shadow(SHADOW_WINDOW)
}

/// Apply the dark warm-accent theme to an egui context.
///
/// Pin the theme preference to dark first: the context's default preference is
/// [`egui::ThemePreference::System`], and a light system theme resolves the
/// active style to the light one, which would discard the visuals set below
/// until the next [`egui::Context::set_theme`] call.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = Visuals::dark();
    visuals.override_text_color = Some(FG);
    visuals.panel_fill = SURFACE;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = SURFACE2;
    // `weak_bg_fill` doubles as egui's fade target (`fade_out_to_color`), the
    // colour disabled `Ui`s and weak text recede toward. Pinning it to the
    // active structure tone replaces egui's default gray-out, which halved
    // text contrast below legibility: a disabled value now lands near the
    // muted content roles on a slightly lifted plate and stays readable, while
    // panel and surface fills stay dark.
    visuals.widgets.noninteractive.weak_bg_fill = BORDER_ACTIVE;
    visuals.widgets.noninteractive.bg_fill = SURFACE2;
    visuals.widgets.noninteractive.bg_stroke = stroke(BORDER);
    visuals.widgets.noninteractive.fg_stroke = stroke(FG_SECONDARY);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS_CARD);
    // At rest a control is a quiet chip on its surface; the egui default fill
    // is a gray that reads louder than the panel.
    visuals.widgets.inactive.weak_bg_fill = SURFACE2;
    visuals.widgets.inactive.bg_fill = SURFACE3;
    visuals.widgets.inactive.bg_stroke = stroke(BORDER);
    visuals.widgets.inactive.fg_stroke = stroke(FG_SECONDARY);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_CARD);
    visuals.widgets.hovered.weak_bg_fill = SURFACE3;
    visuals.widgets.hovered.bg_fill = SURFACE3;
    visuals.widgets.hovered.bg_stroke = stroke(ACCENT_DIM);
    visuals.widgets.hovered.fg_stroke = stroke(FG);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_CARD);
    // Keyboard focus and pointer activation share one toolkit slot, and this
    // theme gives that slot the focus treatment: an outer [`ACCENT`] ring at
    // [`STROKE_FOCUS`] width, with no fill change and legible text. Focus is
    // therefore a ring cue, never the accent fill of a pressed or action
    // control and never the tint of a selected one. Controls that own an
    // action fill (an action button, a switch's on state) paint it themselves.
    visuals.widgets.active.weak_bg_fill = SURFACE3;
    visuals.widgets.active.bg_fill = SURFACE3;
    visuals.widgets.active.bg_stroke = Stroke::new(STROKE_FOCUS, ACCENT);
    visuals.widgets.active.fg_stroke = stroke(FG);
    visuals.widgets.active.corner_radius = CornerRadius::same(RADIUS_CARD);
    visuals.widgets.open.weak_bg_fill = SURFACE3;
    visuals.widgets.open.bg_fill = SURFACE2;
    visuals.widgets.open.bg_stroke = stroke(BORDER_ACTIVE);
    visuals.widgets.open.fg_stroke = stroke(FG);
    visuals.widgets.open.corner_radius = CornerRadius::same(RADIUS_CARD);
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
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(SIZE_HINT));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(SIZE_STAGE));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(SIZE_VALUE));
    style.spacing.interact_size.y = HEIGHT_CONTROL;
    style.spacing.item_spacing = egui::vec2(style.spacing.item_spacing.x, SPACE_ROW);
    style.spacing.button_padding = egui::vec2(BUTTON_PADDING[0], BUTTON_PADDING[1]);
    ctx.set_style(style);
}
