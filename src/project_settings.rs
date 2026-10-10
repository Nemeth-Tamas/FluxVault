//! Optional project defaults. Recorded versions are historical, never readiness certificates.
use crate::{
    external_tools::{self, ToolKind, ToolSettings, ToolStatus},
    project::ProjectState,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
const KEY: &str = "fluxvault_settings";
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(default)]
    pub operator: Option<String>,
    #[serde(default)]
    pub conversion_workers: Option<usize>,
    #[serde(default)]
    pub tools: ToolSettings,
    #[serde(default)]
    pub observed_versions: BTreeMap<String, serde_json::Value>,
}
pub(crate) fn get(project: &ProjectState) -> Result<Settings, String> {
    let value = project
        .extension(KEY)
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let settings: Settings =
        serde_json::from_value(value).map_err(|e| format!("Invalid project settings: {e}"))?;
    if settings
        .conversion_workers
        .is_some_and(|n| !(1..=16).contains(&n))
        || settings
            .operator
            .as_ref()
            .is_some_and(|s| s.len() > 256 || s.contains(['\r', '\n', '\0']))
    {
        return Err("Project worker count/operator setting is invalid".into());
    }
    for kind in ToolKind::ALL {
        if let Some(path) = settings.tools.path(kind) {
            if !path.is_absolute() {
                return Err("Project tool paths must be absolute".into());
            }
            crate::safety::workstation_path(path)?;
        }
    }
    Ok(settings)
}
pub(crate) fn effective_tools(project: &ProjectState) -> Result<ToolSettings, String> {
    let project = get(project)?;
    let mut effective = external_tools::load_settings()?;
    for kind in ToolKind::ALL {
        if let Some(path) = project.tools.path(kind) {
            effective.set_path(kind, Some(path.to_owned()));
        }
    }
    Ok(effective)
}
pub(crate) fn update(
    project: &ProjectState,
    key: &str,
    value: Option<&str>,
) -> Result<serde_json::Value, String> {
    let _owner = crate::project_work::reserve(project.root())?;
    let _snapshot = crate::project_work::snapshot(project.root())?;
    let mut fresh = ProjectState::open_without_session(project.root().to_owned())?;
    let mut settings = get(&fresh)?;
    match key {
        "operator" => settings.operator = value.map(str::to_owned),
        "conversion-workers" => {
            settings.conversion_workers = value
                .map(|s| s.parse::<usize>().map_err(|_| "Worker count must be 1-16"))
                .transpose()?
        }
        "sevenzip" | "libreoffice" | "greaseweazle" => {
            let kind = match key {
                "sevenzip" => ToolKind::SevenZip,
                "libreoffice" => ToolKind::LibreOffice,
                _ => ToolKind::Greaseweazle,
            };
            let path = value.map(std::path::PathBuf::from);
            if let Some(p) = &path {
                crate::safety::workstation_path(p)?;
                if !p.is_absolute() || !p.is_file() {
                    return Err("Use an existing absolute workstation executable path".into());
                }
                crate::safety::workstation_path(&p.canonicalize().map_err(|e| e.to_string())?)?;
            }
            settings.tools.set_path(kind, path);
        }
        _ => {
            return Err(
                "Settings keys: operator, conversion-workers, sevenzip, libreoffice, greaseweazle"
                    .into(),
            );
        }
    }
    let value = serde_json::to_value(&settings).map_err(|e| e.to_string())?;
    // Validate all settings before committing and preserve unrelated metadata extensions.
    if settings
        .conversion_workers
        .is_some_and(|n| !(1..=16).contains(&n))
        || settings
            .operator
            .as_ref()
            .is_some_and(|s| s.len() > 256 || s.contains(['\r', '\n', '\0']))
    {
        return Err("Invalid worker count/operator setting".into());
    }
    crate::cancellation::check()?;
    fresh.save_extension(KEY, value.clone())?;
    Ok(value)
}
pub(crate) fn record_versions(
    project: &ProjectState,
    statuses: &[ToolStatus],
) -> Result<(), String> {
    let _owner = crate::project_work::reserve(project.root())?;
    let _snapshot = crate::project_work::snapshot(project.root())?;
    let mut fresh = ProjectState::open_without_session(project.root().to_owned())?;
    let mut settings = get(&fresh)?;
    for s in statuses {
        settings.observed_versions.insert(s.kind.display_name().to_owned(),serde_json::json!({"executable":s.executable,"version":s.version,"health":format!("{:?}",s.health),"observed_unix_ms":external_tools::current_unix_ms(),"historical_only":true}));
    }
    fresh.save_extension(
        KEY,
        serde_json::to_value(settings).map_err(|e| e.to_string())?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> ProjectState {
        ProjectState::create_without_session(std::env::temp_dir().join(format!(
                "fv-settings-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )))
        .unwrap()
    }
    #[test]
    fn defaults_set_clear_reload_and_numbering_preserve_settings() {
        let p = project();
        assert_eq!(p.default_workers().unwrap(), 4);
        update(&p, "operator", Some("Archive team")).unwrap();
        update(&p, "conversion-workers", Some("12")).unwrap();
        let mut p = ProjectState::open_without_session(p.root().to_owned()).unwrap();
        assert_eq!(p.default_workers().unwrap(), 12);
        p.set_current_disk_number_without_session(53).unwrap();
        assert_eq!(
            get(&ProjectState::open_without_session(p.root().to_owned()).unwrap())
                .unwrap()
                .operator
                .as_deref(),
            Some("Archive team")
        );
        update(&p, "conversion-workers", None).unwrap();
        assert_eq!(
            ProjectState::open_without_session(p.root().to_owned())
                .unwrap()
                .default_workers()
                .unwrap(),
            4
        );
    }
    #[test]
    fn invalid_busy_and_source_paths_do_not_change_metadata() {
        let p = project();
        let before = std::fs::read(p.project_file()).unwrap();
        for (key, value) in [
            ("operator", "bad\nvalue"),
            ("conversion-workers", "17"),
            ("sevenzip", "A:\\tool.exe"),
            ("libreoffice", "relative.exe"),
            ("unknown", "1"),
        ] {
            assert!(update(&p, key, Some(value)).is_err());
        }
        let owner = crate::project_work::reserve(p.root()).unwrap();
        assert!(update(&p, "operator", Some("x")).is_err());
        drop(owner);
        assert_eq!(std::fs::read(p.project_file()).unwrap(), before);
    }
    #[test]
    fn project_tools_override_global_and_versions_are_historical() {
        let p = project();
        let executable = std::env::current_exe().unwrap();
        update(&p, "sevenzip", Some(executable.to_str().unwrap())).unwrap();
        let p = ProjectState::open_without_session(p.root().to_owned()).unwrap();
        assert_eq!(p.tool_settings().unwrap().seven_zip_path, Some(executable));
        let status = external_tools::initial_statuses();
        record_versions(&p, &status).unwrap();
        let settings =
            get(&ProjectState::open_without_session(p.root().to_owned()).unwrap()).unwrap();
        assert!(
            settings
                .observed_versions
                .values()
                .all(|v| v["historical_only"] == true)
        );
    }
}
