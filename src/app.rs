use std::{path::PathBuf, sync::mpsc::Receiver, time::Duration};

use chrono::{DateTime, Local, Utc};
use eframe::egui;

use crate::{
    floppy::{self, DiskGeometry, FloppyDrive, ProbeResult},
    imaging::{
        self, AttemptComparison, AttemptSummary, ImagingEvent, ImagingResult, ProjectStatistics,
        SectorReadState,
    },
    project::{self, ProjectState},
    report,
    safety::{MediaSafetyPolicy, SourceMediaAccess},
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

    fn draw_sector_map(&self, ui: &mut egui::Ui) {
        let Some(geometry) = self.imaging_geometry else {
            ui.weak("Még nincs aktív vagy befejezett lemezkép.");
            return;
        };

        if self.imaging_states.is_empty() {
            ui.weak("Nincs megjeleníthető szektortérkép.");
            return;
        }

        ui.horizontal_wrapped(|ui| {
            ui.colored_label(egui::Color32::DARK_GRAY, "[ ] Olvasatlan");
            ui.colored_label(egui::Color32::from_rgb(70, 200, 120), "[ ] Jo");
            ui.colored_label(egui::Color32::from_rgb(220, 180, 80), "[ ] Retry");
            ui.colored_label(egui::Color32::from_rgb(220, 70, 70), "[ ] Hibas");
        });

        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .id_salt("sector_map_scroll")
            .max_height(380.0)
            .show(ui, |ui| {
                for cylinder in 0..geometry.cylinders as usize {
                    ui.horizontal(|ui| {
                        ui.monospace(format!("C{cylinder:02}"));

                        for head in 0..geometry.heads as usize {
                            ui.add_space(4.0);
                            ui.monospace(format!("H{head}"));

                            for sector_index in 0..geometry.sectors_per_track as usize {
                                let lba = ((cylinder * geometry.heads as usize + head)
                                    * geometry.sectors_per_track as usize)
                                    + sector_index;

                                let state = self
                                    .imaging_states
                                    .get(lba)
                                    .copied()
                                    .unwrap_or(SectorReadState::Unread);

                                let color = match state {
                                    SectorReadState::Unread => egui::Color32::DARK_GRAY,
                                    SectorReadState::Good => egui::Color32::from_rgb(70, 200, 120),
                                    SectorReadState::RetryRecovered => {
                                        egui::Color32::from_rgb(220, 180, 80)
                                    }
                                    SectorReadState::Bad => egui::Color32::from_rgb(220, 70, 70),
                                };

                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(8.0, 8.0),
                                    egui::Sense::hover(),
                                );

                                ui.painter().rect_filled(rect, 1.0, color);

                                response.on_hover_text(format!(
                                    "C{cylinder:02} H{head} S{:02} | LBA {lba} | {:?}",
                                    sector_index + 1,
                                    state
                                ));
                            }
                        }
                    });
                }
            });
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

    fn navigation(&mut self, ui: &mut egui::Ui) {
        ui_theme::subsection_label(ui, "MUNKAFOLYAMAT");

        ui.add_space(4.0);

        ui.selectable_value(&mut self.page, Page::Project, Page::Project.title());

        ui.selectable_value(&mut self.page, Page::Acquire, Page::Acquire.title());

        ui.selectable_value(&mut self.page, Page::Recovery, Page::Recovery.title());

        ui.selectable_value(&mut self.page, Page::Files, Page::Files.title());

        ui.add_space(18.0);

        ui_theme::subsection_label(ui, "KIMENET");

        ui.add_space(4.0);

        ui.selectable_value(&mut self.page, Page::Reports, Page::Reports.title());

        ui.add_space(18.0);

        ui_theme::subsection_label(ui, "RENDSZER");

        ui.add_space(4.0);

        ui.selectable_value(&mut self.page, Page::Settings, Page::Settings.title());

        ui.add_space(24.0);
        ui.separator();
        ui.add_space(12.0);

        ui_theme::subsection_label(ui, "AKTÍV PROJEKT");

        ui.add_space(4.0);

        match &self.project {
            Some(project) => {
                ui.strong(project.name());

                ui.add_space(2.0);

                ui.weak(project.root().display().to_string());
            }
            None => {
                ui.weak("Nincs megnyitva");
            }
        }

        ui.add_space(12.0);

        ui_theme::subsection_label(ui, "FORRÁS");

        ui.add_space(4.0);

        match &self.active_source {
            Some(source) => {
                ui.monospace(source);
            }
            None => {
                ui.weak("Nincs kiválasztva");
            }
        }
    }

    fn workspace_header(&self, ui: &mut egui::Ui) {
        let read_only = MediaSafetyPolicy::SOURCE_MEDIA_ACCESS == SourceMediaAccess::ReadOnly
            && !MediaSafetyPolicy::ALLOW_PHYSICAL_MEDIA_WRITES;

        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("FluxVault").strong().size(22.0));

            ui.weak("Floppy archiváló és adatmentő rendszer");

            ui.separator();

            if read_only {
                ui.colored_label(
                    egui::Color32::from_rgb(70, 200, 120),
                    egui::RichText::new("[READ ONLY]").strong().size(14.0),
                );
            } else {
                ui.colored_label(
                    egui::Color32::RED,
                    egui::RichText::new("[VESZELY: IRAS ENGEDELYEZVE]")
                        .strong()
                        .size(14.0),
                );
            }
        });

        ui.add_space(6.0);

        ui.horizontal_wrapped(|ui| {
            ui.weak("Projekt:");

            match &self.project {
                Some(project) => {
                    ui.strong(project.name());
                    ui.weak(project.root().display().to_string());
                }
                None => {
                    ui.weak("nincs");
                }
            }

            ui.separator();

            ui.weak("Feldolgozott lemezek:");

            ui.strong(
                self.project_statistics
                    .as_ref()
                    .map(|statistics| statistics.disk_count)
                    .unwrap_or(0)
                    .to_string(),
            );

            ui.separator();

            ui.weak("Aktuális:");

            ui.monospace(format!("{:03}", self.current_disk_number));

            ui.separator();

            ui.weak("Következő:");

            ui.monospace(format!("{:03}", self.current_disk_number.saturating_add(1)));

            ui.separator();

            ui.weak("Forrás:");

            match &self.active_source {
                Some(source) => {
                    ui.monospace(source);
                }
                None => {
                    ui.weak("nincs");
                }
            }
        });

        ui.add_space(4.0);

        ui.horizontal_wrapped(|ui| {
            ui.weak("Művelet:");
            ui.label(&self.status);
        });
    }

    fn project_page(&mut self, ui: &mut egui::Ui) {
        ui_theme::page_header(
            ui,
            "Projekt",
            "Projektkezelés, munkamenet és az archiválási munka összesített állapota.",
        );

        ui.horizontal_wrapped(|ui| {
            if ui.button("Új projekt...").clicked() {
                self.create_project_interactive();
            }

            if ui.button("Projekt megnyitása...").clicked() {
                self.open_project_interactive();
            }
        });

        ui.add_space(16.0);

        let Some(project) = &self.project else {
            ui_theme::section(ui, "Nincs aktív projekt", |ui| {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 180, 80),
                    "A teljes archiválási munkafolyamathoz hozzon létre vagy nyisson meg egy projektet.",
                );

                ui.add_space(6.0);

                ui.weak(
                    "Projekt nélkül a teszt acquisitions továbbra is a helyi captures mappába kerülnek.",
                );
            });

            return;
        };

        ui_theme::section(ui, "Projekt adatai", |ui| {
            egui::Grid::new("project_details_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Név");
                    ui.strong(project.name());
                    ui.end_row();

                    ui.label("Gyökérmappa");
                    ui.monospace(project.root().display().to_string());
                    ui.end_row();

                    ui.label("Lemezképek");
                    ui.monospace(project.images_dir().display().to_string());
                    ui.end_row();

                    ui.label("Naplók");
                    ui.monospace(project.logs_dir().display().to_string());
                    ui.end_row();

                    ui.label("Jelentések");
                    ui.monospace(project.reports_dir().display().to_string());
                    ui.end_row();

                    ui.label("Projektfájl");
                    ui.monospace(project.project_file().display().to_string());
                    ui.end_row();

                    ui.label("Aktuális ügyféllemez");
                    ui.strong(format!("{:03}", project.current_disk_number()));
                    ui.end_row();
                });
        });

        ui.add_space(16.0);

        ui_theme::section(ui, "Archiválási áttekintés", |ui| {
            let Some(statistics) = &self.project_statistics else {
                ui.weak("Még nincs elérhető projektstatisztika.");
                return;
            };

            egui::Grid::new("project_overview_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Feldolgozott lemezek");
                    ui.strong(statistics.disk_count.to_string());
                    ui.end_row();

                    ui.label("Olvasási próbálkozások");
                    ui.strong(statistics.total_attempts.to_string());
                    ui.end_row();

                    ui.label("Hibamentes lemezek");
                    ui.colored_label(
                        egui::Color32::from_rgb(70, 200, 120),
                        statistics.ok_disks.to_string(),
                    );
                    ui.end_row();

                    ui.label("Részleges lemezek");
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 180, 80),
                        statistics.partial_disks.to_string(),
                    );
                    ui.end_row();

                    ui.label("Legjobb ismert hibás szektorok");
                    ui.strong(statistics.best_known_bad_sectors.to_string());
                    ui.end_row();
                });
        });
    }

    fn acquire_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Lemez beolvasása");

        ui.add_space(8.0);

        ui.label("Itt készül majd a forrás floppy bitpontos szektoros lemezképe.");

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(egui::RichText::new("Forrás meghajtó").strong().size(16.0));

            ui.add_space(6.0);

            let mut requested_disk_number = self.current_disk_number;
            let mut disk_number_changed = false;

            ui.horizontal(|ui| {
                ui.label("Aktuális ügyféllemez:");

                let response = ui.add_enabled(
                    !self.imaging_running,
                    egui::DragValue::new(&mut requested_disk_number)
                        .range(1..=999_999)
                        .speed(1.0),
                );

                disk_number_changed = response.changed();

                ui.monospace(format!("{:03}", requested_disk_number));
            });

            if disk_number_changed {
                self.select_disk_number(requested_disk_number);
            }

            ui.weak(format!(
                "Kimeneti név: {:03}_attempt_NNN.img",
                self.current_disk_number
            ));

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("Hibás szektor újrapróbálások:");

                ui.add_enabled(
                    !self.imaging_running,
                    egui::DragValue::new(&mut self.sector_retries)
                        .range(0..=10)
                        .speed(1.0),
                );

                ui.weak(format!(
                    "{} összes olvasási próbálkozás / hibás szektor",
                    self.sector_retries + 1
                ));
            });

            ui.add_space(8.0);

            if ui
                .add_enabled(
                    !self.imaging_running,
                    egui::Button::new("Meghajtók frissítése"),
                )
                .clicked()
            {
                self.refresh_floppy_drives();
            }

            ui.add_space(8.0);

            if self.floppy_drives.is_empty() {
                ui.weak("Még nincs felismert cserélhető meghajtó.");
            } else {
                ui.label("Felismert cserélhető meghajtók:");

                let mut newly_selected = None;

                for (index, drive) in self.floppy_drives.iter().enumerate() {
                    let selected = self.selected_drive == Some(index);

                    let drive_clicked = ui
                        .selectable_label(selected, drive.display_name())
                        .clicked();

                    if !self.imaging_running && drive_clicked {
                        newly_selected = Some(index);
                    }
                }

                if let Some(index) = newly_selected {
                    self.selected_drive = Some(index);
                    self.probe_result = None;

                    if let Some(drive) = self.floppy_drives.get(index) {
                        self.active_source = Some(drive.root.clone());
                        self.status = format!("Forrás meghajtó kiválasztva: {}", drive.root);
                    }
                }
            }

            ui.add_space(12.0);

            let drive_selected = self.selected_drive.is_some() && !self.imaging_running;

            if ui
                .add_enabled(
                    drive_selected,
                    egui::Button::new("Read-only próbaolvasás (512 bájt)"),
                )
                .clicked()
            {
                self.probe_selected_drive();
            }

            let imaging_ready = self
                .probe_result
                .as_ref()
                .and_then(|result| result.geometry)
                .map(|geometry| geometry.looks_like_floppy())
                .unwrap_or(false)
                && !self.imaging_running;

            let imaging_button_text = if self.imaging_running {
                "Lemezkép készítése folyamatban..."
            } else {
                "Teljes READ ONLY lemezkép készítése"
            };

            if ui
                .add_enabled(imaging_ready, egui::Button::new(imaging_button_text))
                .clicked()
            {
                self.start_full_imaging();
            }

            ui.add_space(8.0);

            let acquisition_directory = self.acquisition_directory();

            ui.weak(format!(
                "A forrás meghajtó kizárólag olvasási hozzáféréssel van megnyitva. \
                 Kimeneti mappa: {}",
                acquisition_directory.display()
            ));

            ui.weak(
                "Ügyféllemeznél használja a floppy fizikai írásvédő kapcsolóját is, \
                 ha a lemez típusa rendelkezik vele.",
            );

            if self.project.is_none() {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 180, 80),
                    "[INFO] Teszt mód: nincs aktív projekt, ezért a captures mappa használatos.",
                );
            }
        });

        if let Some(result) = &self.probe_result {
            ui.add_space(16.0);

            ui.group(|ui| {
                ui.colored_label(
                    egui::Color32::from_rgb(70, 200, 120),
                    egui::RichText::new("[READ OK] Fizikai floppy olvashato")
                        .strong()
                        .size(18.0),
                );

                ui.add_space(8.0);

                ui.label(format!("Beolvasott bajtok: {}", result.bytes_read));
                ui.monospace(format!("Elso 16 bajt: {}", result.first_bytes_hex()));
                ui.label(format!("510-511. bajt: {}", result.boot_signature_hex()));

                if result.boot_signature == Some([0x55, 0xAA]) {
                    ui.colored_label(
                        egui::Color32::from_rgb(70, 200, 120),
                        "[OK] 55 AA boot signature megtalalva.",
                    );
                } else if result.bytes_read >= 512 {
                    ui.weak("[INFO] Klasszikus 55 AA boot signature nincs.");
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                ui.label(
                    egui::RichText::new("Windows lemezgeometria")
                        .strong()
                        .size(16.0),
                );

                if let Some(geometry) = result.geometry {
                    ui.label(format!("Formatum becsles: {}", geometry.format_guess()));
                    ui.label(format!("Cilinderek: {}", geometry.cylinders));
                    ui.label(format!("Fejek: {}", geometry.heads));
                    ui.label(format!("Szektor / sav: {}", geometry.sectors_per_track));
                    ui.label(format!("Bajt / szektor: {}", geometry.bytes_per_sector));
                    ui.label(format!("Osszes szektor: {}", geometry.total_sectors()));
                    ui.label(format!("Varhato meret: {} bajt", geometry.total_bytes()));
                    ui.label(format!("Windows media type kod: {}", geometry.media_type));
                } else if let Some(error) = &result.geometry_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 180, 80),
                        "[WARN] A szektor olvasasa sikerult, de a geometria lekerdezese nem.",
                    );
                    ui.monospace(error);
                }
            });
        }

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(egui::RichText::new("Élő lemeztérkép").strong().size(16.0));

            ui.add_space(8.0);

            if self.imaging_total_sectors > 0 {
                let progress =
                    self.imaging_completed_sectors as f32 / self.imaging_total_sectors as f32;

                ui.add(
                    egui::ProgressBar::new(progress)
                        .show_percentage()
                        .text(format!(
                            "{} / {} szektor",
                            self.imaging_completed_sectors, self.imaging_total_sectors
                        )),
                );
            }

            if let Some(output) = &self.imaging_output {
                ui.label(format!("Kimenet: {output}"));
            }

            if let Some(result) = &self.imaging_result {
                ui.add_space(8.0);

                if result.bad_sectors.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_rgb(70, 200, 120),
                        "[OK] Hibamentes lemezkép.",
                    );
                } else {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 180, 80),
                        format!(
                            "[PARTIAL] {} olvashatatlan szektor.",
                            result.bad_sectors.len()
                        ),
                    );
                }

                ui.label(format!(
                    "Lemez: {:03} | Olvasási próbálkozás: {:03}",
                    result.disk_number, result.attempt_number
                ));

                ui.label(format!(
                    "Retry után megmentett szektorok: {}",
                    result.retry_recovered
                ));
                ui.label(format!("Méret: {} bájt", result.bytes_written));
                ui.label(format!("Metadata: {}", result.metadata_path.display()));
                ui.label(format!("Napló: {}", result.log_path.display()));
                ui.monospace(format!("SHA-256: {}", result.sha256));

                if !result.bad_sectors.is_empty() {
                    ui.add_space(10.0);

                    ui.label(
                        egui::RichText::new("Olvashatatlan szektorok")
                            .strong()
                            .size(15.0),
                    );

                    if let Some(geometry) = self.imaging_geometry {
                        egui::ScrollArea::vertical()
                            .id_salt("bad_sector_list_scroll")
                            .max_height(110.0)
                            .show(ui, |ui| {
                                for lba in &result.bad_sectors {
                                    let sectors_per_cylinder =
                                        geometry.heads as u64 * geometry.sectors_per_track as u64;

                                    let cylinder = *lba / sectors_per_cylinder;

                                    let within_cylinder = *lba % sectors_per_cylinder;

                                    let head = within_cylinder / geometry.sectors_per_track as u64;

                                    let sector =
                                        within_cylinder % geometry.sectors_per_track as u64 + 1;

                                    ui.monospace(format!(
                                        "LBA {:4} | C{:02} H{} S{:02}",
                                        lba, cylinder, head, sector
                                    ));
                                }
                            });
                    }
                }
            }

            if let Some(error) = &self.imaging_error {
                ui.add_space(8.0);
                ui.colored_label(
                    egui::Color32::from_rgb(220, 70, 70),
                    "[FAILED] A lemezkép készítése megszakadt.",
                );
                ui.monospace(error);
            }

            ui.add_space(10.0);

            self.draw_sector_map(ui);
        });
    }

    fn recovery_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Adatmentés");

            ui.separator();

            ui.strong(format!("Lemez {:03}", self.current_disk_number));

            if ui.button("Előzmények újratöltése").clicked() {
                self.refresh_attempt_history();
            }
        });

        ui.add_space(8.0);

        ui.label(
            "A lemez több, egymástól független olvasási próbálkozása itt \
             hasonlítható össze. Az eredeti próbálkozások változatlanul megmaradnak.",
        );

        if let Some(error) = &self.attempt_history_error {
            ui.add_space(12.0);

            ui.colored_label(
                egui::Color32::from_rgb(220, 70, 70),
                "[HIBA] Nem sikerült betölteni a próbálkozásokat.",
            );

            ui.monospace(error);

            return;
        }

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(
                egui::RichText::new("Olvasási próbálkozások")
                    .strong()
                    .size(16.0),
            );

            ui.add_space(6.0);

            if self.attempt_history.is_empty() {
                ui.weak(format!(
                    "A(z) {:03} lemezhez még nincs számozott FluxVault próbálkozás.",
                    self.current_disk_number
                ));
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("attempt_history_scroll")
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for attempt in &self.attempt_history {
                            ui.group(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(format!("Próbálkozás {:03}", attempt.attempt_number));

                                    ui.separator();

                                    ui.label(&attempt.status);

                                    ui.separator();

                                    if attempt.bad_sectors.is_empty() {
                                        ui.colored_label(
                                            egui::Color32::from_rgb(70, 200, 120),
                                            "0 hibás szektor",
                                        );
                                    } else {
                                        ui.colored_label(
                                            egui::Color32::from_rgb(220, 180, 80),
                                            format!("{} hibás szektor", attempt.bad_sectors.len()),
                                        );
                                    }

                                    ui.separator();

                                    ui.label(format!(
                                        "{} retry után mentett",
                                        attempt.retry_recovered_sectors
                                    ));
                                });

                                ui.label(format!(
                                    "Időpont: {}",
                                    Self::format_timestamp(attempt.timestamp_unix_ms)
                                ));

                                ui.label(format!("Kép: {}", attempt.image_file));

                                ui.label(format!("Metadata: {}", attempt.metadata_path.display()));

                                if !attempt.log_file.is_empty() {
                                    ui.label(format!("Napló: {}", attempt.log_file));
                                } else {
                                    ui.weak(
                                        "Napló: régi acquisition, nincs rögzített log artifact",
                                    );
                                }

                                let short_sha = attempt.sha256.chars().take(16).collect::<String>();

                                ui.monospace(format!("SHA-256: {short_sha}..."));
                            });

                            ui.add_space(6.0);
                        }
                    });
            }
        });

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(
                egui::RichText::new("Legutóbbi két próbálkozás összehasonlítása")
                    .strong()
                    .size(16.0),
            );

            ui.add_space(8.0);

            let Some(comparison) = &self.attempt_comparison else {
                ui.weak("Legalább két, azonos méretű próbálkozás szükséges az összehasonlításhoz.");
                return;
            };

            ui.label(format!(
                "{:03} -> {:03}",
                comparison.older_attempt, comparison.newer_attempt
            ));

            ui.label(format!(
                "Korábbi hibás szektorok: {}",
                comparison.older_bad_count
            ));

            ui.label(format!(
                "Újabb hibás szektorok: {}",
                comparison.newer_bad_count
            ));

            ui.add_space(8.0);

            ui.colored_label(
                egui::Color32::from_rgb(70, 200, 120),
                format!(
                    "Korábban hibás, most olvasható: {}",
                    comparison.recovered_sectors.len()
                ),
            );

            ui.colored_label(
                egui::Color32::from_rgb(220, 180, 80),
                format!(
                    "Mindkét próbálkozásban hibás: {}",
                    comparison.still_bad_sectors.len()
                ),
            );

            ui.colored_label(
                egui::Color32::from_rgb(220, 70, 70),
                format!(
                    "Korábban olvasható, most hibás: {}",
                    comparison.newly_bad_sectors.len()
                ),
            );

            ui.add_space(12.0);

            ui.columns(2, |columns| {
                columns[0].strong("Most visszanyert LBA-k");

                egui::ScrollArea::vertical()
                    .id_salt("recovered_sector_comparison_scroll")
                    .max_height(180.0)
                    .show(&mut columns[0], |ui| {
                        if comparison.recovered_sectors.is_empty() {
                            ui.weak("Nincs.");
                        } else {
                            for lba in &comparison.recovered_sectors {
                                ui.monospace(format!("LBA {lba}"));
                            }
                        }
                    });

                columns[1].strong("Most elveszett LBA-k");

                egui::ScrollArea::vertical()
                    .id_salt("new_bad_sector_comparison_scroll")
                    .max_height(180.0)
                    .show(&mut columns[1], |ui| {
                        if comparison.newly_bad_sectors.is_empty() {
                            ui.weak("Nincs.");
                        } else {
                            for lba in &comparison.newly_bad_sectors {
                                ui.monospace(format!("LBA {lba}"));
                            }
                        }
                    });
            });
        });
    }

    fn files_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Fájlok");

        ui.add_space(8.0);

        ui.label(
            "Kinyert fájlok, SHA-256 értékek, eredeti útvonalak és \
             helyreállítási módszerek áttekintése.",
        );

        ui.add_space(16.0);

        ui.weak("A fájlindex még nincs implementálva.");
    }

    fn reports_page(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Jelentések és statisztikák");

            ui.separator();

            if ui.button("Statisztika újratöltése").clicked() {
                self.refresh_project_statistics();
            }
        });

        ui.add_space(8.0);

        ui.label(
            "Ez a nézet közvetlenül az acquisition metadata fájlokból épül fel. \
             Ugyanez az adatmodell lesz a későbbi Excel jelentések alapja.",
        );

        if let Some(error) = &self.project_statistics_error {
            ui.add_space(12.0);

            ui.colored_label(
                egui::Color32::from_rgb(220, 70, 70),
                "[HIBA] A projektstatisztika nem tölthető be.",
            );

            ui.monospace(error);

            return;
        }

        let Some(statistics) = &self.project_statistics else {
            ui.add_space(16.0);
            ui.weak("Nincs elérhető statisztika.");
            return;
        };

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(egui::RichText::new("Projekt összesítő").strong().size(16.0));

            ui.add_space(8.0);

            egui::Grid::new("project_statistics_summary")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Feldolgozott lemezek");
                    ui.strong(statistics.disk_count.to_string());
                    ui.end_row();

                    ui.label("Összes olvasási próbálkozás");
                    ui.strong(statistics.total_attempts.to_string());
                    ui.end_row();

                    ui.label("Hibamentes lemezek");
                    ui.colored_label(
                        egui::Color32::from_rgb(70, 200, 120),
                        statistics.ok_disks.to_string(),
                    );
                    ui.end_row();

                    ui.label("Részleges lemezek");
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 180, 80),
                        statistics.partial_disks.to_string(),
                    );
                    ui.end_row();

                    ui.label("Legutóbbi próbálkozások hibás szektorai");
                    ui.strong(statistics.latest_bad_sectors.to_string());
                    ui.end_row();

                    ui.label("Legjobb ismert állapot hibás szektorai");
                    ui.strong(statistics.best_known_bad_sectors.to_string());
                    ui.end_row();
                });
        });

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(
                egui::RichText::new("Lemezenkénti állapot")
                    .strong()
                    .size(16.0),
            );

            ui.add_space(8.0);

            if statistics.disks.is_empty() {
                ui.weak("Még nincs számozott FluxVault acquisition ebben a munkaterületben.");
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("project_disk_statistics_scroll")
                    .max_height(340.0)
                    .show(ui, |ui| {
                        for disk in &statistics.disks {
                            ui.group(|ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.strong(format!("Lemez {:03}", disk.disk_number));

                                    ui.separator();

                                    ui.label(format!("{} próbálkozás", disk.attempt_count));

                                    ui.separator();

                                    if disk.best_bad_sectors == 0 {
                                        ui.colored_label(
                                            egui::Color32::from_rgb(70, 200, 120),
                                            "OK",
                                        );
                                    } else {
                                        ui.colored_label(
                                            egui::Color32::from_rgb(220, 180, 80),
                                            "PARTIAL",
                                        );
                                    }
                                });

                                ui.label(format!(
                                    "Legutóbbi: #{:03} | {} | {} hibás szektor",
                                    disk.latest_attempt_number,
                                    disk.latest_status,
                                    disk.latest_bad_sectors
                                ));

                                ui.label(format!(
                                    "Legjobb: #{:03} | {} hibás szektor",
                                    disk.best_attempt_number, disk.best_bad_sectors
                                ));

                                ui.label(format!(
                                    "Utolsó olvasás: {}",
                                    Self::format_timestamp(disk.latest_timestamp_unix_ms)
                                ));

                                ui.weak(format!(
                                    "Lemez szektorainak száma: {}",
                                    disk.total_sectors
                                ));
                            });

                            ui.add_space(6.0);
                        }
                    });
            }
        });

        ui.add_space(16.0);

        let report_ready = self.project.is_some() && self.project_statistics.is_some();

        let mut export_requested = false;

        ui.group(|ui| {
            ui.label(egui::RichText::new("Excel jelentés").strong().size(16.0));

            ui.add_space(6.0);

            ui.label(
                "A projekt aktuális acquisition statisztikáiból közvetlenül \
                 formázott XLSX munkafüzet készül.",
            );

            ui.label("Elsődleges nyelv: magyar");
            ui.label("Később: angol export ugyanebből az adatmodellből.");

            ui.add_space(8.0);

            if ui
                .add_enabled(report_ready, egui::Button::new("Excel jelentés készítése"))
                .clicked()
            {
                export_requested = true;
            }

            if self.project.is_none() {
                ui.weak("A jelentéshez előbb nyisson meg vagy hozzon létre projektet.");
            }
        });

        if export_requested {
            self.export_excel_report();
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Beállítások");

        ui.add_space(8.0);

        ui.group(|ui| {
            ui.label(egui::RichText::new("Külső eszközök").strong().size(16.0));

            ui.add_space(6.0);

            ui.label("7-Zip: még nincs ellenőrizve");
            ui.label("LibreOffice: még nincs ellenőrizve");
            ui.label("Greaseweazle: még nincs ellenőrizve");
        });

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(
                egui::RichText::new("Biztonsági szabályok")
                    .strong()
                    .size(16.0),
            );

            ui.add_space(6.0);

            ui.label("Fizikai floppy írás: TILTOTT");
            ui.label("Greaseweazle írás: TILTOTT");
            ui.label("Forrás média hozzáférés: CSAK OLVASHATÓ");
        });
    }

    fn current_page(&mut self, ui: &mut egui::Ui) {
        match self.page {
            Page::Project => self.project_page(ui),
            Page::Acquire => self.acquire_page(ui),
            Page::Recovery => self.recovery_page(ui),
            Page::Files => self.files_page(ui),
            Page::Reports => self.reports_page(ui),
            Page::Settings => self.settings_page(ui),
        }
    }

    fn operator_log(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Operátori napló");

            ui.separator();

            ui.weak(format!("{} bejegyzés", self.operator_log.len()));
        });

        ui.add_space(4.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("operator_log_scroll")
                .max_height(130.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());

                    for entry in &self.operator_log {
                        ui.monospace(entry);
                    }
                });
        });
    }
}

