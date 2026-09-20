use eframe::egui;

use crate::safety::{MediaSafetyPolicy, SourceMediaAccess};

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
    status: String,
    operator_log: Vec<String>,
}

impl FluxVaultApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self {
            page: Page::Project,
            project_path: None,
            active_source: None,
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

        ui.horizontal(|ui| {
            if read_only {
                ui.colored_label(
                    egui::Color32::from_rgb(70, 200, 120),
                    egui::RichText::new("● FORRÁSLEMEZ: CSAK OLVASHATÓ")
                        .strong()
                        .size(15.0),
                );
            } else {
                ui.colored_label(
                    egui::Color32::RED,
                    egui::RichText::new("● VESZÉLY: ÍRÁSI HOZZÁFÉRÉS ENGEDÉLYEZVE")
                        .strong()
                        .size(15.0),
                );
            }

            ui.separator();

            ui.weak("A FluxVault soha nem írhat az ügyfél eredeti floppy lemezére.");
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

            ui.label("✓ eframe / egui GUI");
            ui.label("✓ Windows DPI manifest");
            ui.label("✓ Központi read-only biztonsági szabály");
            ui.label("✓ Navigáció és operátori napló");
            ui.label("○ Projektkezelés");
            ui.label("○ Fizikai floppy meghajtó felismerése");
            ui.label("○ Nyers, read-only lemezbeolvasás");
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

            match &self.active_source {
                Some(source) => {
                    ui.label(format!("Kiválasztva: {source}"));
                }
                None => {
                    ui.weak("Még nincs fizikai floppy meghajtó kiválasztva.");
                }
            }

            ui.add_space(8.0);

            ui.add_enabled(false, egui::Button::new("Meghajtók frissítése"));

            ui.add_enabled(false, egui::Button::new("Beolvasás indítása"));

            ui.add_space(8.0);

            ui.weak(
                "A vezérlők addig tiltva maradnak, amíg a read-only \
                 Windows floppy backend el nem készül.",
            );
        });

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(
                egui::RichText::new("Tervezett élő lemeztérkép")
                    .strong()
                    .size(16.0),
            );

            ui.add_space(6.0);

            ui.label(
                "A szektorok állapota itt fog élőben megjelenni: \
                 olvasatlan / jó / retry után jó / hibás.",
            );
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

            ui.label("• Összesítő dashboard");
            ui.label("• Feldolgozott floppy lemezek száma");
            ui.label("• Hibátlan / részleges / sikertelen beolvasások");
            ui.label("• Hibás és újrapróbált szektorok statisztikája");
            ui.label("• Visszaállított fájlok száma és mérete");
            ui.label("• Adatmentési módszerek megoszlása");
            ui.label("• Konverziós eredmények");
            ui.label("• Lemezenkénti részletes munkalap");
            ui.label("• Szűrhető és színezett állapotok");
            ui.label("• Grafikonok és összesített statisztikák");

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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(6.0);

            self.safety_banner(ui);

            ui.add_space(6.0);
        });

        egui::SidePanel::left("navigation")
            .resizable(false)
            .default_width(220.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                self.navigation(ui);
            });

        egui::TopBottomPanel::bottom("status_panel")
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label("Állapot:");
                    ui.strong(&self.status);
                });

                self.operator_log(ui);

                ui.add_space(4.0);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);

            self.current_page(ui);
        });
    }
}
