mod view;

use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};

use chrono::{DateTime, Local, Utc};

use crate::{
    composite::{self, CompositeEvent, CompositeResult, CompositeSource},
    external_tools::{self, ToolCheckEvent, ToolKind, ToolSettings, ToolStatus},
    extraction::{self, ExtractionEvent, ExtractionResult},
    floppy::{self, DiskGeometry, FloppyDrive, ProbeResult},
    imaging::{
        self, AttemptComparison, AttemptSummary, ImagingEvent, ImagingResult, ProjectStatistics,
        SectorReadState,
    },
    project::{self, ProjectState},
    report,
    safety::MediaSafetyPolicy,
    sector_recovery::{self, ReconstructionEvent, ReconstructionResult},
    ui as ui_theme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Project,
    Acquire,
    Recovery,
    Files,
    Conversions,
    Audit,
    Reports,
    Package,
    Settings,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Project => "Projekt",
            Self::Acquire => "Lemez beolvasása",
            Self::Recovery => "Adatmentés",
            Self::Files => "Fájlok",
            Self::Conversions => "Konverziók",
            Self::Audit => "Audit",
            Self::Reports => "Jelentések",
            Self::Package => "Csomag",
            Self::Settings => "Beállítások",
        }
    }
}

pub struct FluxVaultApp {
    page: Page,
    project: Option<ProjectState>,
    active_source: Option<String>,
    current_disk_number: u32,
    sector_retries: usize,
    floppy_drives: Vec<FloppyDrive>,
    selected_drive: Option<usize>,
    probe_result: Option<ProbeResult>,
    imaging_receiver: Option<Receiver<ImagingEvent>>,
    imaging_running: bool,
    imaging_geometry: Option<DiskGeometry>,
    imaging_states: Vec<SectorReadState>,
    imaging_completed_sectors: usize,
    imaging_total_sectors: usize,
    imaging_output: Option<String>,
    imaging_result: Option<ImagingResult>,
    imaging_error: Option<String>,
    attempt_history: Vec<AttemptSummary>,
    attempt_comparison: Option<AttemptComparison>,
    attempt_history_error: Option<String>,
    project_statistics: Option<ProjectStatistics>,
    project_statistics_error: Option<String>,
    tool_settings: ToolSettings,
    tool_statuses: Vec<ToolStatus>,
    tool_check_receiver: Option<Receiver<ToolCheckEvent>>,
    tool_check_running: bool,
    extraction_receiver: Option<Receiver<ExtractionEvent>>,
    extraction_running: bool,
    extraction_stage: String,
    extraction_result: Option<ExtractionResult>,
    extraction_error: Option<String>,
    reconstruction_receiver: Option<Receiver<ReconstructionEvent>>,
    reconstruction_running: bool,
    reconstruction_stage: String,
    reconstruction_result: Option<ReconstructionResult>,
    reconstruction_error: Option<String>,
    composite_receiver: Option<Receiver<CompositeEvent>>,
    composite_running: bool,
    composite_stage: String,
    composite_result: Option<CompositeResult>,
    composite_error: Option<String>,
    status: String,
    operator_log: Vec<String>,
}

