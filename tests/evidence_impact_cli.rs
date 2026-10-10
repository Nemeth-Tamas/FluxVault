//! Actual CLI damage/trace contracts, with synthetic saved bytes only.
use fluxvault::project::ProjectState;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

struct Fixture(ProjectState);
impl Fixture {
    fn new(bad: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-impact-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let f = Self(ProjectState::create_without_session(root).unwrap());
        let mut image = vec![0; 2880 * 512];
        image[11..13].copy_from_slice(&512u16.to_le_bytes());
        image[13] = 1;
        image[14..16].copy_from_slice(&1u16.to_le_bytes());
        image[16] = 2;
        image[17..19].copy_from_slice(&224u16.to_le_bytes());
        image[19..21].copy_from_slice(&2880u16.to_le_bytes());
        image[21] = 0xf0;
        image[22..24].copy_from_slice(&9u16.to_le_bytes());
        image[510..512].copy_from_slice(&[0x55, 0xaa]);
        for sector in [1, 10] {
            image[sector * 512..sector * 512 + 6]
                .copy_from_slice(&[0xf0, 0xff, 0xff, 0xff, 0x0f, 0]);
        }
        image[19 * 512..19 * 512 + 11].copy_from_slice(b"HELLO   TXT");
        image[19 * 512 + 11] = 0x20;
        image[19 * 512 + 26..19 * 512 + 28].copy_from_slice(&2u16.to_le_bytes());
        image[19 * 512 + 28..19 * 512 + 32].copy_from_slice(&5u32.to_le_bytes());
        if !bad {
            image[33 * 512..33 * 512 + 5].copy_from_slice(b"hello");
        }
        let sha = format!("{:x}", Sha256::digest(&image));
        let status = if bad { "PARTIAL" } else { "OK" };
        fs::write(f.0.images_dir().join("001_attempt_001.img"), image).unwrap();
        let log = f.0.logs_dir().join("001_attempt_001.log");
        fs::write(&log,format!("BEGIN | disk=1 | attempt=1\nGEOMETRY | cylinders=80 | heads=2 | sectors_per_track=18 | bytes_per_sector=512 | total_sectors=2880 | total_bytes=1474560\n{}END | status={status} | bytes=1474560 | sha256={sha}\n",if bad {"BAD_SECTOR | LBA=33\n"}else{""})).unwrap();
        fs::write(f.0.images_dir().join("001_attempt_001.json"),serde_json::to_vec(&json!({"disk_number":1,"attempt_number":1,"status":status,"source_backend":"synthetic-test","source_device":"none",
            "image_file":"001_attempt_001.img","log_file":log,"sha256":sha,"bytes_written":1474560,"total_sectors":2880,"bad_sector_count":usize::from(bad),
            "bad_sectors":if bad {vec![json!({"lba":33})]}else{vec![]},
            "geometry":{"cylinders":80,"heads":2,"sectors_per_track":18,"bytes_per_sector":512,"total_bytes":1474560}})).unwrap()).unwrap();
        f
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_fluxvault"))
            .args(args)
            .current_dir(self.0.root())
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(self.0.root()).unwrap();
    }
}
fn tree(root: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut rows = Vec::new();
    for e in fs::read_dir(root).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            rows.extend(tree(&e.path()))
        } else {
            rows.push((e.path(), fs::read(e.path()).unwrap()));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}
#[test]
fn impact_and_trace_have_exact_exit_codes_json_and_no_control_or_tool_writes() {
    for bad in [false, true] {
        let f = Fixture::new(bad);
        let before = tree(f.0.root());
        for args in [
            vec!["recovery", "impact", "1", "--json"],
            vec!["recovery", "trace", "1", "hello.txt", "--json"],
        ] {
            let out = f.run(&args);
            assert_eq!(
                out.status.code(),
                Some(if bad { 3 } else { 0 }),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(out.stderr.is_empty());
            let v: Value = serde_json::from_slice(&out.stdout).unwrap();
            assert_eq!(v["physical_media_access"], false);
            assert_eq!(v["files_written"], 0);
            if v["mode"] == "file_trace" {
                assert_eq!(v["file"]["data"][0]["lba"], 33);
                assert_eq!(v["file"]["data"][0]["bytes"], 5);
            }
        }
        assert_eq!(tree(f.0.root()), before);
    }
}
#[test]
fn unsupported_flags_paths_and_missing_labels_fail_without_side_effects() {
    let f = Fixture::new(false);
    let before = tree(f.0.root());
    for args in [
        vec!["recovery", "trace", "1", "../HELLO.TXT", "--json"],
        vec!["recovery", "impact", "1", "--lba", "0", "--json"],
        vec!["recovery", "impact", "1", "--attempt", "99", "--json"],
        vec!["recovery", "trace", "1", "missing.txt", "--json"],
        vec![
            "recovery",
            "trace",
            "1",
            "HELLO.TXT",
            "--include-deleted",
            "--json",
        ],
    ] {
        let out = f.run(&args);
        assert_eq!(out.status.code(), Some(2));
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(v.get("error").is_some());
    }
    assert_eq!(tree(f.0.root()), before);
}
