use std::{
    fs,
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

#[derive(Debug, Serialize, Deserialize)]
struct SessionState {
    last_project_root: String,
}

impl ProjectState {
    pub fn create(root: PathBuf) -> Result<Self, String> {
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

        project.save()?;

        Ok(project)
    }

    pub fn open(root: PathBuf) -> Result<Self, String> {
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

        let project = Self { root, metadata };

        remember_last_project(project.root())?;

        Ok(project)
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.metadata.updated_unix_ms = current_unix_ms()?;

        let json = serde_json::to_string_pretty(&self.metadata)
            .map_err(|error| format!("Projekt JSON generálási hiba: {error}"))?;

        let project_file = self.root.join(PROJECT_FILE_NAME);

        fs::write(&project_file, json).map_err(|error| {
            format!(
                "Nem sikerült menteni a projektfájlt {}: {error}",
                project_file.display()
            )
        })?;

        remember_last_project(&self.root)?;

        Ok(())
    }

    pub fn set_current_disk_number(&mut self, disk_number: u32) -> Result<(), String> {
        self.metadata.current_disk_number = disk_number.max(1);
        self.save()
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

    pub fn reports_dir(&self) -> PathBuf {
        self.root.join("Reports")
    }

    pub fn project_file(&self) -> PathBuf {
        self.root.join(PROJECT_FILE_NAME)
    }
}

pub fn load_last_project() -> Result<Option<ProjectState>, String> {
    let session_file = session_file_path();

    if !session_file.exists() {
        return Ok(None);
    }

    let json = fs::read_to_string(&session_file).map_err(|error| {
        format!(
            "Nem sikerült beolvasni a FluxVault munkamenetet {}: {error}",
            session_file.display()
        )
    })?;

    let session: SessionState = serde_json::from_str(&json).map_err(|error| {
        format!(
            "Hibás FluxVault munkamenetfájl {}: {error}",
            session_file.display()
        )
    })?;

    let root = PathBuf::from(session.last_project_root);

    if !root.join(PROJECT_FILE_NAME).exists() {
        return Err(format!(
            "A legutóbbi projekt már nem található: {}",
            root.display()
        ));
    }

    ProjectState::open(root).map(Some)
}

fn remember_last_project(root: &Path) -> Result<(), String> {
    let session_file = session_file_path();

    if let Some(parent) = session_file.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Nem sikerült létrehozni a FluxVault beállítási mappát {}: {error}",
                parent.display()
            )
        })?;
    }

    let session = SessionState {
        last_project_root: root.display().to_string(),
    };

    let json = serde_json::to_string_pretty(&session)
        .map_err(|error| format!("Munkamenet JSON generálási hiba: {error}"))?;

    fs::write(&session_file, json).map_err(|error| {
        format!(
            "Nem sikerült menteni a FluxVault munkamenetet {}: {error}",
            session_file.display()
        )
    })
}

fn session_file_path() -> PathBuf {
    if let Some(app_data) = std::env::var_os("APPDATA") {
        return PathBuf::from(app_data)
            .join("FluxVault")
            .join("session.json");
    }

    PathBuf::from(".fluxvault-session.json")
}

fn current_unix_ms() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Rendszeridő hiba: {error}"))?;

    u64::try_from(duration.as_millis()).map_err(|_| "A rendszeridő értéke túl nagy.".to_owned())
}
