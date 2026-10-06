use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const PROJECT_FILE_NAME: &str = "project.json";
const PROJECT_SCHEMA_VERSION: u32 = 1;

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
}

#[derive(Debug, Clone)]
pub struct ProjectState {
    root: PathBuf,
    metadata: ProjectMetadata,
}

impl ProjectState {
    pub fn create_without_session(root: PathBuf) -> Result<Self, String> {
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
            },
        };

        project.save_metadata()?;
        Ok(project)
    }

    /// Open a project without changing any application-wide state.
    pub fn open_without_session(root: PathBuf) -> Result<Self, String> {
        let project_file = root.join(PROJECT_FILE_NAME);

        let json = fs::read_to_string(&project_file).map_err(|error| {
            format!(
                "Nem sikerült megnyitni a projektfájlt {}: {error}",
                project_file.display()
            )
        })?;

        let mut metadata: ProjectMetadata = serde_json::from_str(&json).map_err(|error| {
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
        self.metadata.updated_unix_ms = current_unix_ms()?;

        let json = serde_json::to_string_pretty(&self.metadata)
            .map_err(|error| format!("Projekt JSON generálási hiba: {error}"))?;

        let project_file = self.root.join(PROJECT_FILE_NAME);

        if fs::symlink_metadata(&project_file).is_ok_and(|m| !m.file_type().is_file()) {
            return Err("Project metadata must be a regular file".to_owned());
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
