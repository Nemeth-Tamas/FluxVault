use std::{sync::mpsc::Receiver, time::Duration};

use eframe::egui;

use crate::{
    floppy::{self, DiskGeometry, FloppyDrive, ProbeResult},
    imaging::{self, ImagingEvent, ImagingResult, SectorReadState},
    safety::{MediaSafetyPolicy, SourceMediaAccess},
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
    project_path: Option<String>,
    active_source: Option<String>,
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
    status: String,
    operator_log: Vec<String>,
}

impl FluxVaultApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            page: Page::Project,
            project_path: None,
            active_source: None,
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
            status: "Készen áll".to_owned(),
            operator_log: Vec::new(),
        };

        app.log("FluxVault elindult.");
        app.log("Forrás adathordozó biztonsági mód: CSAK OLVASHATÓ.");

        app
    }

    fn log(&mut self, message: impl Into<String>) {
        self.operator_log.push(message.into());
    }

    fn start_full_imaging(&mut self) {
        if self.imaging_running {
            return;
        }

        MediaSafetyPolicy::assert_invariants();

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

        self.status = format!("Teljes lemezkép készítése: {}", drive.root);
        self.log(format!(
            "Teljes READ ONLY lemezkép készítése indul: {}",
            drive.device_path
        ));

        self.imaging_receiver = Some(imaging::start_imaging(drive, geometry, 2));
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
        ui.heading("FluxVault");
        ui.label("Floppy archiváló és adatmentő rendszer");

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        ui.selectable_value(&mut self.page, Page::Project, Page::Project.title());
        ui.selectable_value(&mut self.page, Page::Acquire, Page::Acquire.title());
        ui.selectable_value(&mut self.page, Page::Recovery, Page::Recovery.title());
        ui.selectable_value(&mut self.page, Page::Files, Page::Files.title());
        ui.selectable_value(&mut self.page, Page::Reports, Page::Reports.title());
        ui.selectable_value(&mut self.page, Page::Settings, Page::Settings.title());

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label("Projekt:");

        match &self.project_path {
            Some(path) => {
                ui.label(path);
            }
            None => {
                ui.weak("Nincs megnyitva");
            }
        }

        ui.add_space(8.0);

        ui.label("Forrás:");

        match &self.active_source {
            Some(source) => {
                ui.label(source);
            }
            None => {
                ui.weak("Nincs kiválasztva");
            }
        }
    }

    fn safety_banner(&self, ui: &mut egui::Ui) {
        let read_only = MediaSafetyPolicy::SOURCE_MEDIA_ACCESS == SourceMediaAccess::ReadOnly
            && !MediaSafetyPolicy::ALLOW_PHYSICAL_MEDIA_WRITES;

        ui.horizontal_wrapped(|ui| {
            if read_only {
                ui.colored_label(
                    egui::Color32::from_rgb(70, 200, 120),
                    egui::RichText::new("[READ ONLY] FORRASLEMEZ: CSAK OLVASHATO")
                        .strong()
                        .size(15.0),
                );
            } else {
                ui.colored_label(
                    egui::Color32::RED,
                    egui::RichText::new("[VESZELY] IRASI HOZZAFERES ENGEDELYEZVE")
                        .strong()
                        .size(15.0),
                );
            }

            ui.separator();

            ui.weak("A FluxVault soha nem irhat az ugyfel eredeti floppy lemezere.");
        });
    }

    fn project_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Projekt");

        ui.add_space(8.0);

        ui.label(
            "A FluxVault projekt fogja össze a lemezképeket, naplókat, \
             kinyert fájlokat, adatmentési próbálkozásokat és jelentéseket.",
        );

        ui.add_space(16.0);

        ui.horizontal(|ui| {
            if ui.button("Új projekt").clicked() {
                self.status = "Projekt létrehozása még nincs implementálva.".to_owned();
                self.log("Új projekt gomb megnyomva.");
            }

            if ui.button("Projekt megnyitása").clicked() {
                self.status = "Projekt megnyitása még nincs implementálva.".to_owned();
                self.log("Projekt megnyitása gomb megnyomva.");
            }
        });

        ui.add_space(24.0);

        ui.group(|ui| {
            ui.heading("M0 állapot");

            ui.label("[OK] eframe / egui GUI");
            ui.label("[OK] Windows DPI manifest");
            ui.label("[OK] Központi read-only biztonsági szabály");
            ui.label("[OK] Navigáció és operátori napló");
            ui.label("[TODO] Projektkezelés");
            ui.label("[OK] Fizikai floppy meghajtó felismerése");
            ui.label("[WIP] Nyers, read-only lemezbeolvasás");
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

                    if ui
                        .add_enabled(
                            !self.imaging_running,
                            egui::SelectableLabel::new(selected, drive.display_name()),
                        )
                        .clicked()
                    {
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

            let drive_selected = self.selected_drive.is_some();

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

            ui.weak(
                "A forrás meghajtó kizárólag olvasási hozzáféréssel van megnyitva. \
                 A lemezkép a helyi captures mappába készül.",
            );
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
                    "Retry után megmentett szektorok: {}",
                    result.retry_recovered
                ));
                ui.label(format!("Méret: {} bájt", result.bytes_written));
                ui.monospace(format!("SHA-256: {}", result.sha256));
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
        ui.heading("Adatmentés");

        ui.add_space(8.0);

        ui.label(
            "A hibás vagy részlegesen olvasható lemezek több próbálkozása, \
             DMDE eredményei és később a Greaseweazle flux mentések itt kerülnek össze.",
        );

        ui.add_space(16.0);

        ui.weak("Az adatmentési munkapad még nincs implementálva.");
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
        ui.heading("Jelentések és statisztikák");

        ui.add_space(8.0);

        ui.label(
            "A FluxVault közvetlenül fog professzionális Excel jelentéseket \
             készíteni az archiválási és adatmentési eredményekből.",
        );

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(
                egui::RichText::new("Tervezett Excel jelentés")
                    .strong()
                    .size(16.0),
            );

            ui.add_space(6.0);

            ui.label("- Összesítő dashboard");
            ui.label("- Feldolgozott floppy lemezek száma");
            ui.label("- Hibátlan / részleges / sikertelen beolvasások");
            ui.label("- Hibás és újrapróbált szektorok statisztikája");
            ui.label("- Visszaállított fájlok száma és mérete");
            ui.label("- Adatmentési módszerek megoszlása");
            ui.label("- Konverziós eredmények");
            ui.label("- Lemezenkénti részletes munkalap");
            ui.label("- Szűrhető és színezett állapotok");
            ui.label("- Grafikonok és összesített statisztikák");

            ui.add_space(8.0);

            ui.label(egui::RichText::new("Elsődleges nyelv: magyar").strong());

            ui.label("Később ugyanebből az adatmodellből angol jelentés is készül.");
        });
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
        egui::CollapsingHeader::new("Operátori napló")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
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

            self.safety_banner(ui);

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

            let status_reserve = 72.0;
            let content_height = (ui.available_height() - status_reserve).max(200.0);
            let total_width = ui.available_width();
            let navigation_width = 180.0;

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
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                ui.set_max_width((content_width - 12.0).max(180.0));
                                ui.add_space(4.0);

                                self.current_page(ui);

                                ui.add_space(12.0);
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
            });

            self.operator_log(ui);

            ui.add_space(4.0);
        });
    }
}
