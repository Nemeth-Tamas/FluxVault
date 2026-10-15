use super::*;
use crate::package::{PackageRequest, build_package};
use std::sync::atomic::{AtomicU64, Ordering};
use zip::{ZipWriter, write::SimpleFileOptions};
static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    project: ProjectState,
    package: Value,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fv-root-delivery-{}-{}",
            nonce(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let project = ProjectState::create_without_session(root.join("project")).unwrap();
        let output = root.join("archives");
        fs::create_dir(&output).unwrap();
        let directory = project.reports_dir().join("FinalReports/123-456-0");
        fs::create_dir_all(&directory).unwrap();
        let mut hashes = serde_json::Map::new();
        for n in final_report::ARTIFACTS {
            let bytes = format!("fixture report bytes: {n}");
            fs::write(directory.join(n), &bytes).unwrap();
            hashes.insert(n.to_string(), json!(hash(bytes.as_bytes())));
        }
        fs::write(
            project.reports_dir().join("FinalReportLatest.json"),
            json!({
                "schema_version":1,"generation":"123-456-0","directory":"FinalReports/123-456-0",
                "language":"hu","artifacts_sha256":hashes,"customer_delivery_certified":false
            })
            .to_string(),
        )
        .unwrap();
        fs::create_dir_all(project.root().join("Extracted/001")).unwrap();
        fs::write(
            project.root().join("Extracted/001/original.txt"),
            b"source payload",
        )
        .unwrap();
        let built = build_package(
            &PackageRequest {
                project_root: project.root().into(),
                destination: output,
                project_name: project.name().into(),
            },
            &|_| {},
        )
        .unwrap();
        let package = json!({"zip":built.zip_path,"sha256":built.sha256,"files":built.file_count,"bytes":built.total_bytes});
        Self {
            root,
            project,
            package,
        }
    }
    fn publish(&mut self) -> Result<(), String> {
        publish(&self.project, &mut self.package)
    }
    fn root_copy(&self) -> PathBuf {
        self.project.root().join(
            Path::new(self.package["zip"].as_str().unwrap())
                .file_name()
                .unwrap(),
        )
    }
    fn rewritten_zip(&mut self, mutate: impl FnOnce(&mut Vec<(String, Vec<u8>)>)) {
        let source = Path::new(self.package["zip"].as_str().unwrap());
        let mut zip = ZipArchive::new(File::open(source).unwrap()).unwrap();
        let mut files = (0..zip.len())
            .map(|i| {
                let mut member = zip.by_index(i).unwrap();
                let name = member.name().to_string();
                let mut bytes = Vec::new();
                member.read_to_end(&mut bytes).unwrap();
                (name, bytes)
            })
            .collect::<Vec<_>>();
        drop(zip);
        mutate(&mut files);
        let mut out = ZipWriter::new(File::create(source).unwrap());
        for (n, b) in files {
            out.start_file(n, SimpleFileOptions::default()).unwrap();
            out.write_all(&b).unwrap();
        }
        out.finish().unwrap().sync_all().unwrap();
        self.package["sha256"] = json!(crate::conversion::sha256_file(source).unwrap());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            if std::thread::panicking() {
                eprintln!("Delivery fixture cleanup failed: {error}");
            } else {
                panic!("Delivery fixture cleanup failed: {error}");
            }
        }
    }
}

fn inventory(root: &Path) -> std::collections::BTreeMap<PathBuf, String> {
    let mut found = std::collections::BTreeMap::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let value = if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".fluxvault-")
                    && path.extension().is_some_and(|e| e == "lock")
                {
                    assert_eq!(fs::metadata(&path).unwrap().len(), 0);
                    "zero-byte lock".to_string()
                } else {
                    crate::conversion::sha256_file(&path).unwrap()
                };
                found.insert(path.clone(), value);
            }
        }
    }
    found
}

