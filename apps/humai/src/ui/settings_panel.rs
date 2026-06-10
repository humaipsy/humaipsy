use crate::app_model::{CatppuccinTheme, Humai};
use crate::ui::HumaiUiState;
use eframe::egui;
use std::sync::atomic::Ordering;

impl Humai {
    pub(super) fn render_settings_widgets(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        ui_state: &mut HumaiUiState,
    ) {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("AI Settings").strong().size(16.0));
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
                ui.label("Ollama API host / URL:");
                ui.add(egui::TextEdit::singleline(&mut self.ollama_host).desired_width(300.0));
            });

            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label("Ollama Model:");
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

            if !ui_state.ollama.model_set_status.is_empty() {
                ui.label(egui::RichText::new(&ui_state.ollama.model_set_status).small().weak());
            }

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label("Ollama Inference Test:");
                if ui.button("Ping Ollama").clicked() {
                    *ui_state.ollama.test_status.lock().unwrap() = "Pinging Ollama...".to_string();
                    ui_state.ollama.test_running.store(true, Ordering::Relaxed);
                    let ctx = ctx.clone();
                    let handle = self.rt_handle.clone();
                    let model = self.selected_ollama_model.clone();
                    let ollama_host = self.ollama_host.clone();
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

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width() / 2.0);
                ui.label(egui::RichText::new("Experimental Design Agent").strong().size(16.0));
                ui.separator();
                ui.label(
                    egui::RichText::new(
                        "Define the conditions appended to the global agent.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(6.0);

                ui.label("Cooperative prompt:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.coop_agent_prompt)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                );

                ui.add_space(8.0);
                ui.label("Argumentative prompt:");
                ui.add(
                    egui::TextEdit::multiline(&mut self.argum_agent_prompt)
                        .desired_width(f32::INFINITY)
                        .desired_rows(4),
                );
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);

            ui.label(egui::RichText::new("Experiment Recording").strong().size(16.0));
            ui.separator();
            let mut metrics_changed = false;
            ui.horizontal(|ui| {
                if ui
                    .checkbox(
                        &mut self.metrics_config.enabled,
                        "Enable experiment CSV recording",
                    )
                    .changed()
                {
                    metrics_changed = true;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Output folder:");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.metrics_config.output_dir)
                            .desired_width(320.0),
                    )
                    .changed()
                {
                    metrics_changed = true;
                }
            });
            ui.label(
                egui::RichText::new(
                    "Experiment CSV files are written to this folder (e.g. metrics/Exp001.csv).",
                )
                .small()
                .weak(),
            );

            if metrics_changed {
                self.refresh_metrics_sink();
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(6.0);

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
        });
    }
}
