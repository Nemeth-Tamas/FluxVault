use eframe::egui;

pub const NAVIGATION_WIDTH: f32 = 220.0;
pub const BOTTOM_RESERVED_HEIGHT: f32 = 190.0;

pub fn configure_context(ctx: &egui::Context) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let mut style = (*ctx.style_of(theme)).clone();

        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.interact_size.y = 28.0;

        ctx.set_style_of(theme, style);
    }
}

pub fn page_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(egui::RichText::new(title).strong().size(24.0));

    ui.add_space(2.0);

    ui.weak(subtitle);

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(12.0);
}

pub fn section<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());

        ui.label(egui::RichText::new(title).strong().size(16.0));

        ui.add_space(8.0);

        add_contents(ui)
    })
    .inner
}

pub fn subsection_label(ui: &mut egui::Ui, text: &str) {
    ui.weak(egui::RichText::new(text).strong().size(11.0));
}