#[test]
fn published_snapshot_verification_is_read_only_and_independent_of_external_archive() {
    let mut f = Fixture::new();
    f.publish().unwrap();
    fs::rename(
        f.package["zip"].as_str().unwrap(),
        f.root.join("stored-original.zip"),
    )
    .unwrap();
    let before = inventory(&f.root);
    for _ in 0..2 {
        let v = verify_ready(&f.project, &|_| {}).unwrap();
        assert_eq!(v["integrity_verified"], true);
        assert_eq!(v["reports_verified"], 11);
        assert_eq!(v["files_written"], 0);
        assert_eq!(v["source_archive_required"], false);
        assert_eq!(v["current_project_reprocessed"], false);
        assert_eq!(v["customer_delivery_certified"], false);
    }
    assert_eq!(inventory(&f.root), before);
}

#[test]
fn changed_or_missing_published_copies_refuse_without_any_repair_or_writes() {
    for case in ["zip", "report", "checksum", "missing"] {
        let mut f = Fixture::new();
        f.publish().unwrap();
        let root = f.project.root();
        let r = load(&root.join(READY), &root.canonicalize().unwrap()).unwrap();
        let path = match case {
            "zip" => f.root_copy(),
            "checksum" => root.join(
                r.reports
                    .iter()
                    .find(|a| a.member.is_none())
                    .unwrap()
                    .name
                    .clone(),
            ),
            _ => root.join(&r.reports[0].name),
        };
        if case == "missing" {
            fs::remove_file(&path).unwrap();
        } else {
            fs::write(&path, b"operator changed bytes").unwrap();
        }
        let before = inventory(&f.root);
        assert!(verify_ready(&f.project, &|_| {}).is_err(), "{case}");
        assert_eq!(inventory(&f.root), before, "{case}");
    }
}

#[test]
fn even_matching_ready_and_ledger_cannot_hide_an_omitted_archive_report() {
    let mut f = Fixture::new();
    f.publish().unwrap();
    let root = f.project.root().canonicalize().unwrap();
    let mut r = load(&root.join(READY), &root).unwrap();
    r.reports.remove(0);
    let bytes = serde_json::to_vec(&r).unwrap();
    fs::write(root.join(READY), &bytes).unwrap();
    fs::write(
        root.join(HISTORY)
            .join(format!("{}.json", r.archive_sha256)),
        bytes,
    )
    .unwrap();
    let before = inventory(&f.root);
    assert!(
        verify_ready(&f.project, &|_| {})
            .unwrap_err()
            .contains("inventory")
    );
    assert_eq!(inventory(&f.root), before);
}

#[test]
fn active_writer_and_midcheck_report_changes_refuse_snapshot_verification() {
    let mut f = Fixture::new();
    f.publish().unwrap();
    let owner = crate::project_work::reserve(f.project.root()).unwrap();
    let before = inventory(&f.root);
    assert!(
        verify_ready(&f.project, &|_| {})
            .unwrap_err()
            .contains("busy")
    );
    assert_eq!(inventory(&f.root), before);
    drop(owner);
    let root = f.project.root().canonicalize().unwrap();
    let r = load(&root.join(READY), &root).unwrap();
    let target = root.join(&r.reports[0].name);
    assert!(
        verify_ready(&f.project, &|s| {
            if s.contains("rechecking") {
                fs::write(&target, b"changed during verification").unwrap();
            }
        })
        .is_err()
    );
    assert_eq!(fs::read(target).unwrap(), b"changed during verification");
}

#[test]
fn verify_cli_rejects_mutating_flags_and_creates_nothing_for_unpublished_project() {
    let mut f = Fixture::new();
    let before = inventory(&f.root);
    assert!(
        crate::cli::run(
            &["package".into(), "verify".into(), "--json".into()],
            f.project.root()
        )
        .is_err()
    );
    assert_eq!(inventory(&f.root), before);
    f.publish().unwrap();
    let before = inventory(&f.root);
    let response = crate::cli::run(
        &["package".into(), "verify".into(), "--json".into()],
        f.project.root(),
    )
    .unwrap();
    assert_eq!(response.exit_code, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&response.output).unwrap()["files_written"],
        0
    );
    for flag in [
        "--allow-attention",
        "--keep-staging",
        "--no-verify",
        "--usb",
    ] {
        assert!(
            crate::cli::run(
                &["package".into(), "verify".into(), flag.into()],
                f.project.root()
            )
            .is_err(),
            "{flag}"
        );
    }
    assert_eq!(inventory(&f.root), before);
}

