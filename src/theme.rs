use eframe::egui::{self, Color32};

pub struct Palette {
    pub accent: Color32,
    pub muted: Color32,
    pub surface: Color32,
    pub background: Color32,
    pub border: Color32,
    pub text: Color32,
    pub title: Color32,
    pub danger: Color32,
    pub selected: Color32,
    pub idle: Color32,
    pub idle_border: Color32,
    pub panel: Color32,
    pub panel_border: Color32,
}

pub const PALETTE: Palette = Palette {
    accent: Color32::from_rgb(45, 183, 211),
    muted: Color32::from_rgb(151, 164, 183),
    surface: Color32::from_rgb(24, 30, 41),
    background: Color32::from_rgb(14, 18, 26),
    border: Color32::from_rgb(42, 51, 66),
    text: Color32::from_rgb(232, 238, 246),
    title: Color32::from_rgb(218, 226, 238),
    danger: Color32::from_rgb(159, 69, 76),
    selected: Color32::from_rgb(31, 112, 137),
    idle: Color32::from_rgb(34, 41, 54),
    idle_border: Color32::from_rgb(54, 64, 80),
    panel: Color32::from_rgb(19, 25, 35),
    panel_border: Color32::from_rgb(36, 45, 59),
};

/// 啟動時套用一次深色主題。
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    style.visuals.dark_mode = true;
    style.visuals.panel_fill = PALETTE.background;
    style.visuals.window_fill = PALETTE.surface;
    style.visuals.override_text_color = Some(PALETTE.text);
    style.visuals.selection.bg_fill = PALETTE.accent;
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(39, 47, 61);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(53, 65, 83);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(37, 151, 178);
    style.spacing.item_spacing = egui::vec2(12.0, 12.0);

    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}
