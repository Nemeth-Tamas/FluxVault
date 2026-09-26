mod audit;
mod batch_extraction;
mod cli;
mod composite;
mod conversion;
mod conversion_run;
mod dmde_logs;
mod external_tools;
mod extraction;
mod floppy;
mod greaseweazle;
mod imaging;
mod legacy_logs;
mod manifest;
mod manual_recovery_import;
mod package;
mod pipeline;
mod project;
mod recovery_backup;
mod recovery_plan;
mod report;
mod safety;
mod sector_recovery;
fn main() {
    safety::MediaSafetyPolicy::assert_invariants();
    std::process::exit(cli::run_from_env());
}
