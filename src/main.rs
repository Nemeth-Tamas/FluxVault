fn main() {
    fluxvault::safety::MediaSafetyPolicy::assert_invariants();
    std::process::exit(fluxvault::cli::run_from_env());
}