impl eframe::App for FluxVaultApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_imaging_events();

        if self.imaging_running {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
        }

        ui.vertical(|ui| {
            ui.add_space(6.0);

            self.workspace_header(ui);

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            let content_height =
                (ui.available_height() - ui_theme::BOTTOM_RESERVED_HEIGHT).max(200.0);

            let total_width = ui.available_width();
            let navigation_width = ui_theme::NAVIGATION_WIDTH;

            ui.allocate_ui_with_layout(
                egui::vec2(total_width, content_height),
                egui::Layout::left_to_right(egui::Align::TOP),
                |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(navigation_width, content_height),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            self.navigation(ui);
                        },
                    );

                    ui.separator();

                    let content_width = ui.available_width().max(200.0);

                    ui.allocate_ui_with_layout(
                        egui::vec2(content_width, content_height),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("main_content_scroll")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_min_width((content_width - 24.0).max(180.0));

                                    ui.add_space(8.0);

                                    ui.horizontal(|ui| {
                                        ui.add_space(8.0);

                                        ui.vertical(|ui| {
                                            ui.set_min_width((content_width - 40.0).max(180.0));

                                            self.current_page(ui);

                                            ui.add_space(16.0);
                                        });
                                    });
                                });
                        },
                    );
                },
            );

            ui.separator();
            ui.add_space(4.0);

            ui.horizontal_wrapped(|ui| {
                ui.label("Allapot:");
                ui.strong(&self.status);

                ui.separator();

                ui.strong(format!("Aktualis lemez: {:03}", self.current_disk_number));

                if !self.imaging_running {
                    ui.separator();

                    if ui
                        .add_enabled(
                            self.current_disk_number > 1,
                            egui::Button::new(format!(
                                "< ELOZO: {:03}",
                                self.current_disk_number.saturating_sub(1).max(1)
                            )),
                        )
                        .clicked()
                    {
                        self.return_to_previous_disk();
                    }

                    if ui
                        .add_sized(
                            [190.0, 30.0],
                            egui::Button::new(format!(
                                "KOVETKEZO LEMEZ: {:03} >",
                                self.current_disk_number.saturating_add(1)
                            )),
                        )
                        .clicked()
                    {
                        self.advance_to_next_disk();
                    }
                }
            });

            self.operator_log(ui);

            ui.add_space(4.0);
        });
    }
}
