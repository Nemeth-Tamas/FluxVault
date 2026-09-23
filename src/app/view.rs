mod pages;

use std::time::Duration;

use eframe::egui;

use super::{FluxVaultApp, Page};
use crate::{
    imaging::SectorReadState,
    safety::{MediaSafetyPolicy, SourceMediaAccess},
    ui as ui_theme,
};

impl FluxVaultApp {
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

    fn navigation(&mut self, ui: &mut egui::Ui) {
        ui_theme::subsection_label(ui, "MUNKAFOLYAMAT");

        ui.add_space(4.0);

        ui.selectable_value(&mut self.page, Page::Project, Page::Project.title());

        ui.selectable_value(&mut self.page, Page::Acquire, Page::Acquire.title());

        ui.selectable_value(&mut self.page, Page::Recovery, Page::Recovery.title());

        ui.selectable_value(&mut self.page, Page::Files, Page::Files.title());

        ui.selectable_value(&mut self.page, Page::Conversions, Page::Conversions.title());

        ui.add_space(18.0);

        ui_theme::subsection_label(ui, "KIMENET");

        ui.add_space(4.0);

        ui.selectable_value(&mut self.page, Page::Audit, Page::Audit.title());

        ui.selectable_value(&mut self.page, Page::Reports, Page::Reports.title());

        ui.selectable_value(&mut self.page, Page::Package, Page::Package.title());

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

    fn compact_navigation(&mut self, ui: &mut egui::Ui) {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui_theme::subsection_label(ui, "NÉZET");

                    ui.separator();

                    ui.selectable_value(&mut self.page, Page::Project, Page::Project.title());
                    ui.selectable_value(&mut self.page, Page::Acquire, Page::Acquire.title());
                    ui.selectable_value(&mut self.page, Page::Recovery, Page::Recovery.title());
                    ui.selectable_value(&mut self.page, Page::Files, Page::Files.title());
                    ui.selectable_value(
                        &mut self.page,
                        Page::Conversions,
                        Page::Conversions.title(),
                    );
                    ui.selectable_value(&mut self.page, Page::Audit, Page::Audit.title());
                    ui.selectable_value(&mut self.page, Page::Reports, Page::Reports.title());
                    ui.selectable_value(&mut self.page, Page::Package, Page::Package.title());
                    ui.selectable_value(&mut self.page, Page::Settings, Page::Settings.title());
                });
            });
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

    fn page_scroller(&mut self, ui: &mut egui::Ui, compact: bool) {
        let content_width = ui.available_width();

        egui::ScrollArea::vertical()
            .id_salt("main_content_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let horizontal_margin = if compact { 4 } else { 12 };

                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(horizontal_margin, 8))
                    .show(ui, |ui| {
                        ui.set_min_width(
                            (content_width - f32::from(horizontal_margin * 2)).max(180.0),
                        );

                        self.current_page(ui);

                        ui.add_space(12.0);
                    });
            });
    }

    fn operator_log(&self, ui: &mut egui::Ui, max_height: f32) {
        ui.horizontal(|ui| {
            ui.strong("Operátori napló");

            ui.separator();

            ui.weak(format!("{} bejegyzés", self.operator_log.len()));

            if ui.button("Másolás").clicked() {
                ui.ctx().copy_text(self.operator_log.join("\n"));
            }
        });

        ui.add_space(4.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("operator_log_scroll")
                .max_height(max_height)
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
        self.poll_tool_check_events();
        self.poll_extraction_events();
        self.poll_batch_extraction_events();
        self.poll_reconstruction_events();
        self.poll_composite_events();
        self.poll_recovery_backup_events();
        self.poll_manual_recovery_import_events();
        self.poll_manifest_events();
        self.poll_conversion_planning_events();
        self.poll_conversion_events();
        self.start_next_queued_extraction();

        if self.imaging_running
            || self.tool_check_running
            || self.extraction_running
            || self.batch_extraction_running
            || self.reconstruction_running
            || self.manual_recovery_import_running
            || self.conversion_planning_running
            || self.conversion_running
        {
            ui.ctx().request_repaint_after(Duration::from_millis(40));
        }

        let compact = ui_theme::is_compact(ui.available_width());
        let bottom_reserved_height = if compact {
            ui_theme::COMPACT_BOTTOM_RESERVED_HEIGHT
        } else {
            ui_theme::BOTTOM_RESERVED_HEIGHT
        };

        egui::Frame::new()
            .inner_margin(ui_theme::outer_margin(compact))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            self.workspace_header(ui);
                        });

                    ui.add_space(10.0);

                    let content_height =
                        (ui.available_height() - bottom_reserved_height).max(210.0);
                    let total_width = ui.available_width();

                    ui.allocate_ui_with_layout(
                        egui::vec2(total_width, content_height),
                        egui::Layout::top_down(egui::Align::LEFT),
                        |ui| {
                            if compact {
                                self.compact_navigation(ui);
                                ui.add_space(8.0);

                                let page_height = ui.available_height();
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), page_height),
                                    egui::Layout::top_down(egui::Align::LEFT),
                                    |ui| self.page_scroller(ui, true),
                                );
                            } else {
                                ui.horizontal(|ui| {
                                    let navigation_width = ui_theme::NAVIGATION_WIDTH;

                                    ui.allocate_ui_with_layout(
                                        egui::vec2(navigation_width, content_height),
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| {
                                            let navigation_height =
                                                (content_height - 24.0).max(120.0);

                                            egui::Frame::group(ui.style())
                                                .inner_margin(egui::Margin::same(12))
                                                .show(ui, |ui| {
                                                    ui.set_min_width(navigation_width - 24.0);

                                                    egui::ScrollArea::vertical()
                                                        .id_salt("navigation_scroll")
                                                        .auto_shrink([false, false])
                                                        .max_height(navigation_height)
                                                        .show(ui, |ui| {
                                                            ui.set_min_width(
                                                                navigation_width - 24.0,
                                                            );
                                                            self.navigation(ui);
                                                        });
                                                });
                                        },
                                    );

                                    ui.add_space(4.0);

                                    let page_width = ui.available_width();
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(page_width, content_height),
                                        egui::Layout::top_down(egui::Align::LEFT),
                                        |ui| self.page_scroller(ui, false),
                                    );
                                });
                            }
                        },
                    );

                    ui.add_space(8.0);

                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Állapot:");
                                ui.strong(&self.status);

                                ui.separator();

                                ui.strong(format!(
                                    "Aktuális lemez: {:03}",
                                    self.current_disk_number
                                ));

                                if !self.imaging_running {
                                    ui.separator();

                                    if ui
                                        .add_enabled(
                                            self.current_disk_number > 1,
                                            egui::Button::new(format!(
                                                "< ELŐZŐ: {:03}",
                                                self.current_disk_number.saturating_sub(1).max(1)
                                            )),
                                        )
                                        .clicked()
                                    {
                                        self.return_to_previous_disk();
                                    }

                                    if ui
                                        .button(format!(
                                            "KÖVETKEZŐ LEMEZ: {:03} >",
                                            self.current_disk_number.saturating_add(1)
                                        ))
                                        .clicked()
                                    {
                                        self.advance_to_next_disk();
                                    }
                                }
                            });

                            ui.add_space(4.0);

                            self.operator_log(ui, if compact { 76.0 } else { 108.0 });
                        });
                });
            });
    }
}
