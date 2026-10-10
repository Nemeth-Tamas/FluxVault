//! Recoverable retirement of byte-unchanged program-owned original mirrors.
use crate::conversion::sha256_file;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mirror {
    pub path: String,
    pub clean_path: String,
    pub forensic_path: String,
    pub sha256: String,
    pub owned: bool,
}
#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    schema_version: u32,
    files: Vec<Mirror>,
}
#[derive(Serialize, Deserialize)]
struct Cleanup {
    schema_version: u32,
    old_ledger_sha256: String,
    new_ledger_sha256: String,
    obsolete: Vec<Mirror>,
}
#[derive(Serialize)]
struct Outcome {
    path: String,
    sha256: String,
    status: String,
    quarantine: Option<String>,
}

pub(crate) struct Maintenance {
    converted: PathBuf,
    recovery: PathBuf,
    reports: PathBuf,
    previous: Ledger,
}
#[derive(Debug, Default)]
pub(crate) struct ResultSummary {
    pub retired: usize,
    pub preserved: usize,
    pub reports: Vec<PathBuf>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn key(path: &str) -> String {
    path.replace('\\', "/").to_uppercase()
}
fn valid(path: &str) -> Result<PathBuf, String> {
    let path = path.replace('\\', "/");
    if path.is_empty()
        || path.split('/').any(|p| {
            p.is_empty()
                || matches!(p, "." | "..")
                || p.ends_with(['.', ' '])
                || p.chars().any(|c| c.is_control() || ":*?\"<>|".contains(c))
        })
    {
        return Err("Unsafe managed delivery relative path".into());
    }
    Ok(PathBuf::from(path))
}
fn plain(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Delivery reparse point refused".into());
        }
    }
    if metadata.file_type().is_symlink() {
        return Err("Delivery symlink refused".into());
    }
    Ok(())
}
fn directory(path: &Path, parent: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        fs::create_dir(path).map_err(|e| e.to_string())?;
    }
    plain(path)?;
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    if canonical.parent() != Some(parent) || !canonical.is_dir() {
        return Err("Delivery maintenance directory escapes project".into());
    }
    Ok(canonical)
}
pub(crate) fn safe_target(root: &Path, relative: &str) -> Result<PathBuf, String> {
    plain(root)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    plain(&root)?;
    let relative = valid(relative)?;
    let mut parent = root.clone();
    let components = relative.iter().collect::<Vec<_>>();
    for part in &components[..components.len() - 1] {
        parent = directory(&parent.join(part), &parent)?;
    }
    let path = parent.join(components.last().unwrap());
    if fs::symlink_metadata(&path).is_ok() {
        plain(&path)?;
        if !path.is_file() {
            return Err("Delivery target is not a regular file".into());
        }
    }
    Ok(path)
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    plain(path)?;
    if !path.is_file() {
        return Err("Delivery state is not a regular file".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Delivery state size ceiling exceeded".into());
    }
    Ok(bytes)
}
fn immutable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file
            .write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if read(path)? == bytes {
                Ok(())
            } else {
                Err("Delivery history changed; preserved".into())
            }
        }
        Err(e) => Err(e.to_string()),
    }
}
fn validate_ledger(ledger: &Ledger) -> Result<(), String> {
    if ledger.schema_version != 1 || ledger.files.len() > 100_000 {
        return Err("Invalid delivery ownership ledger".into());
    }
    let mut seen = BTreeSet::new();
    for file in &ledger.files {
        valid(&file.path)?;
        valid(&file.clean_path)?;
        if !seen.insert(key(&file.path))
            || file.sha256.len() != 64
            || !file.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("Invalid/duplicate delivery ownership record".into());
        }
    }
    Ok(())
}
// Retirement is a relocation of equivalent originals, never permission to drop
// payloads merely because a disk/source disappeared from the latest plan.
fn identity(f: &Mirror) -> (String, String) {
    (
        key(&f.path).split('/').next().unwrap().to_owned(),
        f.sha256.clone(),
    )
}
fn replacements(old: &Ledger, current: &Ledger) -> BTreeSet<(String, String)> {
    let counts = |ledger: &Ledger| {
        let mut counts = BTreeMap::new();
        for f in &ledger.files {
            *counts.entry(identity(f)).or_insert(0usize) += 1;
        }
        counts
    };
    let present = counts(current);
    counts(old)
        .into_iter()
        .filter_map(|(id, count)| (present.get(&id).copied().unwrap_or(0) >= count).then_some(id))
        .collect()
}
impl Maintenance {
    pub(crate) fn open(converted: &Path, reports: &Path) -> Result<Self, String> {
        plain(converted)?;
        plain(reports)?;
        let converted = converted.canonicalize().map_err(|e| e.to_string())?;
        let reports = reports.canonicalize().map_err(|e| e.to_string())?;
        plain(&converted)?;
        plain(&reports)?;
        let project = converted.parent().ok_or("Invalid delivery root")?;
        if reports.parent() != Some(project)
            || converted
                .file_name()
                .is_none_or(|n| !n.eq_ignore_ascii_case("Converted"))
            || reports
                .file_name()
                .is_none_or(|n| !n.eq_ignore_ascii_case("Reports"))
        {
            return Err("Delivery maintenance requires sibling Converted/Reports".into());
        }
        let upper = project.to_string_lossy().to_uppercase();
        if ["A:\\", "B:\\", "\\\\?\\A:\\", "\\\\?\\B:\\"]
            .iter()
            .any(|p| upper.starts_with(p))
        {
            return Err("Delivery maintenance refuses floppy output".into());
        }
        let recovery = directory(&project.join("Recovery"), project)?;
        let cursor = reports.join(".fluxvault-delivery-mirrors.json");
        let previous = if cursor.exists() {
            let bytes = read(&cursor)?;
            let history = directory(&reports.join("DeliveryMirrorHistory"), &reports)?;
            if read(&history.join(format!("{}.json", digest(&bytes))))? != bytes {
                return Err("Delivery ownership cursor/history disagree".into());
            }
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?
        } else {
            Ledger {
                schema_version: 1,
                files: vec![],
            }
        };
        validate_ledger(&previous)?;
        Ok(Self {
            converted,
            recovery,
            reports,
            previous,
        })
    }
    pub(crate) fn previous_path(&self, clean: &str, forensic: &str, hash: &str) -> Option<String> {
        self.previous
            .files
            .iter()
            .find(|f| {
                key(&f.clean_path) == key(clean) && f.forensic_path == forensic && f.sha256 == hash
            })
            .map(|f| f.path.clone())
    }
    pub(crate) fn mirror(
        &self,
        path: String,
        clean_path: String,
        forensic_path: String,
        sha256: String,
        created: bool,
    ) -> Mirror {
        let owned = created
            || self
                .previous
                .files
                .iter()
                .any(|f| f.owned && key(&f.path) == key(&path) && f.sha256 == sha256);
        Mirror {
            path,
            clean_path,
            forensic_path,
            sha256,
            owned,
        }
    }
    pub(crate) fn finish(
        &self,
        mut current: Vec<Mirror>,
        mut protected: BTreeSet<String>,
    ) -> Result<ResultSummary, String> {
        current.sort_by_key(|a| key(&a.path));
        let ledger = Ledger {
            schema_version: 1,
            files: current,
        };
        validate_ledger(&ledger)?;
        protected.extend(ledger.files.iter().map(|f| key(&f.path)));
        let bytes = serde_json::to_vec_pretty(&ledger).map_err(|e| e.to_string())?;
        let new_hash = digest(&bytes);
        let history = directory(&self.reports.join("DeliveryMirrorHistory"), &self.reports)?;
        let pending = directory(&self.reports.join("DeliveryMaintenance"), &self.reports)?;
        let replacement_ids = replacements(&self.previous, &ledger);
        for file in &ledger.files {
            if replacement_ids.contains(&identity(file))
                && sha256_file(&safe_target(&self.converted, &file.path)?)? != file.sha256
            {
                return Err("Replacement mirror changed; retirement refused".into());
            }
        }
        let obsolete = self
            .previous
            .files
            .iter()
            .filter(|f| {
                f.owned
                    && !protected.contains(&key(&f.path))
                    && replacement_ids.contains(&identity(f))
            })
            .cloned()
            .collect::<Vec<_>>();
        if !obsolete.is_empty() {
            let old_bytes = serde_json::to_vec_pretty(&self.previous).map_err(|e| e.to_string())?;
            let plan = Cleanup {
                schema_version: 1,
                old_ledger_sha256: digest(&old_bytes),
                new_ledger_sha256: new_hash.clone(),
                obsolete,
            };
            let bytes = serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?;
            immutable(
                &pending.join(format!("{}.planned.json", digest(&bytes))),
                &bytes,
            )?;
        }
        immutable(&history.join(format!("{new_hash}.json")), &bytes)?;
        // Publish ownership before retirement. Pending plans retain prior claims
        // if interruption occurs after the cursor switch or any individual move.
        let cursor = self.reports.join(".fluxvault-delivery-mirrors.json");
        if !cursor.exists() || read(&cursor)? != bytes {
            let temp = self.reports.join(format!(
                ".tmp-delivery-ledger-{}-{}",
                std::process::id(),
                crate::external_tools::current_unix_ms()
            ));
            immutable(&temp, &bytes)?;
            fs::rename(&temp, &cursor).map_err(|e| e.to_string())?;
        }
        let mut result = ResultSummary::default();
        let mut plans = fs::read_dir(&pending)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        plans.sort_by_key(|e| e.file_name());
        if plans.len() > 4096 {
            return Err("Delivery retirement journal ceiling reached".into());
        }
        for entry in plans {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".planned.json") else {
                continue;
            };
            if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid delivery retirement journal identity".into());
            }
            let completed = self.reports.join(format!("DeliveryCleanup-{id}.json"));
            let completed_history = pending.join(format!("{id}.completed.json"));
            if completed.exists() {
                if read(&completed)? != read(&completed_history)? {
                    return Err("Delivery cleanup audit changed; preserved".into());
                }
                continue;
            }
            let planned = read(&entry.path())?;
            if digest(&planned) != id {
                return Err("Delivery retirement plan changed; preserved".into());
            }
            let plan: Cleanup = serde_json::from_slice(&planned).map_err(|e| e.to_string())?;
            for hash in [&plan.old_ledger_sha256, &plan.new_ledger_sha256] {
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Invalid delivery cleanup lineage hash".into());
                }
            }
            validate_ledger(&Ledger {
                schema_version: plan.schema_version,
                files: plan.obsolete.clone(),
            })?;
            let old_bytes = read(&history.join(format!("{}.json", plan.old_ledger_sha256)))?;
            if digest(&old_bytes) != plan.old_ledger_sha256 {
                return Err("Previous delivery ownership lineage changed".into());
            }
            let old: Ledger = serde_json::from_slice(&old_bytes).map_err(|e| e.to_string())?;
            validate_ledger(&old)?;
            let claims = old
                .files
                .iter()
                .filter(|f| f.owned)
                .map(|f| (f.path.as_str(), f))
                .collect::<BTreeMap<_, _>>();
            if plan
                .obsolete
                .iter()
                .any(|f| claims.get(f.path.as_str()).is_none_or(|p| *p != f))
            {
                return Err("Retirement intent lacks prior ownership evidence".into());
            }
            let lineage = read(&history.join(format!("{}.json", plan.new_ledger_sha256)))?;
            if digest(&lineage) != plan.new_ledger_sha256 {
                return Err("Delivery retirement lineage changed".into());
            }
            let quarantine = directory(&self.recovery.join("DeliveryQuarantine"), &self.recovery)?;
            let batch = directory(&quarantine.join(id), &quarantine)?;
            let mut outcomes = Vec::new();
            let replacement_ids = replacements(&old, &ledger);
            for file in &ledger.files {
                if replacement_ids.contains(&identity(file))
                    && sha256_file(&safe_target(&self.converted, &file.path)?)? != file.sha256
                {
                    return Err("Replacement mirror changed; retirement refused".into());
                }
            }
            for file in &plan.obsolete {
                let mut outcome = Outcome {
                    path: file.path.clone(),
                    sha256: file.sha256.clone(),
                    status: String::new(),
                    quarantine: None,
                };
                if !file.owned
                    || protected.contains(&key(&file.path))
                    || !replacement_ids.contains(&identity(file))
                {
                    outcome.status = "preserved_needed_or_unowned".into();
                } else {
                    let source = safe_target(&self.converted, &file.path)?;
                    let target = safe_target(&batch, &file.path)?;
                    if source.exists() && sha256_file(&source)? != file.sha256 {
                        outcome.status = "preserved_modified".into();
                    } else if target.exists() {
                        if !source.exists() && sha256_file(&target)? == file.sha256 {
                            outcome.status = "retired".into();
                            outcome.quarantine = Some(target.to_string_lossy().into());
                        } else {
                            outcome.status = "preserved_quarantine_conflict".into();
                        }
                    } else if !source.exists() {
                        outcome.status = "preserved_missing".into();
                    } else {
                        move_new(&source, &target)?;
                        if sha256_file(&target)? != file.sha256 {
                            move_new(&target, &source)?;
                            outcome.status = "preserved_modified_during_move".into();
                        } else {
                            outcome.status = "retired".into();
                            outcome.quarantine = Some(target.to_string_lossy().into());
                        }
                    }
                }
                if outcome.status == "retired" {
                    result.retired += 1;
                } else {
                    result.preserved += 1;
                }
                outcomes.push(outcome);
            }
            let audit = serde_json::to_vec_pretty(&serde_json::json!({"schema_version":1,"plan_sha256":id,"new_ledger_sha256":plan.new_ledger_sha256,"outcomes":outcomes,"warning":"Only unchanged program-owned ORIGINAL mirrors with retained equivalent payload multiplicity are retired. Prior extracted evidence remains intact. Modified/unowned files and Office derivatives are preserved."})).map_err(|e|e.to_string())?;
            immutable(&completed_history, &audit)?;
            immutable(&completed, &audit)?;
            result.reports.push(completed);
        }
        Ok(result)
    }
}

