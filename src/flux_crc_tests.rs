use super::*;
use crate::{
    flux_recovery,
    greaseweazle::{
        BackendMode, GreaseweazleBackend, GreaseweazleCommand, GreaseweazleExecution,
        GreaseweazleProfile,
    },
};
struct Fixture(ProjectState);
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(self.0.root()).unwrap();
    }
}
struct Backend {
    lie: bool,
}
fn grid() -> String {
    let tens = (0..80)
        .map(|c| {
            if c % 10 == 0 {
                char::from_digit(c / 10, 10).unwrap()
            } else {
                ' '
            }
        })
        .collect::<String>();
    let units = (0..80)
        .map(|c| char::from_digit(c % 10, 10).unwrap())
        .collect::<String>();
    let mut text = format!("Cyl-> {tens}\nH. S: {units}\n");
    for h in 0..2 {
        for r in 0..9 {
            let cells = (0..80)
                .map(|c| if h == 0 && c == 0 && r < 2 { '.' } else { 'X' })
                .collect::<String>();
            text.push_str(&format!("{h}.{r:>2}: {cells}\n"));
        }
    }
    text.push_str("Found 2 sectors of 1440 (0%)\n");
    text
}
impl GreaseweazleBackend for Backend {
    fn mode(&self) -> BackendMode {
        BackendMode::MockNoHardware
    }
    fn execute(&mut self, command: &GreaseweazleCommand) -> Result<GreaseweazleExecution, String> {
        let stdout = match command.subcommand() {
            "info" => {
                "Host Tools: 1.23\nDevice:\n  Model: Greaseweazle V4\n  Firmware: 1.23\n".into()
            }
            "read" => {
                let mut bits = Vec::new();
                for r in 1..=2 {
                    crate::mfm_evidence::tests::sector(
                        &mut bits,
                        [0, 0, r, 2],
                        0x5a,
                        false,
                        false,
                        false,
                    );
                }
                fs::write(
                    command.arguments().last().unwrap(),
                    crate::mfm_evidence::tests::fixture(&bits, 2000, false),
                )
                .unwrap();
                String::new()
            }
            "convert" => {
                let mut bytes = vec![0; 1440 * 512];
                bytes[..1024].fill(if self.lie { 0x5b } else { 0x5a });
                fs::write(command.arguments().last().unwrap(), bytes).unwrap();
                grid()
            }
            _ => panic!("No writes or other hardware commands permitted"),
        };
        Ok(GreaseweazleExecution {
            mode: self.mode(),
            command: command.clone(),
            success: true,
            exit_code: Some(0),
            stdout,
            stderr: String::new(),
            timed_out: false,
            host_version: Some("mock-crc".into()),
            started_unix_ms: 0,
            duration_ms: 0,
        })
    }
}
fn setup(lie: bool) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "fv-crc-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let p = ProjectState::create_without_session(root).unwrap();
    let mut policy = flux_recovery::RecoveryPolicy::default();
    policy.passes.truncate(1);
    flux_recovery::recover(
        &p,
        1,
        GreaseweazleProfile::Ibm720,
        'B',
        policy,
        &mut Backend { lie },
        &|_| {},
    )
    .unwrap();
    Fixture(p)
}
#[test]
fn complete_extent_partial_map_has_two_valid_and_1438_unresolved_not_1438_guesses() {
    let f = setup(false);
    let result = inspect(&f.0, 1).unwrap();
    assert_eq!(result["validated_lbas"], json!([0, 1]));
    assert_eq!(result["unresolved_lbas"].as_array().unwrap().len(), 1438);
    assert_eq!(
        result["all_reported_good_bytes_independently_validated"],
        true
    );
    assert_eq!(result["complete_sector_image"], false);
    assert_eq!(result["project_modified"], false);
    assert_eq!(result["physical_media_access"], false);
    assert_eq!(result["host_tools_invoked"], false);
    assert!(!f.0.reports_dir().join("FluxDiagnostics").exists());
}
#[test]
fn self_consistent_vendor_map_image_hash_and_provenance_do_not_override_crc_bytes() {
    let f = setup(true);
    let result = inspect(&f.0, 1).unwrap();
    assert_eq!(result["conflicting_crc_valid_lbas"], json!([0, 1]));
    assert_eq!(
        result["all_reported_good_bytes_independently_validated"],
        false
    );
}
#[test]
fn packed_and_raw_independent_gate_agree() {
    let f = setup(false);
    let before = inspect(&f.0, 1).unwrap();
    crate::flux_archive::pack(&f.0, 1, 1, true).unwrap();
    let after = inspect(&f.0, 1).unwrap();
    assert_eq!(before, after);
}
#[test]
fn tampered_source_refuses_and_cli_rejects_hardware_or_attempt_options() {
    let f = setup(false);
    for flags in [vec!["--gw-drive", "B"], vec!["--attempt", "1"]] {
        let args: Vec<String> = [vec!["recovery", "crc-check", "1"], flags]
            .concat()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert!(crate::cli::run(&args, f.0.root()).is_err());
    }
    let p = f.0.root().join("Flux/001_attempt_001.scp");
    let old = fs::read(&p).unwrap();
    fs::write(&p, b"altered").unwrap();
    assert!(inspect(&f.0, 1).is_err());
    fs::write(p, old).unwrap();
    assert!(inspect(&f.0, 1).is_ok());
}
#[test]
fn writer_reservation_prevents_inspection_during_live_mutation() {
    let f = setup(false);
    let owner = crate::project_work::reserve(f.0.root()).unwrap();
    assert!(inspect(&f.0, 1).is_err());
    drop(owner);
    assert!(inspect(&f.0, 1).is_ok());
}
fn usb(p: &ProjectState) {
    let mut bytes = vec![0x5a; 1440 * 512];
    bytes[..512].fill(0);
    let image = p.images_dir().join("001_attempt_002.img");
    fs::write(&image, &bytes).unwrap();
    let mut metadata: Value =
        serde_json::from_slice(&fs::read(p.images_dir().join("001_attempt_001.json")).unwrap())
            .unwrap();
    metadata["source_backend"] = json!("windows-raw-sector");
    metadata["source_device"] = json!("mock read only");
    metadata["attempt_number"] = json!(2);
    metadata["image_file"] = json!(image);
    metadata["log_file"] = json!("");
    metadata["bad_sector_count"] = json!(1);
    metadata["bad_sectors"] = json!([{"lba":0,"cylinder":0,"head":0,"sector":1}]);
    metadata["sha256"] = json!(format!("{:x}", Sha256::digest(&bytes)));
    for key in [
        "flux_provenance",
        "flux_provenance_sha256",
        "flux_capture_attempts",
        "read_conflicts",
    ] {
        metadata.as_object_mut().unwrap().remove(key);
    }
    fs::write(
        p.images_dir().join("001_attempt_002.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
}
#[test]
fn strict_composite_uses_crc_valid_gw_donor_and_keeps_replayable_immutable_proof() {
    let f = setup(false);
    usb(&f.0);
    let args = vec![
        "recovery".into(),
        "crc-composite".into(),
        "1".into(),
        "--json".into(),
    ];
    let output = crate::cli::run(&args, f.0.root()).unwrap();
    assert_eq!(output.exit_code, 0);
    let v: Value = serde_json::from_str(&output.output).unwrap();
    let derived = std::path::PathBuf::from(v["derived_image"].as_str().unwrap());
    assert_eq!(fs::read(&derived).unwrap(), vec![0x5a; 1440 * 512]);
    assert_eq!(v["replacements"][0]["target_lba"], 0);
    let proof = std::path::PathBuf::from(v["crc_sidecar"].as_str().unwrap());
    let old = fs::read(&proof).unwrap();
    let again = crate::cli::run(&args, f.0.root()).unwrap();
    let v: Value = serde_json::from_str(&again.output).unwrap();
    assert_eq!(v["reused"], true);
    assert_eq!(fs::read(&proof).unwrap(), old);
    fs::write(&proof, b"altered").unwrap();
    assert!(
        crate::cli::run(&args, f.0.root())
            .unwrap_err()
            .contains("CRC proof changed")
    );
}
#[test]
fn strict_composite_refuses_vendor_good_byte_conflicts_before_output() {
    let f = setup(true);
    usb(&f.0);
    let args = vec!["recovery".into(), "crc-composite".into(), "1".into()];
    assert!(
        crate::cli::run(&args, f.0.root())
            .unwrap_err()
            .contains("CRC gate refused")
    );
    assert!(!f.0.recovery_dir().join("001").exists());
}