#[test]
fn exact_root_handoff_is_idempotent_and_excluded_from_future_archives() {
    let mut f = Fixture::new();
    let project_before = fs::read(f.project.root().join("project.json")).unwrap();
    f.publish().unwrap();
    assert_eq!(
        crate::conversion::sha256_file(&f.root_copy()).unwrap(),
        f.package["sha256"]
    );
    let pointer = load(
        &f.project.root().join(READY),
        &f.project.root().canonicalize().unwrap(),
    )
    .unwrap();
    assert!(!pointer.customer_delivery_certified);
    assert_eq!(pointer.reports.len(), 12);
    for a in &pointer.reports {
        verify(&f.project.root().join(&a.name), a.bytes, &a.sha256).unwrap();
    }
    let receipt = fs::read(f.project.root().join(READY)).unwrap();
    f.publish().unwrap();
    assert_eq!(receipt, fs::read(f.project.root().join(READY)).unwrap());
    assert_eq!(
        project_before,
        fs::read(f.project.root().join("project.json")).unwrap()
    );
    let output = f.root.join("second");
    fs::create_dir(&output).unwrap();
    let second = build_package(
        &PackageRequest {
            project_root: f.project.root().into(),
            destination: output,
            project_name: "second".into(),
        },
        &|_| {},
    )
    .unwrap();
    assert_eq!(
        second.file_count,
        f.package["files"].as_u64().unwrap() as usize
    );
    let mut archive = ZipArchive::new(File::open(second.zip_path).unwrap()).unwrap();
    for i in 0..archive.len() {
        let n = archive.by_index(i).unwrap().name().to_string();
        assert!(
            !n.contains(HISTORY)
                && !n.contains(READY)
                && !n.starts_with("FINAL_")
                && !n.ends_with(".zip")
        );
    }
}

#[test]
fn copies_come_from_archive_not_a_newer_workspace_report() {
    let mut f = Fixture::new();
    fs::write(
        f.project
            .reports_dir()
            .join("FinalReports/123-456-0/FloppyFinalAudit.txt"),
        b"new workspace text",
    )
    .unwrap();
    f.publish().unwrap();
    let root = f.project.root().canonicalize().unwrap();
    let r = load(&root.join(READY), &root).unwrap();
    let a = r
        .reports
        .iter()
        .find(|a| a.name.starts_with("FINAL_FloppyFinalAudit-") && a.name.ends_with(".txt"))
        .unwrap();
    assert_eq!(
        fs::read(root.join(&a.name)).unwrap(),
        b"fixture report bytes: FloppyFinalAudit.txt"
    );
}

#[test]
fn foreign_root_files_or_ready_pointer_are_never_adopted_or_overwritten() {
    let mut f = Fixture::new();
    fs::copy(f.package["zip"].as_str().unwrap(), f.root_copy()).unwrap();
    assert!(f.publish().unwrap_err().contains("Unowned"));
    assert!(!f.project.root().join(READY).exists());
    fs::remove_file(f.root_copy()).unwrap();
    fs::write(f.project.root().join(READY), b"operator file").unwrap();
    assert!(f.publish().is_err());
    assert_eq!(
        fs::read(f.project.root().join(READY)).unwrap(),
        b"operator file"
    );
}

#[test]
fn stopped_publication_can_replay_existing_verified_copies() {
    let mut f = Fixture::new();
    f.publish().unwrap();
    let root = f.project.root().canonicalize().unwrap();
    let r = load(&root.join(READY), &root).unwrap();
    fs::remove_file(root.join(READY)).unwrap();
    fs::remove_file(root.join(&r.reports[0].name)).unwrap();
    let retained = crate::conversion::sha256_file(&f.root_copy()).unwrap();
    f.publish().unwrap();
    assert_eq!(
        crate::conversion::sha256_file(&f.root_copy()).unwrap(),
        retained
    );
    assert_eq!(load(&root.join(READY), &root).unwrap(), r);
}

