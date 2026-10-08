fn main() {
    fluxvault::safety::MediaSafetyPolicy::assert_invariants();
    if let Err(error) = fluxvault::process_supervision::ensure() {
        eprintln!("FluxVault: {error}");
        std::process::exit(2);
    }
    std::process::exit(fluxvault::cli::run_from_env());
}
