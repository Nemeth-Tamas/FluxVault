mod view;

use std::{path::PathBuf, sync::mpsc::Receiver};

use chrono::{DateTime, Local, Utc};

use crate::{
    floppy::{self, DiskGeometry, FloppyDrive, ProbeResult},
    imaging::{
        self, AttemptComparison, AttemptSummary, ImagingEvent, ImagingResult, ProjectStatistics,
        SectorReadState,
    },
    project::{self, ProjectState},
    report,
    safety::MediaSafetyPolicy,
    ui as ui_theme,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Project,
    Acquire,
    Recovery,
    Files,
    Reports,
    Settings,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Project => "Projekt",
            Self::Acquire => "Lemez beolvasása",
            Self::Recovery => "Adatmentés",
            Self::Files => "Fájlok",
            Self::Reports => "Jelentések",
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
    status: String,
    operator_log: Vec<String>,
}

impl FluxVaultApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui_theme::configure_context(&cc.egui_ctx);

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
            status: "Készen áll".to_owned(),
            operator_log: Vec::new(),
        };

        app.log("FluxVault elindult.");
        app.log("Forrás adathordozó biztonsági mód: CSAK OLVASHATÓ.");

        app.restore_last_project();
        app.refresh_attempt_history();
        app.refresh_project_statistics();

        app
    }

    fn log(&mut self, message: impl Into<String>) {
        self.operator_log.push(message.into());
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
