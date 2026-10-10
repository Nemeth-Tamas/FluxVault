//! Optional immutable unpacked delivery snapshot. Never copy from a changing
//! project after verification: the already verified package is the sole input.
use super::*;

pub(super) fn retain(
    archive_path: &Path,
    partial: &Path,
    published: &Path,
    expected_hash: &str,
) -> Result<PathBuf, String> {
    if published.exists() {
        return Err("Staging destination already exists; refusing replacement".into());
    }
    fs::create_dir(partial).map_err(|e| format!("Cannot reserve staging: {e}"))?;
    copy_and_check(archive_path, partial, expected_hash)
        .map_err(|e| format!("{e}. Unpublished staging retained at {}", partial.display()))?;
    crate::cancellation::check()?;
    fs::rename(partial, published).map_err(|e| {
        format!(
            "Cannot publish staging; retained at {}: {e}",
            partial.display()
        )
    })?;
    Ok(published.to_owned())
}

fn checked_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    let relative = Path::new(name);
    if name.contains(['\\', ':'])
        || name.split('/').any(|part| matches!(part, "" | "." | ".."))
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || relative.as_os_str().is_empty()
    {
        return Err("Unsafe unpacked package member path".into());
    }
    let result = root.join(relative);
    let mut ancestor = Some(result.as_path());
    while let Some(path) = ancestor {
        if path.exists()
            && (fs::symlink_metadata(path)
                .map_err(|e| e.to_string())?
                .is_symlink()
                || is_reparse_point(path)?)
        {
            return Err("Staging refuses linked/reparse paths".into());
        }
        if path == root {
            break;
        }
        ancestor = path.parent();
    }
    Ok(result)
}

fn copy_and_check(archive_path: &Path, root: &Path, expected_hash: &str) -> Result<(), String> {
    if hash_file(archive_path)? != expected_hash {
        return Err("Verified package changed before staging".into());
    }
    let mut archive = ZipArchive::new(File::open(archive_path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let mut inventory = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        crate::cancellation::check()?;
        let mut member = archive.by_index(index).map_err(|e| e.to_string())?;
        if member.is_dir() {
            return Err("Unexpected directory package member".into());
        }
        let name = member.name().to_owned();
        let path = checked_path(root, &name)?;
        fs::create_dir_all(path.parent().ok_or("Missing staging parent")?)
            .map_err(|e| e.to_string())?;
        checked_path(root, &name)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let mut digest = Sha256::new();
        let mut bytes = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            crate::cancellation::check()?;
            let count = member.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            digest.update(&buffer[..count]);
            bytes += count as u64;
        }
        output.sync_all().map_err(|e| e.to_string())?;
        if bytes != member.size() {
            return Err("Staging member size mismatch".into());
        }
        inventory.push((name, bytes, format!("{:x}", digest.finalize())));
    }
    for (name, bytes, hash) in &inventory {
        crate::cancellation::check()?;
        let path = checked_path(root, name)?;
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() != *bytes
            || hash_file(&path)? != *hash
        {
            return Err(format!("Unpacked staging verification failed: {name}"));
        }
    }
    let mut pending = vec![root.to_owned()];
    let names = inventory
        .iter()
        .map(|(name, _, _)| name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut count = 0;
    while let Some(folder) = pending.pop() {
        crate::cancellation::check()?;
        for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let name = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            checked_path(root, &name)?;
            if path.is_dir() {
                pending.push(path);
            } else if names.contains(name.as_str()) {
                count += 1;
            } else {
                return Err("Untracked member appeared in staging".into());
            }
        }
    }
    if count != inventory.len() || hash_file(archive_path)? != expected_hash {
        return Err("Staging inventory or verified package changed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "fv-staging-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    fn archive(root: &Path, name: &str) -> (PathBuf, String) {
        let path = root.join("test.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"unchanged payload").unwrap();
        writer.finish().unwrap().sync_all().unwrap();
        let hash = hash_file(&path).unwrap();
        (path, hash)
    }

    #[test]
    fn stage_is_exact_zip_snapshot_and_existing_stage_never_replaced() {
        let root = fixture();
        let (zip, hash) = archive(&root, "Extracted/001/customer.doc");
        let partial = root.join("test.partial.staging");
        let published = root.join("test.staging");
        retain(&zip, &partial, &published, &hash).unwrap();
        assert_eq!(
            fs::read(published.join("Extracted/001/customer.doc")).unwrap(),
            b"unchanged payload"
        );
        assert!(!partial.exists());
        assert!(
            retain(&zip, &partial, &published, &hash)
                .unwrap_err()
                .contains("already exists")
        );
        assert!(!partial.exists());
        assert_eq!(hash_file(&zip).unwrap(), hash);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changed_zip_keeps_failed_stage_without_publishing() {
        let root = fixture();
        let (zip, _) = archive(&root, "README.txt");
        let partial = root.join("partial");
        let published = root.join("published");
        let error = retain(&zip, &partial, &published, "wrong hash").unwrap_err();
        assert!(error.contains("changed before staging"));
        assert!(partial.is_dir());
        assert!(!published.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsafe_zip_member_retains_stage_without_escape() {
        let root = fixture();
        let (zip, hash) = archive(&root, "../outside.doc");
        let partial = root.join("partial");
        let published = root.join("published");
        assert!(
            retain(&zip, &partial, &published, &hash)
                .unwrap_err()
                .contains("Unsafe")
        );
        assert!(partial.is_dir());
        assert!(!published.exists());
        assert!(!root.join("outside.doc").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsafe_names_are_refused_and_external_files_untouched() {
        let root = fixture();
        for name in [
            "../escape",
            "/absolute",
            "A:/media",
            "folder\\escape",
            "folder/./escape",
            "folder//escape",
        ] {
            assert!(checked_path(&root, name).is_err(), "{name}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancellation_keeps_unpublished_stage_and_original_zip() {
        let root = fixture();
        let (zip, hash) = archive(&root, "README.txt");
        let partial = root.join("partial");
        let published = root.join("published");
        let token = crate::cancellation::Token::default();
        let scope = crate::cancellation::enter(token.clone());
        token.request();
        let error = retain(&zip, &partial, &published, &hash).unwrap_err();
        assert!(crate::cancellation::stopped(&error));
        assert!(partial.is_dir());
        assert!(!published.exists());
        drop(scope);
        assert_eq!(hash_file(&zip).unwrap(), hash);
        fs::remove_dir_all(root).unwrap();
    }
}
