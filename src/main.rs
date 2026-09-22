mod app;
mod composite;
mod dmde_logs;
mod external_tools;
mod extraction;
mod floppy;
mod imaging;
mod legacy_logs;
mod project;
mod recovery_backup;
mod report;
mod safety;
mod sector_recovery;
mod ui;

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
