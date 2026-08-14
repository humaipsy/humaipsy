use eframe::egui;

use crate::app_model::{Humai, MainPanelTab};
use crate::ui::HumaiUiState;

impl Humai {
    /// Returns true if the user clicked "Lock" this frame — the vault itself
    /// lives on `HumaiApp`, not `Humai`, so the caller (`ui/mod.rs`) is the one
    /// that actually locks it.
    pub(crate) fn render_main_panel(&mut self, ui: &mut egui::Ui, ui_state: &mut HumaiUiState) -> bool {
        let panel_border_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let panel = egui::Frame::default()
            .fill(ui.visuals().panel_fill)
            .stroke(egui::Stroke::new(1.0_f32, panel_border_color))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::same(6));

        let panel_height = ui.available_height().max(120.0);
        let mut lock_clicked = false;

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), panel_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                panel.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                self.active_tab == MainPanelTab::Overview,
                                "Overview",
                            )
                            .clicked()
                        {
                            self.active_tab = MainPanelTab::Overview;
                        }
                        if ui
                            .selectable_label(
                                self.active_tab == MainPanelTab::Design,
                                "Design",
                            )
                            .clicked()
                        {
                            self.active_tab = MainPanelTab::Design;
                        }
                        if ui
                            .selectable_label(
                                self.active_tab == MainPanelTab::Settings,
                                "Settings",
                            )
                            .clicked()
                        {
                            self.active_tab = MainPanelTab::Settings;
                        }
                        if self.air_gap_enabled {
                            ui.add_space(8.0);
                            let mut badge = "Air-gap mode: outbound HTTP disabled";
                            if !self.allow_local_ollama {
                                badge = "Air-gap mode: outbound HTTP + Ollama disabled";
                            }
                            ui.label(egui::RichText::new(badge).small().strong().color(
                                ui.visuals().warn_fg_color,
                            ))
                            .on_hover_text(
                                "Policy is enforced at runtime: non-loopback HTTP is blocked and logged.",
                            );
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Lock").clicked() {
                                lock_clicked = true;
                            }
                        });
                    });
                    ui.separator();

                    match self.active_tab {
                        MainPanelTab::Overview => {
                            crate::ui::overview_chat::render_overview_panel(self, ui, ui_state);
                        }
                        MainPanelTab::Design => {
                            crate::ui::overview_chat::render_design_panel(self, ui, ui_state);
                        }
                        MainPanelTab::Settings => {
                            let ctx = ui.ctx().clone();
                            self.render_settings_widgets(ui, &ctx, ui_state);
                        }
                    }
                    crate::ui::overview_chat::render_experiment_modals(
                        ui.ctx(),
                        &mut ui_state.overview_chat,
                    );
                });
            },
        );

        lock_clicked
    }
}
