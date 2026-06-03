use crate::agents::{Arpsci, CatppuccinTheme};
use crate::ui::ArpsciUiState;
use eframe::egui;
use std::sync::atomic::Ordering;

impl Arpsci {
    pub(super) fn render_ollama_settings_widgets(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        ui_state: &mut ArpsciUiState,
    ) {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("Ollama Settings").strong().size(16.0));
            ui.separator();
            let models = ui_state.ollama.models.lock().unwrap().clone();
            if self.selected_ollama_model.is_empty()
                && let Some(first) = models.first()
            {
                self.selected_ollama_model = first.clone();
            }
            if ui_state.ollama.model_selection_draft.trim().is_empty() {
                ui_state.ollama.model_selection_draft = self.selected_ollama_model.clone();
            }

            ui.horizontal(|ui| {
                ui.label("API host / URL:");
                ui.add(egui::TextEdit::singleline(&mut self.ollama_host).desired_width(300.0));
            });

            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label("Global Ollama Model:");
                let mut draft = ui_state.ollama.model_selection_draft.clone();
                egui::ComboBox::from_id_salt("ollama_model_selector_global")
                    .selected_text(if draft.is_empty() {
                        "Select model".to_string()
                    } else {
                        draft.clone()
                    })
                    .show_ui(ui, |ui| {
                        for model in &models {
                            ui.selectable_value(&mut draft, model.clone(), model);
                        }
                    });

                // Combo selection updates the app-level global model immediately.
                if draft != ui_state.ollama.model_selection_draft {
                    ui_state.ollama.model_selection_draft = draft.clone();
                    self.selected_ollama_model = draft;
                }

                if ui.button("Set").clicked() {
                    let selected = ui_state.ollama.model_selection_draft.trim().to_string();
                    self.selected_ollama_model = selected.clone();
                    // Update process env so downstream code reading OLLAMA_MODEL sees the same global model.
                    unsafe {
                        std::env::set_var("OLLAMA_MODEL", selected.clone());
                    }
                    if selected.is_empty() {
                        ui_state.ollama.model_set_status =
                            "Global model cleared (using fallback).".to_string();
                    } else {
                        ui_state.ollama.model_set_status =
                            format!("Global model set: {}", selected);
                    }
                }

