use super::*;
use crate::imaging::handoff::tests::{original_bytes, publish_fixture};
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture(ProjectState);
impl Fixture {
    fn new(failed: Option<usize>) -> Self {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = ProjectState::create_without_session(std::env::temp_dir().join(format!(
            "fv-custody-{}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        )))
        .unwrap();
        publish_fixture(&p, 1, failed, 2);
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.root());
    }
}

#[test]
fn zero_readable_confirmation_is_custody_only_not_a_byte_match() {
    let f = Fixture::new(Some(0));
    let gw = original_bytes();
    assert!(build(&f.0, 1, 1, 1, 10, &gw, &[]).is_err());
    let proof = build_custody_only(&f.0, 1, 1, 1, 10, &gw, &[]).unwrap();
    assert!(proof.operator_custody_only);
    assert_eq!(proof.assertion, CUSTODY_ASSERTION);
    assert_eq!(proof.agreeing_sectors, 0);
    assert!(proof.conflicts.is_empty());
    verify(&f.0, &proof, &gw, &[]).unwrap();
    let mut downgraded = proof.clone();
    downgraded.operator_custody_only = false;
    assert!(verify(&f.0, &downgraded, &gw, &[]).is_err());
    let mut changed = gw.clone();
    changed[512] ^= 1;
    assert!(verify(&f.0, &proof, &changed, &[]).is_err());
    assert!(verify(&f.0, &proof, &gw, &[2]).is_err());
    assert!(build_custody_only(&f.0, 1, 1, 0, 10, &gw, &[]).is_err());
    assert!(build_custody_only(&f.0, 1, 1, 1, 10, &gw, &[2, 2]).is_err());
    // Any change to the source log invalidates the exact operator assertion.
    let log = f.0.logs_dir().join("001_attempt_001.log");
    let mut bytes = fs::read(&log).unwrap();
    bytes.extend_from_slice(b"\n");
    fs::write(log, bytes).unwrap();
    assert!(verify(&f.0, &proof, &gw, &[]).is_err());
}

#[test]
fn readable_usb_prefix_cannot_use_no_comparison_confirmation() {
    let f = Fixture::new(Some(34));
    assert!(build_custody_only(&f.0, 1, 1, 1, 10, &original_bytes(), &[]).is_err());
}

#[test]
fn no_comparison_requires_validated_fast_handoff_not_just_an_all_bad_map() {
    let f = Fixture::new(Some(0));
    let path = f.0.images_dir().join("001_attempt_001.json");
    let mut meta: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    meta.as_object_mut().unwrap().remove("usb_fast_handoff");
    fs::write(path, serde_json::to_vec(&meta).unwrap()).unwrap();
    assert!(build_custody_only(&f.0, 1, 1, 1, 10, &original_bytes(), &[]).is_err());
}

#[test]
fn minor_conflict_serialization_keeps_old_shape_and_rejects_mode_changes() {
    let f = Fixture::new(None);
    let mut gw = original_bytes();
    gw[40 * 512] ^= 1;
    let proof = build(&f.0, 1, 1, 1, 10, &gw, &[]).unwrap();
    assert_eq!(proof.agreeing_sectors, 2879);
    assert_eq!(proof.conflicts.len(), 1);
    assert!(!proof.operator_custody_only);
    let json = serde_json::to_value(&proof).unwrap();
    assert!(json.get("operator_custody_only").is_none());
    let replay = serde_json::from_value(json).unwrap();
    assert_eq!(proof, replay);
    verify(&f.0, &replay, &gw, &[]).unwrap();
    let mut changed = proof;
    changed.operator_custody_only = true;
    assert!(verify(&f.0, &changed, &gw, &[]).is_err());
}
