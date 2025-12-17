use egui::{Color32, FontFamily, FontId, Rounding, Stroke, Style, TextStyle, Visuals};

/// Font family for filled Phosphor icons
pub fn phosphor_fill_family() -> FontFamily {
    FontFamily::Name("phosphor-fill".into())
}

/// Font family for bold text
pub fn bold_family() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    // Embed Barlow Regular font (subset)
    fonts.font_data.insert(
        "barlow".to_owned(),
        egui::FontData::from_static(include_bytes!("../../fonts/Barlow-Regular.ttf")),
    );

    // Embed Barlow Bold font (subset)
    fonts.font_data.insert(
        "barlow-bold".to_owned(),
        egui::FontData::from_static(include_bytes!("../../fonts/Barlow-Bold.ttf")),
    );

    // Set Barlow Regular as primary proportional font
    fonts.families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "barlow".to_owned());

    // Add bold as a separate font family
    fonts.families.insert(
        FontFamily::Name("bold".into()),
        vec!["barlow-bold".into()],
    );

    // Add Phosphor Regular icons as fallback in Proportional family
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    // Add Phosphor Fill as a separate font family for filled icons
    // Include barlow as fallback so regular text still renders
    fonts.font_data.insert(
        "phosphor-fill".into(),
        egui_phosphor::Variant::Fill.font_data(),
    );
    fonts.families.insert(
        FontFamily::Name("phosphor-fill".into()),
        vec!["phosphor-fill".into(), "barlow".into()],
    );

    ctx.set_fonts(fonts);
}

pub fn setup_theme(ctx: &egui::Context) {
    let mut style = Style::default();

    // Dark visuals with blue accents
    let mut visuals = Visuals::dark();

    // Background colors - pure black
    let bg = Color32::BLACK;
    visuals.panel_fill = bg;
    visuals.window_fill = bg;
    visuals.faint_bg_color = Color32::from_rgb(20, 20, 18);
    visuals.extreme_bg_color = bg;

    // Widget colors - warm grays (R=G > B for warmth)
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(40, 40, 38);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(176, 176, 168));

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(56, 56, 52);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(200, 200, 192));

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(80, 80, 74);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(255, 255, 255));

    // Accent color for active/pressed buttons
    let accent = Color32::from_rgb(19, 152, 244);
    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    // Selection color (accent background, white text)
    visuals.selection.bg_fill = accent;
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);

    // Hyperlink color (accent)
    visuals.hyperlink_color = accent;

    // Rounded corners
    visuals.widgets.noninteractive.rounding = Rounding::same(6.0);
    visuals.widgets.inactive.rounding = Rounding::same(6.0);
    visuals.widgets.hovered.rounding = Rounding::same(6.0);
    visuals.widgets.active.rounding = Rounding::same(6.0);
    visuals.window_rounding = Rounding::same(8.0);

    style.visuals = visuals;

    // Font sizes - standardized at 14pt
    style.text_styles = [
        (TextStyle::Small, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace)),
    ]
    .into();

    // Spacing (scaled up)
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.button_padding = egui::vec2(18.0, 10.0);
    style.spacing.window_margin = egui::Margin::same(24.0);

    ctx.set_style(style);
}

pub fn day_tab_colors() -> (Color32, Color32, Color32) {
    // Returns (bg_color, border_color, accent)
    let accent = Color32::from_rgb(19, 152, 244);
    (
        Color32::from_rgb(0, 0, 0),        // bg
        Color32::from_rgb(56, 56, 52),     // border - warm gray
        accent,
    )
}

/// Returns (bg_color, text_color, secondary_text_color) for entry cards
pub fn entry_colors() -> (Color32, Color32, Color32) {
    (
        Color32::from_rgb(0, 0, 0),        // bg
        Color32::WHITE,                    // text
        Color32::from_rgb(208, 208, 200),  // secondary text - warm gray
    )
}

/// Returns (day_name_color, hours_color) for day tabs
pub fn day_tab_text_colors(is_selected: bool) -> (Color32, Color32) {
    if is_selected {
        (Color32::from_rgb(208, 208, 200), Color32::WHITE)
    } else {
        // Durations always white to stand out
        (Color32::from_rgb(112, 112, 104), Color32::WHITE)
    }
}

/// Returns (bg_color, text_color) for button-like elements to ensure consistency
pub fn button_colors() -> (Color32, Color32) {
    (
        Color32::from_rgb(56, 56, 52),       // bg - warm gray
        Color32::from_rgb(200, 200, 192),    // text - warm gray
    )
}

/// Returns (content_bg, frame_color, frame_text) for dialogs
pub fn dialog_colors() -> (Color32, Color32, Color32) {
    (
        Color32::BLACK,                      // content bg
        Color32::from_rgb(40, 40, 38),       // frame/border - warm gray
        Color32::from_rgb(176, 176, 168),    // frame text - warm gray
    )
}