                let loading = *ui_state.ollama.models_loading.lock().unwrap();
                if ui
                    .button(if loading { "Loading" } else { "Refresh" })
                    .clicked()
                    && !loading
                {
                    *ui_state.ollama.models_loading.lock().unwrap() = true;
                    let models_arc = ui_state.ollama.models.clone();
                    let loading_arc = ui_state.ollama.models_loading.clone();
                    let ctx = ctx.clone();
                    let handle = self.rt_handle.clone();
                    let ollama_host = self.ollama_host.clone();
                    handle.spawn(async move {
                        let models = crate::ollama::fetch_ollama_models(&ollama_host)
                            .await
                            .unwrap_or_default();
                        *models_arc.lock().unwrap() = models;
                        *loading_arc.lock().unwrap() = false;
                        ctx.request_repaint();
                    });
                }
            });
            ui.label(
                egui::RichText::new("This model is used globally by agent and sidecar inference.")
                    .small()
                    .weak(),
            );
            if !ui_state.ollama.model_set_status.is_empty() {
                ui.label(egui::RichText::new(&ui_state.ollama.model_set_status).small().weak());
            }

            ui.add_space(10.0);
            ui.label(egui::RichText::new("Ollama Test").strong().size(16.0));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Using global model:");
                ui.label(
                    if self.selected_ollama_model.trim().is_empty() {
                        "(none selected)".to_string()
                    } else {
                        self.selected_ollama_model.clone()
                    },
                );
            });
            ui.add_space(5.0);

            ui.horizontal(|ui| {
                ui.label("Chat HTTP Endpoint:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.http_endpoint).desired_width(260.0),
                );
            });
            ui.add_space(5.0);

            ui.horizontal(|ui| {
                if ui.button("Test API").clicked() {
                    *ui_state.ollama.test_status.lock().unwrap() = "Pinging Ollama...".to_string();
                    ui_state.ollama.test_running.store(true, Ordering::Relaxed);
                    let ctx = ctx.clone();
                    let handle = self.rt_handle.clone();
                    let model = self.selected_ollama_model.clone();
                    let ollama_host = self.ollama_host.clone();
                    let app_state = self.app_state.clone();
                    let test_status = ui_state.ollama.test_status.clone();
                    let test_running = ui_state.ollama.test_running.clone();
                    handle.spawn(async move {
                        match crate::ollama::test_ollama(
                            ollama_host.as_str(),
                            if model.trim().is_empty() {
                                None
                            } else {
                                Some(model.as_str())
                            },
                            app_state,
                        )
                        .await
                        {
                            Ok(_) => {
                                *test_status.lock().unwrap() =
                                    "Ollama API test succeeded.".to_string();
                            }
                            Err(e) => {
                                *test_status.lock().unwrap() =
                                    format!("Ollama API test failed: {e}");
                            }
                        }
                        test_running.store(false, Ordering::Relaxed);
                        ctx.request_repaint();
                    });
                }
            });

            if ui_state.ollama.test_running.load(Ordering::Relaxed) {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Testing Ollama API...");
                });
            }

            let test_status = ui_state.ollama.test_status.lock().unwrap().clone();
            if !test_status.is_empty() {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(test_status).small().weak());
            }
        });
    }

    pub(super) fn render_reproducibility_settings_widgets(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("Air-gap Policy").strong().size(16.0));
            ui.separator();

            let mut policy_changed = false;
            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut self.air_gap_enabled,
                        "Enable air-gap mode (block non-loopback HTTP)",
                    )
                    .changed()
                {
                    policy_changed = true;
                }
            });
            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut self.allow_local_ollama,
                        "Allow local Ollama (127.0.0.1 / ::1 / localhost)",
                    )
                    .changed()
                {
                    policy_changed = true;
                }
            });

            if self.air_gap_enabled {
                ui.label(
                    egui::RichText::new(
                        "Verification: outbound HTTP is blocked unless target is loopback; blocked attempts are written to the run ledger as transport.http_blocked.",
                    )
                    .small(),
                );
                if self.allow_local_ollama {
                    ui.label(
                        egui::RichText::new("Allowed: loopback Ollama + local file I/O.")
                            .small()
                            .weak(),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(
                            "Allowed: local file I/O only (Ollama requests are blocked).",
                        )
                        .small()
                        .weak(),
                    );
                }
            } else {
                ui.label(
                    egui::RichText::new("Verification: outbound HTTP is currently enabled.")
                        .small()
                        .weak(),
                );
            }

            if policy_changed {
                self.sync_http_policy();
            }

            ui.add_space(10.0);
            ui.label(egui::RichText::new("Timing and Metrics").strong().size(16.0));
            ui.separator();
            let mut metrics_changed = false;
            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut self.metrics_config.enabled,
                        "Enable Ollama timing metrics (JSONL)",
                    )
                    .changed()
                {
                    metrics_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Metrics file:");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.metrics_config.metrics_file)
                            .desired_width(320.0),
                    )
                    .changed()
                {
                    metrics_changed = true;
                }
            });
            ui.label(
                egui::RichText::new(
                    "Default path is metrics/timings.jsonl and the file is intended for offline research analysis.",
                )
                .small()
                .weak(),
            );

            if metrics_changed {
                self.refresh_metrics_sink();
            }

            ui.add_space(10.0);
            ui.label(egui::RichText::new("Theme").strong().size(16.0));
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Catppuccin Theme:");
                egui::ComboBox::from_id_salt("catppuccin_theme_selector")
                    .selected_text(match self.catppuccin_theme {
                        CatppuccinTheme::Latte => "Latte",
                        CatppuccinTheme::Frappe => "Frappe",
                        CatppuccinTheme::Macchiato => "Macchiato",
                        CatppuccinTheme::Mocha => "Mocha",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.catppuccin_theme,
                            CatppuccinTheme::Latte,
                            "Latte",
                        );
                        ui.selectable_value(
                            &mut self.catppuccin_theme,
                            CatppuccinTheme::Frappe,
                            "Frappe",
                        );
                        ui.selectable_value(
                            &mut self.catppuccin_theme,
                            CatppuccinTheme::Macchiato,
                            "Macchiato",
                        );
                        ui.selectable_value(
                            &mut self.catppuccin_theme,
                            CatppuccinTheme::Mocha,
                            "Mocha",
                        );
                    });
            });
            ui.label(
                egui::RichText::new("Global UI palette for the app shell.")
                    .small()
                    .weak(),
            );
        });
    }
}
