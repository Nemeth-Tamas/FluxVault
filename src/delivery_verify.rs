//! Read-only verification of a published archival snapshot, not current recovery.
use super::*;

fn existing_lock(root: &Path, filename: &str) -> Result<Option<File>, String> {
    let path = root.join(filename);
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    checked(&path, false)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.try_lock().map_err(|e| {
        format!("Project is busy; wait for processing/publication before verifying delivery: {e}")
    })?;
    Ok(Some(file))
}

pub(crate) fn verify_ready(project: &ProjectState, stage: &impl Fn(&str)) -> Result<Value, String> {
    crate::cancellation::check()?;
    crate::processing::validate_workspace(project)?;
    let root = project.root().canonicalize().map_err(|e| e.to_string())?;
    checked(&root, true)?;
    // Never create locks, journals, output directories or a new ready pointer.
    // Existing owners follow the same owner -> artifact-snapshot lock order.
    let owner = existing_lock(&root, ".fluxvault-processing.lock")?;
    let snapshot = existing_lock(&root, ".fluxvault-artifacts.lock")?;
    let pointer = root.join(READY);
    let pointer_bytes = read_small(&pointer, 64 * 1024)?;
    let receipt = load(&pointer, &root)?;
    let ledger_path = root
        .join(HISTORY)
        .join(format!("{}.json", receipt.archive_sha256));
    checked(&root.join(HISTORY), true)?;
    let ledger_bytes = read_small(&ledger_path, 64 * 1024)?;
    if load(&ledger_path, &root)? != receipt {
        return Err("Ready pointer differs from its owned handoff receipt".into());
    }
    let archive = root.join(&receipt.archive);
    stage("HANDOFF CHECK / hashing published ZIP...");
    verify(&archive, receipt.archive_bytes, &receipt.archive_sha256)?;
    let mut zip = ZipArchive::new(File::open(&archive).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let contents = archived_contents(&mut zip, &receipt.archive_sha256, &receipt.archive)?;
    let expected = contents
        .iter()
        .map(|(name, bytes, member)| Artifact {
            name: name.clone(),
            bytes: bytes.len() as u64,
            sha256: hash(bytes),
            member: member.clone(),
        })
        .collect::<Vec<_>>();
    if receipt.reports != expected {
        return Err(
            "Handoff report inventory does not replay the archived report generation".into(),
        );
    }
    stage("HANDOFF CHECK / verifying front-and-center reports and checksum...");
    for artifact in &receipt.reports {
        crate::cancellation::check()?;
        verify(&root.join(&artifact.name), artifact.bytes, &artifact.sha256)?;
    }
    stage("HANDOFF CHECK / rechecking snapshot bindings...");
    drop(zip);
    verify(&archive, receipt.archive_bytes, &receipt.archive_sha256)?;
    for artifact in &receipt.reports {
        verify(&root.join(&artifact.name), artifact.bytes, &artifact.sha256)?;
    }
    if read_small(&pointer, 64 * 1024)? != pointer_bytes
        || read_small(&ledger_path, 64 * 1024)? != ledger_bytes
        || owner.is_none()
            && root
                .join(".fluxvault-processing.lock")
                .try_exists()
                .map_err(|e| e.to_string())?
        || snapshot.is_none()
            && root
                .join(".fluxvault-artifacts.lock")
                .try_exists()
                .map_err(|e| e.to_string())?
    {
        return Err("Published delivery or writer admission changed during verification".into());
    }
    Ok(
        json!({"schema_version":1, "scope":"published_archive_and_front_reports_only", "integrity_verified":true,
        "archive":archive, "archive_sha256":receipt.archive_sha256,"archive_bytes":receipt.archive_bytes,
        "reports_verified":receipt.reports.iter().filter(|a|a.member.is_some()).count(),"checksum_verified":true,
        "ready":pointer,"report_generation_replayed":true,"source_archive_required":false,
        "current_project_reprocessed":false,"customer_delivery_certified":false,
        "physical_media_access":false,"host_tools_invoked":false,"files_written":0,
        "warning":"Integrity of the published archival snapshot only. This does not certify intact recovered content, current project completeness or a newer project generation."}),
    )
}
