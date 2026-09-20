mod app;
mod floppy;
mod imaging;
mod safety;

use app::FluxVaultApp;

fn main() -> eframe::Result<()> {
    safety::MediaSafetyPolicy::assert_invariants();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "FluxVault",
        options,
        Box::new(|cc| Ok(Box::new(FluxVaultApp::new(cc)))),
    )
}