#[test]
fn edited_owned_copy_refuses_replacement_and_does_not_publish_ready() {
    let mut f = Fixture::new();
    f.publish().unwrap();
    let root = f.project.root().canonicalize().unwrap();
    let r = load(&root.join(READY), &root).unwrap();
    fs::remove_file(root.join(READY)).unwrap();
    let target = root.join(&r.reports[0].name);
    fs::write(&target, b"operator changed this").unwrap();
    assert!(f.publish().unwrap_err().contains("refusing replacement"));
    assert!(!root.join(READY).exists());
    assert_eq!(fs::read(target).unwrap(), b"operator changed this");
}

#[test]
fn changed_archive_or_invalid_archived_report_binding_refuses_before_copies() {
    let mut f = Fixture::new();
    let sha = f.package["sha256"].clone();
    f.package["sha256"] = json!("0".repeat(64));
    assert!(f.publish().unwrap_err().contains("Archive hash changed"));
    assert!(!f.root_copy().exists());
    f.package["sha256"] = sha;
    f.rewritten_zip(|files| {
        files
            .iter_mut()
            .find(|(n, _)| n.ends_with("FloppyFinalAudit.txt"))
            .unwrap()
            .1 = b"changed bytes".to_vec();
    });
    assert!(f.publish().unwrap_err().contains("report hash mismatch"));
    assert!(!f.root_copy().exists());
}

#[test]
fn duplicate_members_and_invalid_report_generation_are_refused() {
    let mut f = Fixture::new();
    f.rewritten_zip(|files| {
        let (_, bytes) = files
            .iter_mut()
            .find(|(n, _)| n == "Reports/FinalReportLatest.json")
            .unwrap();
        let mut v: Value = serde_json::from_slice(bytes).unwrap();
        v["generation"] = json!("../../escape");
        *bytes = v.to_string().into_bytes();
    });
    assert!(f.publish().unwrap_err().contains("report identity"));
    assert!(!f.root_copy().exists());
    f.rewritten_zip(|files| {
        let (n, b) = files[0].clone();
        files.push((n.to_ascii_uppercase(), b));
    });
    assert!(f.publish().unwrap_err().contains("Duplicate"));
}

#[test]
fn raw_only_archive_has_explicit_report_not_invented_office_outputs() {
    let mut f = Fixture::new();
    f.rewritten_zip(|files| {
        files.retain(|(n, _)| !n.starts_with("Reports/Final"));
        files.push((
            "Reports/RawOnlyEndpoint-123-456.json".into(),
            b"{\"sector_images\":0,\"attention\":true}".to_vec(),
        ));
    });
    f.publish().unwrap();
    assert_eq!(
        f.package["delivery"]["reports"].as_array().unwrap().len(),
        1
    );
    assert!(
        f.package["delivery"]["reports"][0]
            .as_str()
            .unwrap()
            .contains("FINAL_RawEvidence")
    );
    let before = inventory(&f.root);
    let checked = verify_ready(&f.project, &|_| {}).unwrap();
    assert_eq!(checked["reports_verified"], 1);
    assert_eq!(checked["customer_delivery_certified"], false);
    assert_eq!(checked["files_written"], 0);
    assert_eq!(inventory(&f.root), before);
}

#[test]
fn device_spellings_and_reserved_names_refuse_without_io() {
    let mut f = Fixture::new();
    for p in ["A:\\test.zip", "B:\\test.zip", "\\\\.\\PhysicalDrive0"] {
        f.package["zip"] = json!(p);
        assert!(f.publish().unwrap_err().contains("never floppy"));
    }
    for n in [
        "../escape",
        "A:foo.zip",
        "CON.zip",
        "LPT1.zip",
        "bad.",
        "bad ",
        "two/parts.zip",
    ] {
        assert!(name(n).is_err());
    }
}

#[test]
fn cancellation_refuses_before_creating_root_handoff() {
    let mut f = Fixture::new();
    let token = crate::cancellation::Token::default();
    token.request();
    let _scope = crate::cancellation::enter(token);
    assert!(crate::cancellation::stopped(&f.publish().unwrap_err()));
    assert!(!f.project.root().join(HISTORY).exists());
}
