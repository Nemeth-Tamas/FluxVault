mod view;

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, TryRecvError},
};

use chrono::{DateTime, Local, Utc};

use crate::{
    batch_extraction::{self, BatchExtractionEvent, BatchExtractionRequest, BatchExtractionResult},
    composite::{self, CompositeEvent, CompositeResult, CompositeSource},
    conversion::{
        self, ConversionPlanningEvent, ConversionPlanningRequest, ConversionPlanningResult,
    },
    conversion_run::{self, ConversionEvent, ConversionRequest, ConversionResult},
    external_tools::{self, ToolCheckEvent, ToolKind, ToolSettings, ToolStatus},
    extraction::{self, ExtractionEvent, ExtractionPresence, ExtractionResult},
    floppy::{self, DiskGeometry, FloppyDrive, ProbeResult, WriteProtectionStatus},
    imaging::{
        self, AttemptComparison, AttemptSummary, ImagingEvent, ImagingResult, ProjectStatistics,
        SectorReadState,
    },
    manifest::{self, ManifestEvent, ManifestResult},
    manual_recovery_import::{
        self, ManualRecoveryImportEvent, ManualRecoveryImportRequest, ManualRecoveryImportResult,
    },
    project::{self, ProjectState},
    recovery_backup::{self, RecoveryBackupEvent, RecoveryBackupResult},
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

#[cfg(test)]
mod auto_extraction_tests {
    use super::*;

    fn clean_result() -> ImagingResult {
        ImagingResult {
            output_path: PathBuf::from("project/Images/007_attempt_001.img"),
            metadata_path: PathBuf::from("project/Images/007_attempt_001.json"),
            log_path: PathBuf::from("project/Logs/007_attempt_001.log"),
            disk_number: 7,
            attempt_number: 1,
            sha256: String::new(),
            total_sectors: 2880,
            bad_sectors: Vec::new(),
            retry_recovered: 0,
            bytes_written: 1_474_560,
        }
    }

    #[test]
    fn queues_only_clean_images_belonging_to_the_project() {
        let clean = clean_result();
        let queued = PendingExtraction::from_clean_acquisition(Path::new("project"), &clean)
            .expect("clean image should be queued");
        assert_eq!(queued.disk_number, 7);
        assert_eq!(queued.attempt_number, 1);

        let mut partial = clean.clone();
        partial.bad_sectors.push(16);
        assert!(
            PendingExtraction::from_clean_acquisition(Path::new("project"), &partial).is_none()
        );
        assert!(PendingExtraction::from_clean_acquisition(Path::new("another"), &clean).is_none());
    }
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

#[derive(Debug, Clone)]
struct PendingExtraction {
    project_root: PathBuf,
    image_path: PathBuf,
    disk_number: u32,
    attempt_number: u32,
}

impl PendingExtraction {
    fn from_clean_acquisition(project_root: &Path, result: &ImagingResult) -> Option<Self> {
        if !result.bad_sectors.is_empty()
            || result.output_path.parent() != Some(project_root.join("Images").as_path())
        {
            return None;
        }
        Some(Self {
            project_root: project_root.to_path_buf(),
            image_path: result.output_path.clone(),
            disk_number: result.disk_number,
            attempt_number: result.attempt_number,
        })
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
    pending_extractions: VecDeque<PendingExtraction>,
    extraction_running: bool,
    extraction_stage: String,
    extraction_result: Option<ExtractionResult>,
    extraction_error: Option<String>,
    extraction_presence: Option<ExtractionPresence>,
    extraction_presence_error: Option<String>,
    batch_extraction_receiver: Option<Receiver<BatchExtractionEvent>>,
    batch_extraction_running: bool,
    batch_extraction_stage: String,
    batch_extraction_completed: usize,
    batch_extraction_total: usize,
    batch_extraction_result: Option<BatchExtractionResult>,
    batch_extraction_error: Option<String>,
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
    recovery_backup_receiver: Option<Receiver<RecoveryBackupEvent>>,
    recovery_backup_running: bool,
    recovery_backup_stage: String,
    recovery_backup_result: Option<RecoveryBackupResult>,
    recovery_backup_error: Option<String>,
    manual_recovery_import_receiver: Option<Receiver<ManualRecoveryImportEvent>>,
    manual_recovery_import_running: bool,
    manual_recovery_import_stage: String,
    manual_recovery_import_result: Option<ManualRecoveryImportResult>,
    manual_recovery_import_error: Option<String>,
    manifest_receiver: Option<Receiver<ManifestEvent>>,
    manifest_running: bool,
    manifest_stage: String,
    manifest_result: Option<ManifestResult>,
    manifest_error: Option<String>,
    conversion_planning_receiver: Option<Receiver<ConversionPlanningEvent>>,
    conversion_planning_running: bool,
    conversion_planning_stage: String,
    conversion_planning_result: Option<ConversionPlanningResult>,
    conversion_planning_error: Option<String>,
    conversion_receiver: Option<Receiver<ConversionEvent>>,
    conversion_running: bool,
    conversion_stage: String,
    conversion_completed: usize,
    conversion_total: usize,
    conversion_result: Option<ConversionResult>,
    conversion_error: Option<String>,
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
            pending_extractions: VecDeque::new(),
            extraction_running: false,
            extraction_stage: "Nincs aktív extraction.".to_owned(),
            extraction_result: None,
            extraction_error: None,
            extraction_presence: None,
            extraction_presence_error: None,
            batch_extraction_receiver: None,
            batch_extraction_running: false,
            batch_extraction_stage: "Nincs aktív projekt-batch feldolgozás.".to_owned(),
            batch_extraction_completed: 0,
            batch_extraction_total: 0,
            batch_extraction_result: None,
            batch_extraction_error: None,
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
            recovery_backup_receiver: None,
            recovery_backup_running: false,
            recovery_backup_stage: "Nincs aktív pass1 backup.".to_owned(),
            recovery_backup_result: None,
            recovery_backup_error: None,
            manual_recovery_import_receiver: None,
            manual_recovery_import_running: false,
            manual_recovery_import_stage: "Nincs aktív manual recovery import.".to_owned(),
            manual_recovery_import_result: None,
            manual_recovery_import_error: None,
            manifest_receiver: None,
            manifest_running: false,
            manifest_stage: "Nincs aktív manifest-készítés.".to_owned(),
            manifest_result: None,
            manifest_error: None,
            conversion_planning_receiver: None,
            conversion_planning_running: false,
            conversion_planning_stage: "Nincs aktív delivery/conversion tervezés.".to_owned(),
            conversion_planning_result: None,
            conversion_planning_error: None,
            conversion_receiver: None,
            conversion_running: false,
            conversion_stage: "Nincs aktív Office konverzió.".to_owned(),
            conversion_completed: 0,
            conversion_total: 0,
            conversion_result: None,
            conversion_error: None,
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

        if matches!(
            self.extraction_presence.as_ref(),
            Some(
                ExtractionPresence::ManualRecovery { .. }
                    | ExtractionPresence::InvalidAutomatic { .. }
            )
        ) {
            self.status =
                "A meglévő recovery mappa védett; automatikus extraction nem írhatja felül."
                    .to_owned();
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

    fn queue_extraction_after_imaging(&mut self, result: &ImagingResult) {
        let Some(project) = &self.project else {
            return;
        };
        if let Some(pending) = PendingExtraction::from_clean_acquisition(project.root(), result) {
            self.log(format!(
                "Lemez {:03} automatikus extraction várólistára került (próbálkozás {:03}).",
                pending.disk_number, pending.attempt_number
            ));
            self.pending_extractions.push_back(pending);
        } else if !result.bad_sectors.is_empty() {
            self.log(format!(
                "Lemez {:03}: {} hibás szektor; automatikus extraction helyett recovery szükséges.",
                result.disk_number,
                result.bad_sectors.len()
            ));
        }
    }

    fn start_next_queued_extraction(&mut self) {
        if self.extraction_running || self.batch_extraction_running || self.manifest_running {
            return;
        }
        let Some(seven_zip_executable) = self.ready_tool_path(ToolKind::SevenZip) else {
            return;
        };
        while let Some(pending) = self.pending_extractions.pop_front() {
            let extracted_root = pending.project_root.join("Extracted");
            match extraction::inspect_extraction_presence(
                &extracted_root,
                pending.disk_number,
                pending.attempt_number,
            ) {
                Ok(
                    ExtractionPresence::ManualRecovery { .. }
                    | ExtractionPresence::InvalidAutomatic { .. },
                ) => {
                    self.log(format!(
                        "Lemez {:03}: a meglévő manuális vagy hibás extraction érintetlen maradt.",
                        pending.disk_number
                    ));
                    continue;
                }
                Err(error) => {
                    self.log(format!(
                        "Lemez {:03}: automatikus extraction ellenőrzési hiba: {error}",
                        pending.disk_number
                    ));
                    continue;
                }
                _ => {}
            }

            let logs_directory = pending.project_root.join("Logs");
            let request = extraction::ExtractionRequest {
                seven_zip_executable: seven_zip_executable.clone(),
                image_path: pending.image_path.clone(),
                disk_number: pending.disk_number,
                attempt_number: pending.attempt_number,
                extracted_root,
                command_audit_path: logs_directory.join("external-tools.jsonl"),
                logs_directory,
            };
            self.extraction_receiver = Some(extraction::spawn_extraction(request));
            self.extraction_running = true;
            self.extraction_stage =
                format!("Lemez {:03} automatikus extraction...", pending.disk_number);
            self.extraction_result = None;
            self.extraction_error = None;
            self.log(format!(
                "Automatikus extraction elindult: {}",
                pending.image_path.display()
            ));
            return;
        }
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
                            self.refresh_extraction_presence();
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

    fn start_batch_extraction(&mut self) {
        if self.batch_extraction_running || self.extraction_running || self.manifest_running {
            return;
        }

        let Some(project) = &self.project else {
            self.status = "Projekt-batch feldolgozás előtt nyisson meg egy projektet.".to_owned();
            return;
        };
        let Some(seven_zip_executable) = self.ready_tool_path(ToolKind::SevenZip) else {
            self.status = "A 7-Zip nem érhető el. Ellenőrizze a Beállítások oldalon.".to_owned();
            return;
        };

        let request = BatchExtractionRequest {
            seven_zip_executable,
            images_directory: project.images_dir(),
            logs_directory: project.logs_dir(),
            extracted_root: project.extracted_dir(),
            recovery_root: project.recovery_dir(),
            reports_directory: project.reports_dir(),
            command_audit_path: self.tool_audit_path(),
        };

        self.batch_extraction_receiver = Some(batch_extraction::spawn_batch_extraction(request));
        self.batch_extraction_running = true;
        self.batch_extraction_stage = "Projekt-batch feldolgozás előkészítése...".to_owned();
        self.batch_extraction_completed = 0;
        self.batch_extraction_total = 0;
        self.batch_extraction_result = None;
        self.batch_extraction_error = None;
        self.status = "Projekt-batch extraction és recovery routing folyamatban...".to_owned();
        self.log("Teljes projekt extraction/recovery feldolgozása elindult.");
    }

    fn poll_batch_extraction_events(&mut self) {
        let Some(receiver) = self.batch_extraction_receiver.take() else {
            return;
        };
        let mut finished = false;

        loop {
            match receiver.try_recv() {
                Ok(BatchExtractionEvent::Stage(stage)) => {
                    self.batch_extraction_stage = stage;
                }
                Ok(BatchExtractionEvent::Progress { completed, total }) => {
                    self.batch_extraction_completed = completed;
                    self.batch_extraction_total = total;
                }
                Ok(BatchExtractionEvent::Finished(result)) => {
                    finished = true;
                    self.batch_extraction_running = false;

                    match result {
                        Ok(result) => {
                            self.batch_extraction_stage =
                                "Projekt-batch extraction és recovery routing elkészült."
                                    .to_owned();
                            self.status = self.batch_extraction_stage.clone();
                            self.log(format!(
                                "Projekt-batch kész: {} lemez, {} recovery, {} manuális.",
                                result.total_disks, result.recovery_disks, result.manual_disks
                            ));
                            self.manifest_result = Some(result.manifest.clone());
                            self.batch_extraction_result = Some(result);
                            self.batch_extraction_error = None;
                            self.refresh_project_statistics();
                            self.refresh_extraction_presence();
                        }
                        Err(error) => {
                            self.batch_extraction_stage =
                                "Projekt-batch feldolgozás sikertelen.".to_owned();
                            self.status = self.batch_extraction_stage.clone();
                            self.log(format!("Projekt-batch hiba: {error}"));
                            self.batch_extraction_result = None;
                            self.batch_extraction_error = Some(error);
                        }
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.batch_extraction_running = false;
                    self.batch_extraction_stage =
                        "A projekt-batch háttérfolyamat megszakadt.".to_owned();
                    self.status = self.batch_extraction_stage.clone();
                    self.batch_extraction_error =
                        Some("A projekt-batch háttérfolyamat eredmény nélkül leállt.".to_owned());
                    self.log("A projekt-batch háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }

        if !finished {
            self.batch_extraction_receiver = Some(receiver);
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

    fn start_recovery_backup(&mut self) {
        if self.recovery_backup_running {
            return;
        }

        let Some(project) = &self.project else {
            self.status = "Recovery backup előtt nyisson meg egy projektet.".to_owned();
            return;
        };
        let Some(attempt) = self.attempt_history.last() else {
            self.status = "Nincs backupolható acquisition.".to_owned();
            return;
        };

        if !attempt.attention_required {
            self.status =
                "A hibamentes acquisition nem igényel pass1 recovery backupot.".to_owned();
            return;
        }

        let request = recovery_backup::RecoveryBackupRequest {
            recovery_root: project.recovery_dir(),
            disk_number: self.current_disk_number,
            attempt_number: attempt.attempt_number,
            image_path: project.images_dir().join(&attempt.image_file),
            log_path: (!attempt.log_file.is_empty()).then(|| PathBuf::from(&attempt.log_file)),
            reason: format!(
                "{}; {} hibás szektor",
                attempt.status,
                attempt.bad_sectors.len()
            ),
        };

        self.recovery_backup_receiver = Some(recovery_backup::spawn_backup(request));
        self.recovery_backup_running = true;
        self.recovery_backup_stage = "Első recovery backup készítése...".to_owned();
        self.recovery_backup_result = None;
        self.recovery_backup_error = None;
        self.status = self.recovery_backup_stage.clone();
        self.log(format!(
            "Immutable pass1 recovery backup ellenőrzése: lemez {:03}.",
            self.current_disk_number
        ));
    }

    fn poll_recovery_backup_events(&mut self) {
        let Some(receiver) = self.recovery_backup_receiver.take() else {
            return;
        };
        let mut finished = false;

        loop {
            match receiver.try_recv() {
                Ok(RecoveryBackupEvent::Stage(stage)) => {
                    self.recovery_backup_stage = stage;
                }
                Ok(RecoveryBackupEvent::Finished(result)) => {
                    finished = true;
                    self.recovery_backup_running = false;

                    match result {
                        Ok(result) => {
                            self.recovery_backup_stage = if result.created {
                                "Immutable pass1 recovery backup elkészült.".to_owned()
                            } else {
                                "A meglévő pass1 backup változatlanul megmaradt.".to_owned()
                            };
                            self.status = self.recovery_backup_stage.clone();
                            self.log(format!(
                                "Pass1 recovery backup: {}",
                                result.directory.display()
                            ));
                            self.recovery_backup_result = Some(result);
                            self.recovery_backup_error = None;
                        }
                        Err(error) => {
                            self.recovery_backup_stage =
                                "Pass1 recovery backup sikertelen.".to_owned();
                            self.status = self.recovery_backup_stage.clone();
                            self.log(format!("Recovery backup hiba: {error}"));
                            self.recovery_backup_result = None;
                            self.recovery_backup_error = Some(error);
                        }
                    }

                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.recovery_backup_running = false;
                    self.recovery_backup_stage =
                        "A recovery backup háttérfolyamat megszakadt.".to_owned();
                    self.status = self.recovery_backup_stage.clone();
                    self.recovery_backup_error =
                        Some("A recovery backup háttérfolyamat eredmény nélkül leállt.".to_owned());
                    self.log("A recovery backup háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }

        if !finished {
            self.recovery_backup_receiver = Some(receiver);
        }
    }

    fn start_manual_recovery_import(&mut self) {
        if self.manual_recovery_import_running {
            return;
        }
        if matches!(
            self.extraction_presence,
            Some(ExtractionPresence::ManualRecovery { .. })
        ) {
            self.status =
                "Ehhez a lemezhez már létezik védett manual recovery eredmény.".to_owned();
            return;
        }
        let Some(project) = &self.project else {
            self.status = "Manual recovery import előtt nyisson meg egy projektet.".to_owned();
            return;
        };
        let Some(source_directory) = rfd::FileDialog::new()
            .set_title("DMDE recovered fájlmappa kiválasztása")
            .pick_folder()
        else {
            return;
        };
        let Some(dmde_log_path) = rfd::FileDialog::new()
            .set_title("A recoveryhez tartozó DMDE napló kiválasztása")
            .add_filter("DMDE napló", &["log", "txt"])
            .pick_file()
        else {
            self.status = "A manual recovery import megszakítva: nincs DMDE napló.".to_owned();
            return;
        };

        let request = ManualRecoveryImportRequest {
            source_directory: source_directory.clone(),
            dmde_log_path: dmde_log_path.clone(),
            extracted_root: project.extracted_dir(),
            recovery_root: project.recovery_dir(),
            disk_number: self.current_disk_number,
        };
        self.manual_recovery_import_receiver = Some(manual_recovery_import::spawn_import(request));
        self.manual_recovery_import_running = true;
        self.manual_recovery_import_stage = "Manual recovery import előkészítése...".to_owned();
        self.manual_recovery_import_result = None;
        self.manual_recovery_import_error = None;
        self.status = self.manual_recovery_import_stage.clone();
        self.log(format!(
            "Manual recovery import indult: {} | DMDE napló: {}",
            source_directory.display(),
            dmde_log_path.display()
        ));
    }

    fn poll_manual_recovery_import_events(&mut self) {
        let Some(receiver) = self.manual_recovery_import_receiver.take() else {
            return;
        };
        let mut finished = false;

        loop {
            match receiver.try_recv() {
                Ok(ManualRecoveryImportEvent::Stage(stage)) => {
                    self.manual_recovery_import_stage = stage;
                }
                Ok(ManualRecoveryImportEvent::Finished(result)) => {
                    finished = true;
                    self.manual_recovery_import_running = false;
                    match result {
                        Ok(result) => {
                            self.manual_recovery_import_stage =
                                "Manual recovery import elkészült; manifest frissítése..."
                                    .to_owned();
                            self.status = self.manual_recovery_import_stage.clone();
                            self.log(format!(
                                "Manual recovery import kész: {} fájl, {} bájt.",
                                result.file_count, result.total_bytes
                            ));
                            self.manual_recovery_import_result = Some(result);
                            self.manual_recovery_import_error = None;
                            self.refresh_extraction_presence();
                            self.start_recovered_manifest();
                        }
                        Err(error) => {
                            self.manual_recovery_import_stage =
                                "Manual recovery import sikertelen.".to_owned();
                            self.status = self.manual_recovery_import_stage.clone();
                            self.log(format!("Manual recovery import hiba: {error}"));
                            self.manual_recovery_import_result = None;
                            self.manual_recovery_import_error = Some(error);
                        }
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.manual_recovery_import_running = false;
                    self.manual_recovery_import_stage =
                        "A manual recovery import háttérfolyamat megszakadt.".to_owned();
                    self.status = self.manual_recovery_import_stage.clone();
                    self.manual_recovery_import_error = Some(
                        "A manual recovery import háttérfolyamat eredmény nélkül leállt."
                            .to_owned(),
                    );
                    self.log("A manual recovery import háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }

        if !finished {
            self.manual_recovery_import_receiver = Some(receiver);
        }
    }

    fn start_recovered_manifest(&mut self) {
        if self.manifest_running {
            return;
        }

        let Some(project) = &self.project else {
            self.status = "Manifest készítéséhez nyisson meg egy projektet.".to_owned();
            return;
        };
        let request = manifest::ManifestRequest {
            extracted_root: project.extracted_dir(),
            images_directory: project.images_dir(),
            reports_directory: project.reports_dir(),
        };

        self.manifest_receiver = Some(manifest::spawn_manifest(request));
        self.manifest_running = true;
        self.manifest_stage = "Recovered fájl manifest készítése...".to_owned();
        self.manifest_result = None;
        self.manifest_error = None;
        self.status = self.manifest_stage.clone();
        self.log("MasterFileList.csv frissítése elindult.");
    }

    fn poll_manifest_events(&mut self) {
        let Some(receiver) = self.manifest_receiver.take() else {
            return;
        };
        let mut finished = false;

        loop {
            match receiver.try_recv() {
                Ok(ManifestEvent::Stage(stage)) => self.manifest_stage = stage,
                Ok(ManifestEvent::Finished(result)) => {
                    finished = true;
                    self.manifest_running = false;

                    match result {
                        Ok(result) => {
                            self.manifest_stage = "Recovered fájl manifest elkészült.".to_owned();
                            self.status = self.manifest_stage.clone();
                            self.log(format!(
                                "MasterFileList.csv: {} lemez, {} fájl.",
                                result.disk_count, result.file_count
                            ));
                            self.manifest_result = Some(result);
                            self.manifest_error = None;
                        }
                        Err(error) => {
                            self.manifest_stage = "Manifest készítése sikertelen.".to_owned();
                            self.status = self.manifest_stage.clone();
                            self.log(format!("Manifest hiba: {error}"));
                            self.manifest_result = None;
                            self.manifest_error = Some(error);
                        }
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.manifest_running = false;
                    self.manifest_stage = "A manifest háttérfolyamat megszakadt.".to_owned();
                    self.status = self.manifest_stage.clone();
                    self.manifest_error =
                        Some("A manifest háttérfolyamat eredmény nélkül leállt.".to_owned());
                    self.log("A manifest háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }

        if !finished {
            self.manifest_receiver = Some(receiver);
        }
    }

    fn start_conversion_planning(&mut self) {
        if self.conversion_planning_running || self.conversion_running {
            return;
        }
        let Some(project) = &self.project else {
            self.status =
                "Delivery/conversion tervezés előtt nyisson meg egy projektet.".to_owned();
            return;
        };
        let request = ConversionPlanningRequest {
            extracted_root: project.extracted_dir(),
            converted_root: project.converted_dir(),
            reports_directory: project.reports_dir(),
        };
        self.conversion_planning_receiver = Some(conversion::spawn_conversion_planning(request));
        self.conversion_planning_running = true;
        self.conversion_planning_stage =
            "Recovered eredetik delivery útvonalainak tervezése...".to_owned();
        self.conversion_planning_result = None;
        self.conversion_planning_error = None;
        self.status = self.conversion_planning_stage.clone();
        self.log("Delivery path map és legacy Office conversion plan készítése elindult.");
    }

    fn poll_conversion_planning_events(&mut self) {
        let Some(receiver) = self.conversion_planning_receiver.take() else {
            return;
        };
        let mut finished = false;
        loop {
            match receiver.try_recv() {
                Ok(ConversionPlanningEvent::Stage(stage)) => {
                    self.conversion_planning_stage = stage;
                }
                Ok(ConversionPlanningEvent::Finished(result)) => {
                    finished = true;
                    self.conversion_planning_running = false;
                    match result {
                        Ok(result) => {
                            self.conversion_planning_stage =
                                "Delivery útvonalak és conversion plan elkészült.".to_owned();
                            self.status = self.conversion_planning_stage.clone();
                            self.log(format!(
                                "Conversion plan: {} legacy Office jelölt, {} új eredeti tükör.",
                                result.conversion_candidates, result.mirrored_files
                            ));
                            self.conversion_planning_result = Some(result);
                            self.conversion_planning_error = None;
                        }
                        Err(error) => {
                            self.conversion_planning_stage =
                                "Delivery/conversion tervezés sikertelen.".to_owned();
                            self.status = self.conversion_planning_stage.clone();
                            self.log(format!("Conversion plan hiba: {error}"));
                            self.conversion_planning_result = None;
                            self.conversion_planning_error = Some(error);
                        }
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.conversion_planning_running = false;
                    self.conversion_planning_stage =
                        "A delivery/conversion tervező háttérfolyamat megszakadt.".to_owned();
                    self.status = self.conversion_planning_stage.clone();
                    self.conversion_planning_error =
                        Some("A delivery/conversion tervező eredmény nélkül leállt.".to_owned());
                    self.log("A delivery/conversion tervező háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }
        if !finished {
            self.conversion_planning_receiver = Some(receiver);
        }
    }

    fn start_conversion(&mut self) {
        if self.conversion_running || self.conversion_planning_running {
            return;
        }
        let Some(project) = &self.project else {
            self.status = "Konverzió előtt nyisson meg egy projektet.".to_owned();
            return;
        };
        let Some(libreoffice_executable) = self.ready_tool_path(ToolKind::LibreOffice) else {
            self.status =
                "A LibreOffice nem érhető el. Ellenőrizze a Beállítások oldalon.".to_owned();
            return;
        };
        let request = ConversionRequest {
            planning: ConversionPlanningRequest {
                extracted_root: project.extracted_dir(),
                converted_root: project.converted_dir(),
                reports_directory: project.reports_dir(),
            },
            libreoffice_executable,
            command_audit_path: self.tool_audit_path(),
            timeout_seconds: 45,
        };
        self.conversion_receiver = Some(conversion_run::spawn_conversion(request));
        self.conversion_running = true;
        self.conversion_stage = "Office konverziós sor előkészítése...".to_owned();
        self.conversion_completed = 0;
        self.conversion_total = 0;
        self.conversion_result = None;
        self.conversion_error = None;
        self.status = self.conversion_stage.clone();
        self.log("Régi Office fájlok DOCX/XLSX/PPTX és PDF konverziója elindult.");
    }

    fn poll_conversion_events(&mut self) {
        let Some(receiver) = self.conversion_receiver.take() else {
            return;
        };
        let mut finished = false;
        loop {
            match receiver.try_recv() {
                Ok(ConversionEvent::Stage(stage)) => self.conversion_stage = stage,
                Ok(ConversionEvent::Progress { completed, total }) => {
                    self.conversion_completed = completed;
                    self.conversion_total = total;
                }
                Ok(ConversionEvent::Finished(result)) => {
                    finished = true;
                    self.conversion_running = false;
                    match result {
                        Ok(result) => {
                            self.conversion_stage = "Office konverziós sor elkészült.".to_owned();
                            self.status = self.conversion_stage.clone();
                            self.log(format!(
                                "Office konverzió: {} OK, {} részleges, {} sikertelen, {} timeout.",
                                result.ok, result.partial, result.failed, result.timed_out
                            ));
                            self.conversion_planning_result = Some(result.planning.clone());
                            self.conversion_result = Some(result);
                            self.conversion_error = None;
                        }
                        Err(error) => {
                            self.conversion_stage = "Office konverziós sor sikertelen.".to_owned();
                            self.status = self.conversion_stage.clone();
                            self.log(format!("Office konverzió hiba: {error}"));
                            self.conversion_result = None;
                            self.conversion_error = Some(error);
                        }
                    }
                    break;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    self.conversion_running = false;
                    self.conversion_stage =
                        "Az Office konverziós háttérfolyamat megszakadt.".to_owned();
                    self.status = self.conversion_stage.clone();
                    self.conversion_error = Some(
                        "Az Office konverziós háttérfolyamat eredmény nélkül leállt.".to_owned(),
                    );
                    self.log("Az Office konverziós háttérfolyamat váratlanul leállt.");
                    break;
                }
            }
        }
        if !finished {
            self.conversion_receiver = Some(receiver);
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

        self.refresh_extraction_presence();
    }

    fn refresh_extraction_presence(&mut self) {
        let Some(project) = &self.project else {
            self.extraction_presence = None;
            self.extraction_presence_error = None;
            return;
        };
        let Some(attempt) = self.attempt_history.last() else {
            self.extraction_presence = None;
            self.extraction_presence_error = None;
            return;
        };

        match extraction::inspect_extraction_presence(
            &project.extracted_dir(),
            self.current_disk_number,
            attempt.attempt_number,
        ) {
            Ok(presence) => {
                self.extraction_presence = Some(presence);
                self.extraction_presence_error = None;
            }
            Err(error) => {
                self.extraction_presence = None;
                self.extraction_presence_error = Some(error.clone());
                self.log(format!("Extraction állapotfelmérési hiba: {error}"));
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

        if self
            .probe_result
            .as_ref()
            .map(|result| &result.write_protection)
            != Some(&WriteProtectionStatus::Protected)
        {
            self.status =
                "A floppy fizikai írásvédelme nem igazolt; a lemezkép készítése megtagadva."
                    .to_owned();
            self.log("A forráslemez írásvédelme nem igazolt. Ellenőrizze a fület (nyitott lyuk); ha már védett, az USB meghajtót külön, eldobható lemezzel kell ellenőrizni.");
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

                    self.queue_extraction_after_imaging(&result);
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

                match &result.write_protection {
                    WriteProtectionStatus::Protected => {
                        self.log("A meghajtó írásvédettnek jelzi a lemezt.")
                    }
                    WriteProtectionStatus::Writable => {
                        self.status =
                            "FIGYELEM: a floppy írható; a teljes lemezkép tiltva.".to_owned();
                        self.log("A meghajtó írhatónak jelzi a lemezt. A Windows a háttérben is módosíthatja. Ellenőrizze a fizikai fület (nyitott lyuk); ha az már védett, a meghajtó/adapter írásvédelmi működését külön kell ellenőrizni, nem ügyféllemezen.");
                    }
                    WriteProtectionStatus::Unknown(error) => {
                        self.status = "Az írásvédelem nem ellenőrizhető; a teljes lemezkép tiltva."
                            .to_owned();
                        self.log(format!("Az írásvédelem ellenőrzése sikertelen: {error}"));
                    }
                }

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