impl FluxVaultApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui_theme::configure_context(&cc.egui_ctx);

        let (tool_settings, tool_settings_error) = match external_tools::load_settings() {
            Ok(settings) => (settings, None),
            Err(error) => (ToolSettings::default(), Some(error)),
        };

        let mut app = Self {
            page: Page::Project,
            project: None,
            active_source: None,
            current_disk_number: 1,
            sector_retries: 2,
            floppy_drives: Vec::new(),
            selected_drive: None,
            probe_result: None,
            imaging_receiver: None,
            imaging_running: false,
            imaging_geometry: None,
            imaging_states: Vec::new(),
            imaging_completed_sectors: 0,
            imaging_total_sectors: 0,
            imaging_output: None,
            imaging_result: None,
            imaging_error: None,
            attempt_history: Vec::new(),
            attempt_comparison: None,
            attempt_history_error: None,
            project_statistics: None,
            project_statistics_error: None,
            tool_settings,
            tool_statuses: external_tools::initial_statuses(),
            tool_check_receiver: None,
            tool_check_running: false,
            extraction_receiver: None,
            extraction_running: false,
            extraction_stage: "Nincs aktív extraction.".to_owned(),
            extraction_result: None,
            extraction_error: None,
            reconstruction_receiver: None,
            reconstruction_running: false,
            reconstruction_stage: "Nincs aktív szektorrekonstrukció.".to_owned(),
            reconstruction_result: None,
            reconstruction_error: None,
            composite_receiver: None,
            composite_running: false,
            composite_stage: "Nincs aktív kompozitkép-készítés.".to_owned(),
            composite_result: None,
            composite_error: None,
            status: "Készen áll".to_owned(),
            operator_log: Vec::new(),
        };

        app.log("FluxVault elindult.");
        app.log("Forrás adathordozó biztonsági mód: CSAK OLVASHATÓ.");

        if let Some(error) = tool_settings_error {
            app.log(format!("Eszközbeállítás betöltési hiba: {error}"));
        }

        app.restore_last_project();
        app.refresh_attempt_history();
        app.refresh_project_statistics();
        app.start_tool_check();

        app
    }

    fn log(&mut self, message: impl Into<String>) {
        self.operator_log.push(format!(
            "[{}] {}",
            Local::now().format("%H:%M:%S"),
            message.into()
        ));
    }

    fn tool_audit_path(&self) -> PathBuf {
        self.project
            .as_ref()
            .map(|project| project.logs_dir().join("external-tools.jsonl"))
            .unwrap_or_else(external_tools::default_audit_path)
    }

    fn start_tool_check(&mut self) {
        if self.tool_check_running {
            return;
        }

        self.tool_statuses = external_tools::checking_statuses();
        self.tool_check_receiver = Some(external_tools::spawn_checks(
            self.tool_settings.clone(),
            self.tool_audit_path(),
        ));
        self.tool_check_running = true;
        self.status = "Külső eszközök ellenőrzése...".to_owned();
        self.log("Külső eszközök ellenőrzése elindult.");
    }

    fn poll_tool_check_events(&mut self) {
        let Some(receiver) = self.tool_check_receiver.take() else {
            return;
        };
        let mut finished = false;

        loop {
            match receiver.try_recv() {
                Ok(ToolCheckEvent::Status(tool_status)) => {
                    let name = tool_status.kind.display_name();
                    self.log(format!("Eszközellenőrzés: {name} | {}", tool_status.detail));

                    if let Some(error) = &tool_status.audit_error {
                        self.log(format!("Eszköznapló írási hiba ({name}): {error}"));
                    }

                    if let Some(existing) = self
                        .tool_statuses
                        .iter_mut()
                        .find(|existing| existing.kind == tool_status.kind)
                    {
                        *existing = tool_status;
                    }
                }
                Ok(ToolCheckEvent::Finished) => {
                    finished = true;
                    self.tool_check_running = false;
                    self.status = "Külső eszközök ellenőrzése kész.".to_owned();
                    self.log("Külső eszközök ellenőrzése befejeződött.");
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.tool_check_running = false;
                    self.status = "Külső eszközök ellenőrzése megszakadt.".to_owned();
                    self.log("A külsőeszköz-ellenőrző háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }

        if !finished {
            self.tool_check_receiver = Some(receiver);
        }
    }

    fn select_tool_path(&mut self, kind: ToolKind) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(format!("{} futtatható fájl", kind.display_name()))
            .add_filter("Windows futtatható fájl", &["exe", "com"])
            .pick_file()
        else {
            return;
        };

        self.tool_settings.set_path(kind, Some(path.clone()));

        match external_tools::save_settings(&self.tool_settings) {
            Ok(()) => {
                self.log(format!(
                    "{} egyéni útvonala mentve: {}",
                    kind.display_name(),
                    path.display()
                ));
                self.start_tool_check();
            }
            Err(error) => self.log(format!("Eszközbeállítás mentési hiba: {error}")),
        }
    }

    fn clear_tool_path(&mut self, kind: ToolKind) {
        self.tool_settings.set_path(kind, None);

        match external_tools::save_settings(&self.tool_settings) {
            Ok(()) => {
                self.log(format!(
                    "{} visszaállítva automatikus felismerésre.",
                    kind.display_name()
                ));
                self.start_tool_check();
            }
            Err(error) => self.log(format!("Eszközbeállítás mentési hiba: {error}")),
        }
    }

    fn ready_tool_path(&self, kind: ToolKind) -> Option<PathBuf> {
        self.tool_statuses
            .iter()
            .find(|status| {
                status.kind == kind && status.health == external_tools::ToolHealth::Ready
            })
            .and_then(|status| status.executable.clone())
    }

    fn start_extraction(&mut self) {
        if self.extraction_running {
            return;
        }

        let Some(project) = &self.project else {
            self.status = "Extraction előtt nyisson meg egy projektet.".to_owned();
            return;
        };
        let Some(attempt) = self.attempt_history.last() else {
            self.status = "Ehhez a lemezhez nincs kinyerhető acquisition.".to_owned();
            return;
        };

        if attempt.attention_required {
            self.status =
                "A legutóbbi lemezkép nem tiszta; előbb az Adatmentés nézetben ellenőrizze."
                    .to_owned();
            return;
        }

        let Some(seven_zip_executable) = self.ready_tool_path(ToolKind::SevenZip) else {
            self.status = "A 7-Zip nem érhető el. Ellenőrizze a Beállítások oldalon.".to_owned();
            return;
        };
        let image_path = project.images_dir().join(&attempt.image_file);
        let request = extraction::ExtractionRequest {
            seven_zip_executable,
            image_path: image_path.clone(),
            disk_number: self.current_disk_number,
            attempt_number: attempt.attempt_number,
            extracted_root: project.extracted_dir(),
            logs_directory: project.logs_dir(),
            command_audit_path: self.tool_audit_path(),
        };

        self.extraction_receiver = Some(extraction::spawn_extraction(request));
        self.extraction_running = true;
        self.extraction_stage = "Extraction előkészítése...".to_owned();
        self.extraction_result = None;
        self.extraction_error = None;
        self.status = format!("Extraction folyamatban: {}", image_path.display());
        self.log(format!("Extraction elindult: {}", image_path.display()));
    }

    fn poll_extraction_events(&mut self) {
        let Some(receiver) = self.extraction_receiver.take() else {
            return;
        };
        let mut finished = false;

        loop {
            match receiver.try_recv() {
                Ok(ExtractionEvent::Stage(stage)) => {
                    self.extraction_stage = stage;
                }
                Ok(ExtractionEvent::Finished(result)) => {
                    finished = true;
                    self.extraction_running = false;

                    match result {
                        Ok(result) => {
                            self.extraction_stage = if result.reused {
                                "A változatlan forráshoz tartozó extraction újra felhasználva."
                                    .to_owned()
                            } else {
                                "Extraction sikeresen befejezve.".to_owned()
                            };
                            self.status = self.extraction_stage.clone();
                            self.log(format!(
                                "Extraction kész: {} | {} fájl | {} bájt",
                                result.output_directory.display(),
                                result.file_count,
                                result.total_bytes
                            ));
                            self.extraction_result = Some(result);
                            self.extraction_error = None;
                        }
                        Err(error) => {
                            self.extraction_stage = "Extraction sikertelen.".to_owned();
                            self.status = self.extraction_stage.clone();
                            self.log(format!("Extraction hiba: {error}"));
                            self.extraction_result = None;
                            self.extraction_error = Some(error);
                        }
                    }

                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.extraction_running = false;
                    self.extraction_stage = "Extraction háttérfolyamat megszakadt.".to_owned();
                    self.status = self.extraction_stage.clone();
                    self.extraction_error =
                        Some("Az extraction háttérfolyamat eredmény nélkül leállt.".to_owned());
                    self.log("Az extraction háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }

        if !finished {
            self.extraction_receiver = Some(receiver);
        }
    }

    fn start_sector_reconstruction(&mut self) {
        if self.reconstruction_running {
            return;
        }

        let Some(project) = &self.project else {
            self.status = "Szektorrekonstrukció előtt nyisson meg egy projektet.".to_owned();
            return;
        };
        let Some(attempt) = self.attempt_history.last() else {
            self.status = "Nincs elemezhető acquisition.".to_owned();
            return;
        };

        if attempt.bad_sectors.is_empty() || attempt.bad_sectors.len() > 2 {
            self.status =
                "A gyors rekonstrukció 1 vagy 2 hibás szektor esetén használható.".to_owned();
            return;
        }

        let image_path = project.images_dir().join(&attempt.image_file);
        let request = sector_recovery::ReconstructionRequest {
            image_path: image_path.clone(),
            recovery_root: project.recovery_dir(),
            disk_number: self.current_disk_number,
            attempt_number: attempt.attempt_number,
            bad_sectors: attempt.bad_sectors.clone(),
        };

        self.reconstruction_receiver = Some(sector_recovery::spawn_reconstruction(request));
        self.reconstruction_running = true;
        self.reconstruction_stage = "Rekonstrukciós lehetőségek elemzése...".to_owned();
        self.reconstruction_result = None;
        self.reconstruction_error = None;
        self.status = format!("Szektorrekonstrukció elemzése: {}", image_path.display());
        self.log(format!(
            "Bizonyíték-alapú szektorrekonstrukció elindult: {}",
            image_path.display()
        ));
    }

    fn poll_reconstruction_events(&mut self) {
        let Some(receiver) = self.reconstruction_receiver.take() else {
            return;
        };
        let mut finished = false;

        loop {
            match receiver.try_recv() {
                Ok(ReconstructionEvent::Stage(stage)) => {
                    self.reconstruction_stage = stage;
                }
                Ok(ReconstructionEvent::Finished(result)) => {
                    finished = true;
                    self.reconstruction_running = false;

                    match result {
                        Ok(result) => {
                            self.reconstruction_stage = if result.reconstructed.is_empty() {
                                "Nincs biztonságosan rekonstruálható szektor.".to_owned()
                            } else if result.unresolved_bad_sectors.is_empty() {
                                "FAT redundancia-alapú szektorrekonstrukció elkészült.".to_owned()
                            } else {
                                "Részleges FAT rekonstrukció elkészült; maradt megoldatlan szektor."
                                    .to_owned()
                            };
                            self.status = self.reconstruction_stage.clone();
                            self.log(format!(
                                "Szektorrekonstrukció: {} rekonstruált, {} megoldatlan szektor.",
                                result.reconstructed.len(),
                                result.unresolved_bad_sectors.len()
                            ));
                            self.reconstruction_result = Some(result);
                            self.reconstruction_error = None;
                        }
                        Err(error) => {
                            self.reconstruction_stage =
                                "Szektorrekonstrukció sikertelen.".to_owned();
                            self.status = self.reconstruction_stage.clone();
                            self.log(format!("Szektorrekonstrukciós hiba: {error}"));
                            self.reconstruction_result = None;
                            self.reconstruction_error = Some(error);
                        }
                    }

                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.reconstruction_running = false;
                    self.reconstruction_stage =
                        "A szektorrekonstrukciós háttérfolyamat megszakadt.".to_owned();
                    self.status = self.reconstruction_stage.clone();
                    self.reconstruction_error = Some(
                        "A szektorrekonstrukciós háttérfolyamat eredmény nélkül leállt.".to_owned(),
                    );
                    self.log("A szektorrekonstrukciós háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }

        if !finished {
            self.reconstruction_receiver = Some(receiver);
        }
    }

    fn start_composite(&mut self) {
        if self.composite_running {
            return;
        }

        let Some(project) = &self.project else {
            self.status = "Kompozit kép előtt nyisson meg egy projektet.".to_owned();
            return;
        };

        if self.attempt_history.len() < 2 {
            self.status = "Kompozit képhez legalább két próbálkozás szükséges.".to_owned();
            return;
        }

        if self
            .attempt_history
            .iter()
            .any(|attempt| attempt.bad_sectors.is_empty())
        {
            self.status =
                "Már létezik hibamentes próbálkozás; kompozit kép nem szükséges.".to_owned();
            return;
        }

        let sources = self
            .attempt_history
            .iter()
            .map(|attempt| CompositeSource {
                attempt_number: attempt.attempt_number,
                image_path: project.images_dir().join(&attempt.image_file),
                total_sectors: attempt.total_sectors,
                bad_sectors: attempt.bad_sectors.clone(),
            })
            .collect();
        let request = composite::CompositeRequest {
            recovery_root: project.recovery_dir(),
            disk_number: self.current_disk_number,
            sources,
        };

        self.composite_receiver = Some(composite::spawn_composite(request));
        self.composite_running = true;
        self.composite_stage = "Kompozit lehetőségek elemzése...".to_owned();
        self.composite_result = None;
        self.composite_error = None;
        self.status = self.composite_stage.clone();
        self.log(format!(
            "Bizonyíték-alapú kompozitkép-elemzés elindult a(z) {:03} lemezhez.",
            self.current_disk_number
        ));
    }

    fn poll_composite_events(&mut self) {
        let Some(receiver) = self.composite_receiver.take() else {
            return;
        };
        let mut finished = false;

        loop {
            match receiver.try_recv() {
                Ok(CompositeEvent::Stage(stage)) => {
                    self.composite_stage = stage;
                }
                Ok(CompositeEvent::Finished(result)) => {
                    finished = true;
                    self.composite_running = false;

                    match result {
                        Ok(result) => {
                            self.composite_stage = if result.replacements.is_empty() {
                                "A próbálkozások között nincs bizonyíthatóan pótolható szektor."
                                    .to_owned()
                            } else if result.unresolved_bad_sectors.is_empty() {
                                "A bizonyíték-alapú kompozit kép elkészült.".to_owned()
                            } else {
                                "Részleges kompozit elkészült; maradt megoldatlan szektor."
                                    .to_owned()
                            };
                            self.status = self.composite_stage.clone();
                            self.log(format!(
                                "Kompozit eredmény: {} pótolt, {} megoldatlan szektor.",
                                result.replacements.len(),
                                result.unresolved_bad_sectors.len()
                            ));
                            self.composite_result = Some(result);
                            self.composite_error = None;
                        }
                        Err(error) => {
                            self.composite_stage = "Kompozitkép-készítés sikertelen.".to_owned();
                            self.status = self.composite_stage.clone();
                            self.log(format!("Kompozitkép-hiba: {error}"));
                            self.composite_result = None;
                            self.composite_error = Some(error);
                        }
                    }

                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.composite_running = false;
                    self.composite_stage = "A kompozitkép-háttérfolyamat megszakadt.".to_owned();
                    self.status = self.composite_stage.clone();
                    self.composite_error =
                        Some("A kompozitkép-háttérfolyamat eredmény nélkül leállt.".to_owned());
                    self.log("A kompozitkép-háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }

        if !finished {
            self.composite_receiver = Some(receiver);
        }
    }

    fn acquisition_directory(&self) -> PathBuf {
        self.project
            .as_ref()
            .map(ProjectState::images_dir)
            .unwrap_or_else(|| PathBuf::from("captures"))
    }

    fn acquisition_log_directory(&self) -> PathBuf {
        self.project
            .as_ref()
            .map(ProjectState::logs_dir)
            .unwrap_or_else(|| PathBuf::from("captures"))
    }

    fn restore_last_project(&mut self) {
        match project::load_last_project() {
            Ok(Some(project)) => {
                self.current_disk_number = project.current_disk_number();

                let name = project.name().to_owned();
                let root = project.root().display().to_string();

                self.project = Some(project);

                self.status = format!("Projekt visszaállítva: {name}");

                self.log(format!("Legutóbbi projekt visszaállítva: {name} | {root}"));
            }
            Ok(None) => {}
            Err(error) => {
                self.log(format!("Legutóbbi projekt visszaállítási hiba: {error}"));
            }
        }
    }

    fn activate_project(&mut self, project: ProjectState) {
        let name = project.name().to_owned();
        let root = project.root().display().to_string();
        let disk_number = project.current_disk_number();

        self.project = Some(project);
        self.current_disk_number = disk_number;

        self.probe_result = None;
        self.imaging_geometry = None;
        self.imaging_states.clear();
        self.imaging_completed_sectors = 0;
        self.imaging_total_sectors = 0;
        self.imaging_output = None;
        self.imaging_result = None;
        self.imaging_error = None;

        self.status = format!("Projekt megnyitva: {name}");

        self.log(format!("Projekt aktiválva: {name} | {root}"));

        self.refresh_attempt_history();
        self.refresh_project_statistics();
    }

    fn create_project_interactive(&mut self) {
        let Some(root) = rfd::FileDialog::new()
            .set_title("Új FluxVault projekt mappája")
            .pick_folder()
        else {
            return;
        };

        match ProjectState::create(root) {
            Ok(project) => {
                self.activate_project(project);
            }
            Err(error) => {
                self.status = "A projekt létrehozása sikertelen.".to_owned();

                self.log(format!("Projekt létrehozási hiba: {error}"));
            }
        }
    }

    fn open_project_interactive(&mut self) {
        let Some(root) = rfd::FileDialog::new()
            .set_title("FluxVault projekt megnyitása")
            .pick_folder()
        else {
            return;
        };

        match ProjectState::open(root) {
            Ok(project) => {
                self.activate_project(project);
            }
            Err(error) => {
                self.status = "A projekt megnyitása sikertelen.".to_owned();

                self.log(format!("Projekt megnyitási hiba: {error}"));
            }
        }
    }

    fn export_excel_report(&mut self) {
        let Some(project) = &self.project else {
            self.status = "Excel jelentéshez aktív projekt szükséges.".to_owned();
            return;
        };

        let Some(statistics) = self.project_statistics.clone() else {
            self.status = "Nincs exportálható projektstatisztika.".to_owned();
            return;
        };

        let project_name = project.name().to_owned();
        let reports_directory = project.reports_dir();
        let acquisition_directory = project.images_dir();

        self.status = "Excel jelentés készítése...".to_owned();

        match report::export_hungarian_report(
            &project_name,
            &reports_directory,
            &acquisition_directory,
            &statistics,
        ) {
            Ok(path) => {
                self.status = "Excel jelentés elkészült.".to_owned();

                self.log(format!("Excel jelentés elkészült: {}", path.display()));
            }
            Err(error) => {
                self.status = "Excel jelentés készítése sikertelen.".to_owned();

                self.log(format!("Excel jelentés készítési hiba: {error}"));
            }
        }
    }

    fn format_timestamp(timestamp_unix_ms: u128) -> String {
        let Ok(timestamp_unix_ms) = i64::try_from(timestamp_unix_ms) else {
            return "Ismeretlen időpont".to_owned();
        };

        let Some(timestamp_utc) = DateTime::<Utc>::from_timestamp_millis(timestamp_unix_ms) else {
            return "Ismeretlen időpont".to_owned();
        };

        timestamp_utc
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    fn refresh_attempt_history(&mut self) {
        let acquisition_directory = self.acquisition_directory();

        match imaging::load_attempts_for_disk(&acquisition_directory, self.current_disk_number) {
            Ok(attempts) => {
                self.attempt_comparison = imaging::compare_latest_attempts(&attempts);

                self.attempt_history = attempts;
                self.attempt_history_error = None;
            }
            Err(error) => {
                self.attempt_history.clear();
                self.attempt_comparison = None;
                self.attempt_history_error = Some(error.clone());

                self.log(format!("Próbálkozási előzmények betöltési hibája: {error}"));
            }
        }
    }

    fn refresh_project_statistics(&mut self) {
        let acquisition_directory = self.acquisition_directory();

        match imaging::load_project_statistics(&acquisition_directory) {
            Ok(statistics) => {
                self.project_statistics = Some(statistics);
                self.project_statistics_error = None;
            }
            Err(error) => {
                self.project_statistics = None;
                self.project_statistics_error = Some(error.clone());

                self.log(format!("Projektstatisztika betöltési hiba: {error}"));
            }
        }
    }

    fn select_disk_number(&mut self, disk_number: u32) {
        if self.imaging_running {
            return;
        }

        let disk_number = disk_number.max(1);

        if self.current_disk_number == disk_number {
            self.refresh_attempt_history();
            return;
        }

        self.current_disk_number = disk_number;

        self.probe_result = None;
        self.imaging_geometry = None;
        self.imaging_states.clear();
        self.imaging_completed_sectors = 0;
        self.imaging_total_sectors = 0;
        self.imaging_output = None;
        self.imaging_result = None;
        self.imaging_error = None;

        self.status = "Készen áll".to_owned();

        let project_save_error = self.project.as_mut().and_then(|project| {
            project
                .set_current_disk_number(self.current_disk_number)
                .err()
        });

        if let Some(error) = project_save_error {
            self.log(format!("Projektállapot mentési hiba: {error}"));
        }

        self.log(format!(
            "Aktuális ügyféllemez kiválasztva: {:03}.",
            self.current_disk_number
        ));

        self.refresh_attempt_history();
    }

    fn advance_to_next_disk(&mut self) {
        let next_disk = self.current_disk_number.saturating_add(1).max(1);
        self.select_disk_number(next_disk);
    }

    fn return_to_previous_disk(&mut self) {
        let previous_disk = self.current_disk_number.saturating_sub(1).max(1);
        self.select_disk_number(previous_disk);
    }

    fn start_full_imaging(&mut self) {
        if self.imaging_running {
            return;
        }

        MediaSafetyPolicy::assert_invariants();

        if self.current_disk_number == 0 {
            self.status = "A lemezszám nem lehet 000.".to_owned();
            return;
        }

        let Some(index) = self.selected_drive else {
            self.status = "Nincs kiválasztott meghajtó.".to_owned();
            return;
        };

        let Some(drive) = self.floppy_drives.get(index).cloned() else {
            self.status = "A kiválasztott meghajtó már nem érhető el.".to_owned();
            return;
        };

        let Some(geometry) = self
            .probe_result
            .as_ref()
            .and_then(|result| result.geometry)
        else {
            self.status = "Előbb sikeres próbaolvasás szükséges.".to_owned();
            return;
        };

        if !geometry.looks_like_floppy() {
            self.status = "A meghajtó geometriája nem tűnik floppy geometriának.".to_owned();
            self.log("Teljes kép készítése megtagadva: nem floppy méretű geometria.");
            return;
        }

        let total_sectors = geometry.total_sectors() as usize;

        self.imaging_geometry = Some(geometry);
        self.imaging_states = vec![SectorReadState::Unread; total_sectors];
        self.imaging_completed_sectors = 0;
        self.imaging_total_sectors = total_sectors;
        self.imaging_output = None;
        self.imaging_result = None;
        self.imaging_error = None;
        self.imaging_running = true;

        self.status = format!(
            "Lemez {:03} teljes lemezképének készítése: {}",
            self.current_disk_number, drive.root
        );

        self.log(format!(
            "Lemez {:03} READ ONLY lemezkép készítése indul: {}",
            self.current_disk_number, drive.device_path
        ));

        let acquisition_directory = self.acquisition_directory();
        let acquisition_log_directory = self.acquisition_log_directory();

        self.log(format!(
            "Kimeneti mappa: {}",
            acquisition_directory.display()
        ));

        self.log(format!(
            "Napló mappa: {}",
            acquisition_log_directory.display()
        ));

        self.log(format!(
            "Hibás szektor retry beállítás: {}",
            self.sector_retries
        ));

        self.imaging_receiver = Some(imaging::start_imaging(
            drive,
            geometry,
            acquisition_directory,
            acquisition_log_directory,
            self.current_disk_number,
            self.sector_retries,
        ));
    }

    fn poll_imaging_events(&mut self) {
        let mut events = Vec::new();

        if let Some(receiver) = &self.imaging_receiver {
            while let Ok(event) = receiver.try_recv() {
                events.push(event);
            }
        }

        let mut finished = false;

        for event in events {
            match event {
                ImagingEvent::Started {
                    output_path,
                    total_sectors,
                } => {
                    self.imaging_output = Some(output_path.display().to_string());
                    self.imaging_total_sectors = total_sectors;

                    self.log(format!("Cél lemezkép: {}", output_path.display()));
                }
                ImagingEvent::Sector { lba, state } => {
                    if let Some(slot) = self.imaging_states.get_mut(lba) {
                        *slot = state;
                    }
                }
                ImagingEvent::Progress { completed, total } => {
                    self.imaging_completed_sectors = completed;
                    self.imaging_total_sectors = total;

                    self.status = format!("Lemezkép készítése: {completed}/{total} szektor");
                }
                ImagingEvent::Log(message) => {
                    self.log(message);
                }
                ImagingEvent::Completed(result) => {
                    let bad_count = result.bad_sectors.len();

                    if bad_count == 0 {
                        self.status =
                            format!("Lemezkép kész: {} szektor, 0 hiba.", result.total_sectors);
                    } else {
                        self.status =
                            format!("Részleges lemezkép kész: {bad_count} hibás szektor.");
                    }

                    self.log(format!(
                        "Kép elkészült: {} | SHA-256: {} | hibás szektorok: {}",
                        result.output_path.display(),
                        result.sha256,
                        bad_count
                    ));

                    self.imaging_result = Some(result);
                    self.imaging_running = false;
                    finished = true;
                }
                ImagingEvent::Failed(error) => {
                    self.status = "A lemezkép készítése sikertelen.".to_owned();
                    self.log(format!("KÉPKÉSZÍTÉSI HIBA: {error}"));
                    self.imaging_error = Some(error);
                    self.imaging_running = false;
                    finished = true;
                }
            }
        }

        if finished {
            self.imaging_receiver = None;
            self.refresh_attempt_history();
            self.refresh_project_statistics();
        }
    }

    fn refresh_floppy_drives(&mut self) {
        self.probe_result = None;
        self.selected_drive = None;
        self.active_source = None;

        match floppy::enumerate_removable_drives() {
            Ok(drives) => {
                let count = drives.len();

                self.floppy_drives = drives;

                if count == 0 {
                    self.status = "Nem található cserélhető meghajtó.".to_owned();
                    self.log("Meghajtókeresés: nem található cserélhető meghajtó.");
                } else {
                    self.status = format!("{count} cserélhető meghajtó található.");
                    self.log(format!(
                        "Meghajtókeresés kész: {count} cserélhető meghajtó."
                    ));
                }
            }
            Err(error) => {
                self.floppy_drives.clear();
                self.status = "Meghajtókeresési hiba.".to_owned();
                self.log(format!("Meghajtókeresési hiba: {error}"));
            }
        }
    }

    fn probe_selected_drive(&mut self) {
        MediaSafetyPolicy::assert_invariants();

        let Some(index) = self.selected_drive else {
            self.status = "Nincs kiválasztott meghajtó.".to_owned();
            return;
        };

        let Some(drive) = self.floppy_drives.get(index).cloned() else {
            self.status = "A kiválasztott meghajtó már nem érhető el.".to_owned();
            self.selected_drive = None;
            self.active_source = None;
            return;
        };

        self.status = format!("Read-only próbaolvasás: {}", drive.root);
        self.log(format!(
            "READ ONLY próbaolvasás indul: {} -> {}",
            drive.root, drive.device_path
        ));

        match floppy::probe_read_only(&drive) {
            Ok(result) => {
                self.status = format!(
                    "Sikeres read-only próbaolvasás: {} bájt.",
                    result.bytes_read
                );

                self.log(format!(
                    "READ ONLY próbaolvasás sikeres: {} bájt a(z) {} meghajtóról.",
                    result.bytes_read, drive.root
                ));

                self.probe_result = Some(result);
            }
            Err(error) => {
                self.status = "A read-only próbaolvasás sikertelen.".to_owned();
                self.log(format!("READ ONLY próbaolvasási hiba: {error}"));
                self.probe_result = None;
            }
        }
    }
}
