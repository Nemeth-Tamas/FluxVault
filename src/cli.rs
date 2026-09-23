//! Small, hardware-free CLI foundation. Physical media commands will be added
//! only when they can use the same read-only workflow as the GUI.

use std::{
    env,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{imaging, project::ProjectState};

const HELP: &str = "FluxVault — floppy archiving\n\
Usage:\n\
  fluxvault                         Open the GUI\n\
  fluxvault init [path]             Create a project\n\
  fluxvault status [--project PATH] Show project status\n\
  fluxvault --help                  Show this help\n\
Options:\n\
  --json                            Output machine-readable JSON\n\
  --project PATH                    Use a specific project instead of searching upward";

pub fn run_from_env() -> Option<i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        return None;
    }
    Some(
        match run(
            &args,
            &env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ) {
            Ok(output) => {
                println!("{output}");
                0
            }
            Err(message) => {
                eprintln!("FluxVault: {message}");
                2
            }
        },
    )
}

fn run(args: &[String], cwd: &Path) -> Result<String, String> {
    let mut json_output = false;
    let mut project_override: Option<PathBuf> = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => json_output = true,
            "--project" => {
                index += 1;
                let value = args.get(index).ok_or("--project requires a path")?;
                project_override = Some(PathBuf::from(value));
            }
            "--help" | "-h" => positional.push("help".to_owned()),
            value if value.starts_with('-') => {
                return Err(format!("Unknown option: {value}\n{HELP}"));
            }
            value => positional.push(value.to_owned()),
        }
        index += 1;
    }

    match positional.first().map(String::as_str) {
        Some("help") if positional.len() == 1 => Ok(HELP.to_owned()),
        Some("init") if positional.len() <= 2 && project_override.is_none() => {
            let root = positional
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| cwd.to_path_buf());
            let root = if root.is_absolute() {
                root
            } else {
                cwd.join(root)
            };
            let project = ProjectState::create(root)?;
            if json_output {
                Ok(
                    json!({"project": project.root(), "name": project.name(), "created": true})
                        .to_string(),
                )
            } else {
                Ok(format!(
                    "Created project {} at {}",
                    project.name(),
                    project.root().display()
                ))
            }
        }
        Some("status") if positional.len() == 1 => {
            let root = match project_override {
                Some(path) if path.is_absolute() => path,
                Some(path) => cwd.join(path),
                None => discover_project(cwd).ok_or("No FluxVault project found in this directory or its parents; use --project PATH")?,
            };
            let project = ProjectState::open(root)?;
            let stats = imaging::load_project_statistics(&project.images_dir())?;
            if json_output {
                Ok(json!({
                    "project": project.root(),
                    "name": project.name(),
                    "current_disk": project.current_disk_number(),
                    "disks": stats.disk_count,
                    "attempts": stats.total_attempts,
                    "ok_disks": stats.ok_disks,
                    "partial_disks": stats.partial_disks,
                    "best_known_bad_sectors": stats.best_known_bad_sectors,
                })
                .to_string())
            } else {
                Ok(format!(
                    "{} ({})\nCurrent disk: {:03}\nDisks: {} ({} OK, {} partial)\nAttempts: {}\nBest known bad sectors: {}",
                    project.name(),
                    project.root().display(),
                    project.current_disk_number(),
                    stats.disk_count,
                    stats.ok_disks,
                    stats.partial_disks,
                    stats.total_attempts,
                    stats.best_known_bad_sectors
                ))
            }
        }
        _ => Err(format!("Unsupported command or arguments.\n{HELP}")),
    }
}

fn discover_project(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|directory| directory.join("project.json").is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_project_from_nested_directory() {
        let temp = env::temp_dir().join(format!("fluxvault-cli-discovery-{}", std::process::id()));
        // Never delete an existing directory, even if an earlier test run left it behind.
        std::fs::create_dir(&temp).unwrap();
        let nested = temp.join("Extracted").join("007");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(temp.join("project.json"), "{}").unwrap();
        assert_eq!(discover_project(&nested), Some(temp.clone()));
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn rejects_unknown_commands_without_opening_gui() {
        assert!(run(&["acquire".to_owned()], Path::new(".")).is_err());
    }
}
