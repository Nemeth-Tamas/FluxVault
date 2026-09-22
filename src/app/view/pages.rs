use eframe::egui;

use super::super::{FluxVaultApp, Page};
use crate::{
    external_tools::{ToolHealth, ToolKind},
    ui as ui_theme,
};

impl FluxVaultApp {
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
                ui.label("Hibás szektor retry passzok:");

                ui.add_enabled(
                    !self.imaging_running,
                    egui::DragValue::new(&mut self.sector_retries)
                        .range(0..=10)
                        .speed(1.0),
                );

                ui.weak(format!(
                    "1 kezdeti előre olvasás + {} váltott irányú retry passz",
                    self.sector_retries
                ));
            });

            if self.sector_retries >= 1 {
                ui.weak("Retry 1: visszafelé a hibás szektorokon.");
            }

            if self.sector_retries >= 2 {
                ui.weak("Retry 2: előrefelé a még hibás szektorokon.");
            }

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

        let mut recovery_queue = self
            .project_statistics
            .as_ref()
            .map(|statistics| {
                statistics
                    .disks
                    .iter()
                    .filter(|disk| disk.attention_required)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        recovery_queue.sort_by(|left, right| {
            right
                .best_bad_sectors
                .cmp(&left.best_bad_sectors)
                .then_with(|| right.latest_bad_sectors.cmp(&left.latest_bad_sectors))
                .then_with(|| left.disk_number.cmp(&right.disk_number))
        });
        let mut requested_disk = None;

        ui_theme::section(ui, "Recovery várólista", |ui| {
            if recovery_queue.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(70, 200, 120),
                    "[URES] Nincs hibás szektor miatt várakozó lemez.",
                );
                return;
            }

            ui.label(format!(
                "{} lemez igényel figyelmet; a legsúlyosabb esetek vannak elöl.",
                recovery_queue.len()
            ));

            egui::ScrollArea::vertical()
                .id_salt("recovery_queue_scroll")
                .max_height(190.0)
                .show(ui, |ui| {
                    for disk in &recovery_queue {
                        ui.horizontal_wrapped(|ui| {
                            ui.strong(format!("Lemez {:03}", disk.disk_number));
                            ui.separator();
                            ui.colored_label(
                                egui::Color32::from_rgb(220, 180, 80),
                                format!("legjobb állapot: {} hibás", disk.best_bad_sectors),
                            );
                            ui.separator();
                            ui.label(format!(
                                "legutóbbi: {} hibás | {} próbálkozás",
                                disk.latest_bad_sectors, disk.attempt_count
                            ));

                            if ui.button("Megnyitás").clicked() {
                                requested_disk = Some(disk.disk_number);
                            }
                        });
                    }
                });
        });

        if let Some(disk_number) = requested_disk {
            self.select_disk_number(disk_number);
        }

        ui.add_space(16.0);

        let latest_attempt = self.attempt_history.last().cloned();
        let quick_reconstruction_ready = self.project.is_some()
            && latest_attempt
                .as_ref()
                .is_some_and(|attempt| matches!(attempt.bad_sectors.len(), 1 | 2))
            && !self.reconstruction_running;

        ui_theme::section(
            ui,
            "1-2 hibás szektor: bizonyíték-alapú rekonstrukció",
            |ui| {
                ui.label(
                "A FluxVault csak redundáns, olvasható adatokból készít külön származtatott képet. Ismeretlen fájladatot nem talál ki.",
            );

                if let Some(attempt) = &latest_attempt {
                    ui.label(format!(
                        "Legutóbbi próbálkozás: {:03} | hibás szektorok: {}",
                        attempt.attempt_number,
                        attempt.bad_sectors.len()
                    ));

                    if attempt.bad_sectors.len() > 2 {
                        ui.weak("A gyors elemzés legfeljebb két hibás szektorra használható.");
                    } else if attempt.bad_sectors.is_empty() {
                        ui.weak("A lemezkép hibamentes; rekonstrukció nem szükséges.");
                    }
                } else {
                    ui.weak("Nincs elemezhető acquisition.");
                }

                ui.add_space(6.0);

                if ui
                    .add_enabled(
                        quick_reconstruction_ready,
                        egui::Button::new(if self.reconstruction_running {
                            "Rekonstrukció elemzése folyamatban..."
                        } else {
                            "FAT redundancia elemzése"
                        }),
                    )
                    .clicked()
                {
                    self.start_sector_reconstruction();
                }

                ui.label(&self.reconstruction_stage);

                if let Some(error) = &self.reconstruction_error {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 70, 70),
                        "[HIBA] A rekonstrukciós elemzés sikertelen.",
                    );
                    ui.monospace(error);
                }

                if let Some(result) = &self.reconstruction_result {
                    ui.label(format!("Fájlrendszer: {}", result.filesystem));
                    ui.label(format!(
                        "Rekonstruált szektorok: {} | megoldatlan: {}",
                        result.reconstructed.len(),
                        result.unresolved_bad_sectors.len()
                    ));
                    ui.monospace(format!("Forrás: {}", result.source_image.display()));
                    ui.monospace(format!("Forrás SHA-256: {}", result.source_sha256));

                    for record in &result.reconstructed {
                        ui.monospace(format!(
                            "LBA {} <- tükrözött FAT LBA {} ({})",
                            record.target_lba, record.source_lba, record.method
                        ));
                    }

                    if !result.unresolved_bad_sectors.is_empty() {
                        ui.weak(format!(
                            "Nem található ki biztonságosan: {:?}",
                            result.unresolved_bad_sectors
                        ));
                    }

                    if let Some(path) = &result.derived_image {
                        ui.colored_label(
                            egui::Color32::from_rgb(70, 200, 120),
                            "[SZARMAZTATOTT KEP ELKESZULT]",
                        );
                        ui.monospace(format!("Származtatott kép: {}", path.display()));
                    }

                    if let Some(path) = &result.provenance_path {
                        ui.monospace(format!("Provenance: {}", path.display()));
                    }

                    if let Some(hash) = &result.derived_sha256 {
                        ui.monospace(format!("Származtatott SHA-256: {hash}"));
                    }
                }
            },
        );

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

        let composite_ready = self.project.is_some()
            && self.attempt_history.len() >= 2
            && self
                .attempt_history
                .iter()
                .all(|attempt| !attempt.bad_sectors.is_empty())
            && !self.composite_running;

        ui_theme::section(ui, "Legjobb kompozit több próbálkozásból", |ui| {
            ui.label(
                "A legjobb próbálkozás hibás szektorait csak olyan másik próbálkozásból pótolja, ahol ugyanaz a szektor olvasható volt. Az eredetik változatlanok maradnak.",
            );

            if self.attempt_history.len() < 2 {
                ui.weak("Legalább két, azonos geometriájú próbálkozás szükséges.");
            } else if self
                .attempt_history
                .iter()
                .any(|attempt| attempt.bad_sectors.is_empty())
            {
                ui.colored_label(
                    egui::Color32::from_rgb(70, 200, 120),
                    "Már van hibamentes próbálkozás; kompozit nem szükséges.",
                );
            }

            ui.add_space(6.0);

            if ui
                .add_enabled(
                    composite_ready,
                    egui::Button::new(if self.composite_running {
                        "Kompozit elemzése folyamatban..."
                    } else {
                        "Legjobb kompozit elkészítése"
                    }),
                )
                .clicked()
            {
                self.start_composite();
            }

            ui.label(&self.composite_stage);

            if let Some(error) = &self.composite_error {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 70, 70),
                    "[HIBA] A kompozitkép-készítés sikertelen.",
                );
                ui.monospace(error);
            }

            if let Some(result) = &self.composite_result {
                ui.label(format!(
                    "Bázis: {:03} | pótolt: {} | megoldatlan: {}",
                    result.base_attempt,
                    result.replacements.len(),
                    result.unresolved_bad_sectors.len()
                ));

                for replacement in &result.replacements {
                    ui.monospace(format!(
                        "LBA {} <- próbálkozás {:03}",
                        replacement.target_lba, replacement.source_attempt
                    ));
                }

                if !result.unresolved_bad_sectors.is_empty() {
                    ui.weak(format!(
                        "Egyik próbálkozásból sem olvasható: {:?}",
                        result.unresolved_bad_sectors
                    ));
                }

                if let Some(path) = &result.derived_image {
                    ui.colored_label(
                        egui::Color32::from_rgb(70, 200, 120),
                        "[SZARMAZTATOTT KEP ELKESZULT]",
                    );
                    ui.monospace(format!("Kompozit kép: {}", path.display()));
                }

                if let Some(path) = &result.provenance_path {
                    ui.monospace(format!("Provenance: {}", path.display()));
                }

                if let Some(hash) = &result.derived_sha256 {
                    ui.monospace(format!("Kompozit SHA-256: {hash}"));
                }
            }
        });

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
                                    ui.strong(if attempt.legacy_image {
                                        "Legacy kép".to_owned()
                                    } else {
                                        format!("Próbálkozás {:03}", attempt.attempt_number)
                                    });

                                    ui.separator();

                                    ui.label(&attempt.status);

                                    ui.separator();

                                    if !attempt.attention_required {
                                        ui.colored_label(
                                            egui::Color32::from_rgb(70, 200, 120),
                                            "0 hibás szektor",
                                        );
                                    } else if attempt.bad_sectors.is_empty() {
                                        ui.colored_label(
                                            egui::Color32::from_rgb(220, 180, 80),
                                            "ellenőrzést igényel",
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

                                if attempt.legacy_image {
                                    ui.weak("Legacy import: külön FluxVault metadata még nincs.");
                                } else {
                                    ui.label(format!(
                                        "Metadata: {}",
                                        attempt.metadata_path.display()
                                    ));
                                }

                                if !attempt.log_file.is_empty() {
                                    ui.label(format!("Napló: {}", attempt.log_file));

                                    if let Some(parsed_log) = &attempt.parsed_log {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(format!(
                                                "Napló állapot: {}",
                                                parsed_log.status.label()
                                            ));
                                            ui.separator();
                                            ui.label(format!(
                                                "{} hibás LBA, {} retry után mentett",
                                                parsed_log.bad_sectors.len(),
                                                parsed_log.retry_recovered
                                            ));

                                            if !parsed_log.end_seen {
                                                ui.separator();
                                                ui.colored_label(
                                                    egui::Color32::from_rgb(220, 180, 80),
                                                    "Befejezetlen napló",
                                                );
                                            }
                                        });

                                        egui::CollapsingHeader::new("Napló részletei")
                                            .id_salt(format!(
                                                "attempt_log_details_{}",
                                                attempt.attempt_number
                                            ))
                                            .show(ui, |ui| {
                                                ui.horizontal_wrapped(|ui| {
                                                ui.label(format!(
                                                    "Rekord: lemez {}, próbálkozás {}",
                                                    parsed_log
                                                        .disk_number
                                                        .map(|value| format!("{value:03}"))
                                                        .unwrap_or_else(|| "?".to_owned()),
                                                    parsed_log
                                                        .attempt_number
                                                        .map(|value| format!("{value:03}"))
                                                        .unwrap_or_else(|| "?".to_owned())
                                                ));
                                                ui.separator();
                                                ui.label(format!(
                                                    "Forrás: {}",
                                                    parsed_log.source.as_deref().unwrap_or("nincs")
                                                ));
                                                });

                                                ui.label(format!(
                                                "Geometria: {} cilinder, {} fej, {} szektor/sáv, {} bájt/szektor",
                                                parsed_log
                                                    .geometry
                                                    .cylinders
                                                    .map(|value| value.to_string())
                                                    .unwrap_or_else(|| "?".to_owned()),
                                                parsed_log
                                                    .geometry
                                                    .heads
                                                    .map(|value| value.to_string())
                                                    .unwrap_or_else(|| "?".to_owned()),
                                                parsed_log
                                                    .geometry
                                                    .sectors_per_track
                                                    .map(|value| value.to_string())
                                                    .unwrap_or_else(|| "?".to_owned()),
                                                parsed_log
                                                    .geometry
                                                    .bytes_per_sector
                                                    .map(|value| value.to_string())
                                                    .unwrap_or_else(|| "?".to_owned())
                                                ));

                                                ui.label(format!(
                                                "Retry hibák: {} | Kiírt bájtok: {} | BEGIN: {}",
                                                parsed_log.retry_failures,
                                                parsed_log
                                                    .bytes_written
                                                    .map(|value| value.to_string())
                                                    .unwrap_or_else(|| "?".to_owned()),
                                                if parsed_log.begin_seen { "igen" } else { "nem" }
                                                ));

                                                if let Some(sha256) = &parsed_log.sha256 {
                                                    ui.monospace(format!(
                                                        "Napló SHA-256: {sha256}"
                                                    ));
                                                }
                                            });
                                    } else if let Some(dmde_log) = &attempt.parsed_dmde_log {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(format!(
                                                "DMDE állapot: {}",
                                                dmde_log.status.label()
                                            ));
                                            ui.separator();
                                            ui.label(format!(
                                                "{} passz ({} előre, {} hátra)",
                                                dmde_log.pass_count,
                                                dmde_log.forward_passes,
                                                dmde_log.reverse_passes
                                            ));
                                            ui.separator();
                                            ui.label(format!(
                                                "{} hibás szektor",
                                                dmde_log.bad_sectors.len()
                                            ));
                                        });

                                        egui::CollapsingHeader::new("DMDE napló részletei")
                                            .id_salt(format!(
                                                "attempt_dmde_details_{}",
                                                attempt.attempt_number
                                            ))
                                            .show(ui, |ui| {
                                                ui.label(format!(
                                                    "Szektorméret: {} bájt | feltérképezett: {} szektor",
                                                    dmde_log
                                                        .sector_size
                                                        .map(|value| value.to_string())
                                                        .unwrap_or_else(|| "?".to_owned()),
                                                    dmde_log.highest_sector_exclusive
                                                ));
                                                ui.label(format!(
                                                    "Legutolsó állapot szerint olvasható: {} | START: {} | STOP: {}",
                                                    dmde_log.copied_sectors,
                                                    dmde_log.start_count,
                                                    dmde_log.stop_count
                                                ));
                                            });
                                    } else {
                                        ui.weak("A napló nem olvasható vagy nem felismerhető.");
                                    }
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
        ui_theme::page_header(
            ui,
            "Fájlok és extraction",
            "Tiszta lemezképek ellenőrzött kibontása, fájlleltára és SHA-256 nyilvántartása.",
        );

        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Lemez {:03}", self.current_disk_number));
            ui.separator();

            if ui.button("Acquisition újratöltése").clicked() {
                self.refresh_attempt_history();
            }
        });

        ui.add_space(14.0);

        let latest_attempt = self.attempt_history.last().cloned();
        let seven_zip_ready = self.ready_tool_path(ToolKind::SevenZip).is_some();
        let extraction_ready = self.project.is_some()
            && latest_attempt
                .as_ref()
                .is_some_and(|attempt| !attempt.attention_required)
            && seven_zip_ready
            && !self.extraction_running;

        ui_theme::section(ui, "Forrás lemezkép", |ui| {
            let Some(attempt) = &latest_attempt else {
                ui.weak("Ehhez a lemezhez még nincs FluxVault acquisition.");
                return;
            };

            ui.horizontal_wrapped(|ui| {
                ui.strong(if attempt.legacy_image {
                    "Legacy kép".to_owned()
                } else {
                    format!("Próbálkozás {:03}", attempt.attempt_number)
                });
                ui.separator();
                ui.label(&attempt.status);
                ui.separator();

                if !attempt.attention_required {
                    ui.colored_label(egui::Color32::from_rgb(70, 200, 120), "TISZTA LEMEZKÉP");
                } else {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 180, 80),
                        format!("{} HIBÁS SZEKTOR", attempt.bad_sectors.len()),
                    );
                }
            });

            ui.monospace(&attempt.image_file);
            ui.monospace(format!("SHA-256: {}", attempt.sha256));

            if !attempt.bad_sectors.is_empty() {
                ui.weak(
                    "A nem tiszta lemezkép automatikus extraction helyett az Adatmentés sorba kerül.",
                );
            }
        });

        ui.add_space(12.0);

        ui_theme::section(ui, "Automatikus extraction", |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        extraction_ready,
                        egui::Button::new(if self.extraction_running {
                            "Extraction folyamatban..."
                        } else {
                            "Tiszta lemezkép kibontása"
                        }),
                    )
                    .clicked()
                {
                    self.start_extraction();
                }

                if !seven_zip_ready {
                    ui.weak("A 7-Zip nem érhető el; ellenőrizze a Beállítások oldalt.");
                } else if self.project.is_none() {
                    ui.weak("Extraction előtt nyisson meg egy projektet.");
                }
            });

            ui.add_space(6.0);
            ui.label(&self.extraction_stage);
            ui.weak(
                "A kibontás először ideiglenes mappába történik. Csak a teljes listing, extraction és hash-leltár sikere után kerül végleges helyre.",
            );

            if let Some(error) = &self.extraction_error {
                ui.add_space(8.0);
                ui.colored_label(egui::Color32::from_rgb(220, 70, 70), "EXTRACTION HIBA");
                ui.monospace(error);
            }

            if let Some(result) = &self.extraction_result {
                ui.add_space(8.0);
                ui.colored_label(
                    egui::Color32::from_rgb(70, 200, 120),
                    if result.reused {
                        "VÁLTOZATLAN EXTRACTION ÚJRA FELHASZNÁLVA"
                    } else {
                        "EXTRACTION KÉSZ"
                    },
                );
                ui.label(format!("Kinyert fájlok: {}", result.file_count));
                ui.label(format!("Összes fájlméret: {} bájt", result.total_bytes));
                ui.monospace(format!("Forrás: {}", result.image_path.display()));
                ui.monospace(format!("Cél: {}", result.output_directory.display()));
                ui.monospace(format!("Listing: {}", result.listing_path.display()));
                ui.monospace(format!("Leltár: {}", result.inventory_path.display()));
                ui.monospace(format!("Forrás SHA-256: {}", result.source_sha256));
            }
        });

        ui.add_space(12.0);

        ui_theme::section(ui, "Biztonság és megőrzés", |ui| {
            ui.label("A forrás fizikai floppyhoz ez a művelet nem fér hozzá.");
            ui.label(
                "A lemezkép olvasása és a projekt Extracted / Logs mappáinak írása engedélyezett.",
            );
            ui.label("Meglévő, eltérő hashű extraction mappa soha nem kerül felülírásra.");
        });
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

    fn conversions_page(&mut self, ui: &mut egui::Ui) {
        ui_theme::page_header(
            ui,
            "Konverziók",
            "Régi Office dokumentumok ellenőrzött DOCX, XLSX, PPTX és PDF átalakítása.",
        );

        ui_theme::section(ui, "Még nem elérhető", |ui| {
            ui.colored_label(
                egui::Color32::from_rgb(220, 180, 80),
                "A konverziós futtatás még nincs implementálva, ezért nincs aktív indítógomb.",
            );
            ui.label(
                "A LibreOffice felismerése és egészségügyi ellenőrzése már a Beállítások oldalon működik.",
            );
        });
    }

    fn audit_page(&mut self, ui: &mut egui::Ui) {
        ui_theme::page_header(
            ui,
            "Audit",
            "Acquisition, extraction, recovery és konverziós bizonyítékok egyesített ellenőrzése.",
        );

        ui_theme::section(ui, "Még nem elérhető", |ui| {
            ui.colored_label(
                egui::Color32::from_rgb(220, 180, 80),
                "Az egyesített auditmotor még nincs implementálva.",
            );
            ui.label(
                "A jelenlegi acquisition metadata, 7-Zip listing, fájlleltár és külsőparancs-napló változatlanul megmarad későbbi auditáláshoz.",
            );
        });
    }

    fn package_page(&mut self, ui: &mut egui::Ui) {
        ui_theme::page_header(
            ui,
            "Ügyfélcsomag",
            "Ellenőrzött customer-delivery mappák, manifestek és ZIP csomagok készítése.",
        );

        ui_theme::section(ui, "Még nem elérhető", |ui| {
            ui.colored_label(
                egui::Color32::from_rgb(220, 180, 80),
                "A csomagépítő még nincs implementálva, ezért nem ír célmappába.",
            );
            ui.label(
                "A későbbi megvalósítás külön célmappát kér majd, és megakadályozza a projekt- vagy forrásfa felülírását.",
            );
        });
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui_theme::page_header(
            ui,
            "Beállítások és eszközök",
            "Külső programok felismerése, verzióellenőrzése és biztonsági állapot.",
        );

        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.tool_check_running,
                    egui::Button::new(if self.tool_check_running {
                        "Ellenőrzés folyamatban..."
                    } else {
                        "Eszközök tesztelése"
                    }),
                )
                .clicked()
            {
                self.start_tool_check();
            }

            ui.weak(
                "A tesztek csak verzióinformációt kérnek le; fizikai adathordozót nem érintenek.",
            );
        });

        ui.add_space(14.0);

        for kind in ToolKind::ALL {
            let status = self
                .tool_statuses
                .iter()
                .find(|status| status.kind == kind)
                .cloned();
            let configured_path = self
                .tool_settings
                .path(kind)
                .map(|path| path.display().to_string());

            ui_theme::section(ui, kind.display_name(), |ui| {
                let Some(status) = status else {
                    ui.weak("Nincs állapotinformáció.");
                    return;
                };

                ui.horizontal_wrapped(|ui| {
                    match status.health {
                        ToolHealth::Ready => {
                            ui.colored_label(egui::Color32::from_rgb(70, 200, 120), "[KESZ]");
                        }
                        ToolHealth::Missing => {
                            ui.colored_label(egui::Color32::from_rgb(220, 180, 80), "[HIANYZIK]");
                        }
                        ToolHealth::Failed => {
                            ui.colored_label(egui::Color32::from_rgb(220, 70, 70), "[HIBA]");
                        }
                        ToolHealth::Checking => {
                            ui.colored_label(egui::Color32::from_rgb(90, 150, 230), "[ELLENORZES]");
                        }
                        ToolHealth::NotChecked => {
                            ui.weak("[NINCS ELLENORIZVE]");
                        }
                    }

                    if let Some(version) = &status.version {
                        ui.separator();
                        ui.strong(version);
                    }
                });

                ui.label(&status.detail);

                if let Some(path) = &status.executable {
                    ui.monospace(path.display().to_string());
                } else if let Some(path) = &configured_path {
                    ui.monospace(path);
                }

                ui.add_space(6.0);

                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.tool_check_running,
                            egui::Button::new("Fájl kiválasztása..."),
                        )
                        .clicked()
                    {
                        self.select_tool_path(kind);
                    }

                    if ui
                        .add_enabled(
                            configured_path.is_some() && !self.tool_check_running,
                            egui::Button::new("Automatikus felismerés"),
                        )
                        .clicked()
                    {
                        self.clear_tool_path(kind);
                    }
                });

                if let Some(audit) = &status.audit {
                    egui::CollapsingHeader::new("Legutóbbi parancs részletei")
                        .id_salt(format!("tool_audit_{kind:?}"))
                        .show(ui, |ui| {
                            ui.monospace(format!(
                                "{} {}",
                                audit.executable.display(),
                                audit.arguments.join(" ")
                            ));
                            ui.label(format!(
                                "Kilépési kód: {:?} | Időtartam: {} ms",
                                audit.exit_code, audit.duration_ms
                            ));

                            if !audit.stdout.is_empty() {
                                ui.weak("stdout");
                                ui.monospace(&audit.stdout);
                            }

                            if !audit.stderr.is_empty() {
                                ui.weak("stderr");
                                ui.monospace(&audit.stderr);
                            }
                        });
                }
            });

            ui.add_space(10.0);
        }

        ui_theme::section(ui, "Biztonsági szabályok", |ui| {
            ui.label("Fizikai floppy írás: TILTOTT");
            ui.label("Greaseweazle írás: TILTOTT");
            ui.label("Forrás média hozzáférés: CSAK OLVASHATÓ");
            ui.weak("A Greaseweazle ellenőrzés kizárólag a gw.exe --version parancsot használja.");
        });
    }

    pub(super) fn current_page(&mut self, ui: &mut egui::Ui) {
        match self.page {
            Page::Project => self.project_page(ui),
            Page::Acquire => self.acquire_page(ui),
            Page::Recovery => self.recovery_page(ui),
            Page::Files => self.files_page(ui),
            Page::Conversions => self.conversions_page(ui),
            Page::Audit => self.audit_page(ui),
            Page::Reports => self.reports_page(ui),
            Page::Package => self.package_page(ui),
            Page::Settings => self.settings_page(ui),
        }
    }
}
