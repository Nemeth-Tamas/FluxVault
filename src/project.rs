use std::{
    collections::BTreeMap,
    fs,
    fs::OpenOptions,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const PROJECT_FILE_NAME: &str = "project.json";
const PROJECT_SCHEMA_VERSION: u32 = 1;
const MAX_METADATA_BYTES: u64 = 64 * 1024;

const PROJECT_DIRECTORIES: &[&str] = &[
    "Images",
    "Logs",
    "Extracted",
    "Converted",
    "Recovery",
    "Reports",
    "Flux",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjectMetadata {
    schema_version: u32,
    project_name: String,
    created_unix_ms: u64,
    updated_unix_ms: u64,
    current_disk_number: u32,
    // Preserve optional/older producer extensions when numbering advances.
    // Unknown schema versions are still refused, never silently migrated.
    #[serde(flatten)]
    extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ProjectState {
    root: PathBuf,
    metadata: ProjectMetadata,
}

impl ProjectState {
    pub fn create_without_session(root: PathBuf) -> Result<Self, String> {
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(root)
        };
        crate::safety::workstation_path(&root)?;
        // Check the nearest existing parent before creating anything: a junction
        // into source media must not receive even an empty project directory.
        let mut ancestor = root.as_path();
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .ok_or("Cannot resolve project destination parent")?;
        }
        crate::safety::workstation_path(&ancestor.canonicalize().map_err(|e| e.to_string())?)?;
        fs::create_dir_all(&root).map_err(|error| {
            format!(
                "Nem sikerült létrehozni a projektmappát {}: {error}",
                root.display()
            )
        })?;

        let project_file = root.join(PROJECT_FILE_NAME);

        if project_file.exists() {
            return Err(format!(
                "Ebben a mappában már létezik FluxVault projekt: {}",
                project_file.display()
            ));
        }

        for directory in PROJECT_DIRECTORIES {
            let path = root.join(directory);

            fs::create_dir_all(&path).map_err(|error| {
                format!(
                    "Nem sikerült létrehozni a projekt könyvtárát {}: {error}",
                    path.display()
                )
            })?;
        }

        let project_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("FluxVault projekt")
            .to_owned();

        let now = current_unix_ms()?;

        let mut project = Self {
            root,
            metadata: ProjectMetadata {
                schema_version: PROJECT_SCHEMA_VERSION,
                project_name,
                created_unix_ms: now,
                updated_unix_ms: now,
                current_disk_number: 1,
                extensions: BTreeMap::new(),
            },
        };

        project.save_metadata()?;
        Ok(project)
    }

    /// Open a project without changing any application-wide state.
    pub fn open_without_session(root: PathBuf) -> Result<Self, String> {
        crate::safety::workstation_path(&root)?;
        crate::safety::workstation_path(&root.canonicalize().map_err(|e| e.to_string())?)?;
        let project_file = root.join(PROJECT_FILE_NAME);
        regular_metadata(&project_file)?;
        let mut json = Vec::new();
        fs::File::open(&project_file)
            .and_then(|f| f.take(MAX_METADATA_BYTES + 1).read_to_end(&mut json))
            .map_err(|error| {
                format!(
                    "Nem sikerült megnyitni a projektfájlt {}: {error}",
                    project_file.display()
                )
            })?;
        if json.len() as u64 > MAX_METADATA_BYTES {
            return Err("Project metadata exceeds 64 KiB".into());
        }

        let mut metadata: ProjectMetadata = serde_json::from_slice(&json).map_err(|error| {
            format!(
                "Hibás FluxVault projektfájl {}: {error}",
                project_file.display()
            )
        })?;

        if metadata.schema_version != PROJECT_SCHEMA_VERSION {
            return Err(format!(
                "Nem támogatott projektséma: {}. A program ezt támogatja: {}.",
                metadata.schema_version, PROJECT_SCHEMA_VERSION
            ));
        }

        metadata.current_disk_number = metadata.current_disk_number.max(1);

        Ok(Self { root, metadata })
    }

    fn save_metadata(&mut self) -> Result<(), String> {
        crate::safety::workstation_path(&self.root)?;
        crate::safety::workstation_path(&self.root.canonicalize().map_err(|e| e.to_string())?)?;
        self.metadata.updated_unix_ms = current_unix_ms()?;

        let json = serde_json::to_string_pretty(&self.metadata)
            .map_err(|error| format!("Projekt JSON generálási hiba: {error}"))?;
        if json.len() as u64 > MAX_METADATA_BYTES {
            return Err("Project metadata exceeds 64 KiB".into());
        }

        let project_file = self.root.join(PROJECT_FILE_NAME);

        if project_file.try_exists().map_err(|e| e.to_string())? {
            regular_metadata(&project_file)?;
        }
        // A crash must not leave a truncated project.json while scan numbering advances.
        let temporary = self.root.join(format!(
            ".fluxvault-project-{}-{}.partial.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(json.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temporary, &project_file)
            .map_err(|e| format!("Cannot commit project metadata: {e}"))?;

        Ok(())
    }

    /// Persist the active disk number without touching physical media.
    pub fn set_current_disk_number_without_session(
        &mut self,
        disk_number: u32,
    ) -> Result<(), String> {
        if disk_number == 0 {
            return Err("Disk number must be positive".to_owned());
        }
        let previous = self.metadata.clone();
        self.metadata.current_disk_number = disk_number;
        if let Err(error) = self.save_metadata() {
            self.metadata = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.metadata.project_name
    }

    pub(crate) fn extension(&self, key: &str) -> Option<&serde_json::Value> {
        self.metadata.extensions.get(key)
    }

    // Caller must hold project ownership and its short metadata snapshot guard.
    pub(crate) fn save_extension(
        &mut self,
        key: &str,
        value: serde_json::Value,
    ) -> Result<(), String> {
        let previous = self.metadata.clone();
        self.metadata.extensions.insert(key.to_owned(), value);
        if let Err(error) = self.save_metadata() {
            self.metadata = previous;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn tool_settings(&self) -> Result<crate::external_tools::ToolSettings, String> {
        crate::project_settings::effective_tools(self)
    }

    pub(crate) fn default_workers(&self) -> Result<usize, String> {
        Ok(crate::project_settings::get(self)?
            .conversion_workers
            .unwrap_or(crate::conversion_run::DEFAULT_CONVERSION_WORKERS))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn current_disk_number(&self) -> u32 {
        self.metadata.current_disk_number
    }

    pub fn images_dir(&self) -> PathBuf {
        self.root.join("Images")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("Logs")
    }

    pub fn extracted_dir(&self) -> PathBuf {
        self.root.join("Extracted")
    }

    pub fn converted_dir(&self) -> PathBuf {
        self.root.join("Converted")
    }

    pub fn recovery_dir(&self) -> PathBuf {
        self.root.join("Recovery")
    }

    pub fn reports_dir(&self) -> PathBuf {
        self.root.join("Reports")
    }

    pub fn project_file(&self) -> PathBuf {
        self.root.join(PROJECT_FILE_NAME)
    }
}

fn regular_metadata(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_METADATA_BYTES {
        return Err("Project metadata must be a bounded regular file".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Project metadata must not be a reparse point".into());
        }
    }
    Ok(())
}

fn current_unix_ms() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Rendszeridő hiba: {error}"))?;

    u64::try_from(duration.as_millis()).map_err(|_| "A rendszeridő értéke túl nagy.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "fluxvault-project-atomic-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn old_metadata_extensions_survive_numbering_without_read_only_rewrites() {
        let root = temporary_root();
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(project.project_file()).unwrap()).unwrap();
        value["operator_settings"] = serde_json::json!({"operator":"archive team","notes":"keep"});
        value["tools"] =
            serde_json::json!({"seven_zip":{"path":"saved path","version":"historical"}});
        value["current_disk_number"] = serde_json::json!(0);
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(project.project_file(), &bytes).unwrap();
        let mut old = ProjectState::open_without_session(root.clone()).unwrap();
        assert_eq!(old.current_disk_number(), 1);
        assert_eq!(fs::read(old.project_file()).unwrap(), bytes);
        old.set_current_disk_number_without_session(53).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(old.project_file()).unwrap()).unwrap();
        assert_eq!(saved["operator_settings"], value["operator_settings"]);
        assert_eq!(saved["tools"], value["tools"]);
        assert_eq!(saved["created_unix_ms"], value["created_unix_ms"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_oversized_and_future_schemas_are_refused_without_changing_them() {
        let root = temporary_root();
        let project = ProjectState::create_without_session(root.clone()).unwrap();
        let mut future: serde_json::Value =
            serde_json::from_slice(&fs::read(project.project_file()).unwrap()).unwrap();
        future["schema_version"] = serde_json::json!(2);
        for bytes in [
            b"{".to_vec(),
            serde_json::to_vec(&future).unwrap(),
            vec![b' '; MAX_METADATA_BYTES as usize + 1],
        ] {
            fs::write(project.project_file(), &bytes).unwrap();
            assert!(ProjectState::open_without_session(root.clone()).is_err());
            assert_eq!(fs::read(project.project_file()).unwrap(), bytes);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_numbering_preserves_metadata_schema_and_reopens_cleanly() {
        let root = temporary_root();
        let mut project = ProjectState::create_without_session(root.clone()).unwrap();
        let created = project.metadata.created_unix_ms;
        let name = project.name().to_owned();
        project
            .set_current_disk_number_without_session(136)
            .unwrap();
        let reopened = ProjectState::open_without_session(root.clone()).unwrap();
        assert_eq!(reopened.current_disk_number(), 136);
        assert_eq!(reopened.metadata.created_unix_ms, created);
        assert_eq!(reopened.name(), name);
        assert_eq!(reopened.metadata.schema_version, PROJECT_SCHEMA_VERSION);
        assert!(!fs::read_dir(&root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("partial")
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn failed_numbering_commit_keeps_old_file_and_restores_in_memory_cursor() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temporary_root();
        let mut project = ProjectState::create_without_session(root.clone()).unwrap();
        let path = root.join(PROJECT_FILE_NAME);
        let original = fs::read(&path).unwrap();
        // Allow metadata/read access, but deny rename/delete while the handle is held.
        let held = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        assert!(project.set_current_disk_number_without_session(2).is_err());
        assert_eq!(project.current_disk_number(), 1);
        assert_eq!(fs::read(&path).unwrap(), original);
        drop(held);
        project.set_current_disk_number_without_session(2).unwrap();
        assert_eq!(
            ProjectState::open_without_session(root.clone())
                .unwrap()
                .current_disk_number(),
            2
        );
        fs::remove_dir_all(root).unwrap();
    }
}