#[cfg(windows)]
fn move_new(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        windows::Win32::Storage::FileSystem::MoveFileW(
            windows::core::PCWSTR(source.as_ptr()),
            windows::core::PCWSTR(target.as_ptr()),
        )
    }
    .map_err(|e| format!("Non-overwriting quarantine move failed: {e}"))
}
#[cfg(not(windows))]
fn move_new(source: &Path, target: &Path) -> Result<(), String> {
    fs::hard_link(source, target).map_err(|e| e.to_string())?;
    fs::remove_file(source).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "fv-mirror-{}-{}-{}",
                std::process::id(),
                crate::external_tools::current_unix_ms(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("Converted")).unwrap();
            fs::create_dir_all(root.join("Reports")).unwrap();
            Self(root)
        }
        fn open(&self) -> Maintenance {
            Maintenance::open(&self.0.join("Converted"), &self.0.join("Reports")).unwrap()
        }
        fn mirror(&self, path: &str, created: bool) -> Mirror {
            let target = safe_target(&self.0.join("Converted"), path).unwrap();
            fs::write(&target, b"payload").unwrap();
            self.open().mirror(
                path.into(),
                path.into(),
                path.into(),
                sha256_file(&target).unwrap(),
                created,
            )
        }
        fn seed(&self, files: Vec<Mirror>) {
            self.open().finish(files, BTreeSet::new()).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn unchanged_owned_rename_is_recoverable_and_idempotent() {
        let f = Fixture::new();
        f.seed(vec![f.mirror("001/alias.txt", true)]);
        let new = f.mirror("001/full name.txt", true);
        let result = f.open().finish(vec![new.clone()], BTreeSet::new()).unwrap();
        assert_eq!(result.retired, 1);
        assert!(!f.0.join("Converted/001/alias.txt").exists());
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(&result.reports[0]).unwrap()).unwrap();
        let target = report["outcomes"][0]["quarantine"].as_str().unwrap();
        assert_eq!(fs::read(target).unwrap(), b"payload");
        assert_eq!(
            f.open().finish(vec![new], BTreeSet::new()).unwrap().retired,
            0
        );
    }
    #[test]
    fn edited_and_unowned_copies_are_not_removed_or_claimed() {
        let f = Fixture::new();
        f.seed(vec![
            f.mirror("001/edited.txt", true),
            f.mirror("001/operator.txt", false),
        ]);
        fs::write(f.0.join("Converted/001/edited.txt"), b"operator edit").unwrap();
        let new = f.mirror("001/new.txt", true);
        let second = f.mirror("001/new2.txt", true);
        let result = f.open().finish(vec![new, second], BTreeSet::new()).unwrap();
        assert_eq!(result.retired, 0);
        assert_eq!(result.preserved, 1);
        assert_eq!(
            fs::read(f.0.join("Converted/001/edited.txt")).unwrap(),
            b"operator edit"
        );
        assert_eq!(
            fs::read(f.0.join("Converted/001/operator.txt")).unwrap(),
            b"payload"
        );
        assert!(
            !f.open()
                .mirror(
                    "001/operator.txt".into(),
                    "001/operator.txt".into(),
                    "other".into(),
                    digest(b"payload"),
                    false
                )
                .owned
        );
    }
    #[test]
    fn disappeared_disk_regressed_payload_and_duplicate_loss_never_retire() {
        let f = Fixture::new();
        f.seed(vec![f.mirror("001/a", true), f.mirror("001/b", true)]);
        let only = f.mirror("001/renamed", true);
        assert_eq!(
            f.open()
                .finish(vec![only], BTreeSet::new())
                .unwrap()
                .retired,
            0
        );
        assert!(f.0.join("Converted/001/a").is_file());
        assert!(f.0.join("Converted/001/b").is_file());
        assert_eq!(f.open().finish(vec![], BTreeSet::new()).unwrap().retired, 0);
        assert!(f.0.join("Converted/001/renamed").is_file());
    }
    #[test]
    fn derivative_and_current_paths_are_protected_and_collision_mapping_survives() {
        let f = Fixture::new();
        f.seed(vec![f.mirror("001/document.pdf", true)]);
        let mut new = f.mirror("001/document [recovered copy 2].pdf", true);
        new.clean_path = "001/document.pdf".into();
        new.forensic_path = "document.pdf".into();
        let result = f
            .open()
            .finish(vec![new.clone()], BTreeSet::from([key("001/document.pdf")]))
            .unwrap();
        assert_eq!(result.retired, 0);
        assert!(f.0.join("Converted/001/document.pdf").is_file());
        assert_eq!(
            f.open()
                .previous_path(&new.clean_path, &new.forensic_path, &new.sha256),
            Some(new.path.clone())
        );
        assert_eq!(
            f.open().finish(vec![new], BTreeSet::new()).unwrap().retired,
            0
        );
    }
    // Simulate the durable boundaries, not a successful run followed by replay.
    fn interrupt(f: &Fixture, old: Mirror, current: Mirror, moved: bool, conflict: bool) -> Mirror {
        f.seed(vec![old.clone()]);
        let maintenance = f.open();
        let old_bytes = serde_json::to_vec_pretty(&maintenance.previous).unwrap();
        let new_bytes = serde_json::to_vec_pretty(&Ledger {
            schema_version: 1,
            files: vec![current.clone()],
        })
        .unwrap();
        let new_hash = digest(&new_bytes);
        let plan = serde_json::to_vec_pretty(&Cleanup {
            schema_version: 1,
            old_ledger_sha256: digest(&old_bytes),
            new_ledger_sha256: new_hash.clone(),
            obsolete: vec![old.clone()],
        })
        .unwrap();
        let id = digest(&plan);
        fs::write(
            f.0.join("Reports/DeliveryMaintenance")
                .join(format!("{id}.planned.json")),
            plan,
        )
        .unwrap();
        fs::write(
            f.0.join("Reports/DeliveryMirrorHistory")
                .join(format!("{new_hash}.json")),
            &new_bytes,
        )
        .unwrap();
        fs::write(
            f.0.join("Reports/.fluxvault-delivery-mirrors.json"),
            new_bytes,
        )
        .unwrap();
        if moved || conflict {
            let batch = maintenance.recovery.join("DeliveryQuarantine").join(&id);
            fs::create_dir_all(&batch).unwrap();
            let target = safe_target(&batch, &old.path).unwrap();
            if moved {
                move_new(&f.0.join("Converted").join(&old.path), &target).unwrap();
            } else {
                fs::write(target, b"quarantine operator edit").unwrap();
            }
        }
        current
    }
    #[test]
    fn restart_after_cursor_publication_finishes_pending_move_once() {
        let f = Fixture::new();
        let old = f.mirror("001/old", true);
        let new = f.mirror("001/new", true);
        let new = interrupt(&f, old, new, false, false);
        assert_eq!(
            f.open()
                .finish(vec![new.clone()], BTreeSet::new())
                .unwrap()
                .retired,
            1
        );
        assert_eq!(
            f.open().finish(vec![new], BTreeSet::new()).unwrap().retired,
            0
        );
    }
    #[test]
    fn restart_after_move_recognizes_exact_quarantine_and_conflicts_preserve_both() {
        for conflict in [false, true] {
            let f = Fixture::new();
            let old = f.mirror("001/old", true);
            let new = f.mirror("001/new", true);
            let new = interrupt(&f, old, new, !conflict, conflict);
            let result = f.open().finish(vec![new], BTreeSet::new()).unwrap();
            assert_eq!(result.retired, usize::from(!conflict));
            assert_eq!(f.0.join("Converted/001/old").exists(), conflict);
            if conflict {
                assert_eq!(result.preserved, 1);
            }
        }
    }
    #[test]
    fn stale_pending_intent_cannot_remove_a_newly_needed_copy() {
        let f = Fixture::new();
        let old = f.mirror("001/old", true);
        let new = f.mirror("001/new", true);
        interrupt(&f, old.clone(), new, false, false);
        f.open().finish(vec![old], BTreeSet::new()).unwrap();
        assert!(f.0.join("Converted/001/old").exists());
    }
    #[test]
    fn tampered_cursor_or_plan_lineage_is_refused_before_move() {
        let f = Fixture::new();
        let old = f.mirror("001/old", true);
        let new = f.mirror("001/new", true);
        let new = interrupt(&f, old, new, false, false);
        let history = f.0.join("Reports/DeliveryMirrorHistory");
        let old_hash = fs::read_dir(&history)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| fs::read_to_string(p).unwrap().contains("001/old"))
            .unwrap();
        fs::write(old_hash, b"changed").unwrap();
        assert!(
            f.open()
                .finish(vec![new], BTreeSet::new())
                .unwrap_err()
                .contains("lineage changed")
        );
        assert!(f.0.join("Converted/001/old").exists());
        fs::write(f.0.join("Reports/.fluxvault-delivery-mirrors.json"), b"{}").unwrap();
        assert!(Maintenance::open(&f.0.join("Converted"), &f.0.join("Reports")).is_err());
    }
    #[test]
    fn unsafe_paths_and_existing_move_targets_are_refused_without_overwrite() {
        let f = Fixture::new();
        for path in [
            "../outside",
            "/absolute",
            "C:/elsewhere",
            "001/../bad",
            "001/space ",
            "001/a:b",
            "001//bad",
        ] {
            assert!(safe_target(&f.0.join("Converted"), path).is_err(), "{path}");
        }
        let source = safe_target(&f.0.join("Converted"), "001/source").unwrap();
        let target = safe_target(&f.0.join("Converted"), "001/target").unwrap();
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"target").unwrap();
        assert!(move_new(&source, &target).is_err());
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(target).unwrap(), b"target");
    }
    #[test]
    fn changed_replacement_and_edited_cleanup_audit_are_refused() {
        let f = Fixture::new();
        let old = f.mirror("001/old", true);
        f.seed(vec![old]);
        let new = f.mirror("001/new", true);
        fs::write(f.0.join("Converted/001/new"), b"edit").unwrap();
        assert!(
            f.open()
                .finish(vec![new.clone()], BTreeSet::new())
                .unwrap_err()
                .contains("Replacement mirror changed")
        );
        assert!(f.0.join("Converted/001/old").is_file());
        fs::write(f.0.join("Converted/001/new"), b"payload").unwrap();
        let result = f.open().finish(vec![new.clone()], BTreeSet::new()).unwrap();
        fs::write(&result.reports[0], b"audit edit").unwrap();
        assert!(
            f.open()
                .finish(vec![new], BTreeSet::new())
                .unwrap_err()
                .contains("audit changed")
        );
        assert_eq!(fs::read(&result.reports[0]).unwrap(), b"audit edit");
    }
}
