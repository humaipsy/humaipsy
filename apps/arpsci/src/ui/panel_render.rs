use eframe::egui;

use crate::agents::Arpsci;
use crate::agents::nodes_panel::PanelTab;
use crate::ui::ArpsciUiState;

impl Arpsci {
    pub(crate) fn render_nodes_panel(&mut self, ui: &mut egui::Ui, ui_state: &mut ArpsciUiState) {
        // Workspace tab is deprecated in this UI path.
        if self.nodes_panel.active_tab == PanelTab::Agents {
            self.nodes_panel.active_tab = PanelTab::Overview;
        }

        let panel_border_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let panel = egui::Frame::default()
            .fill(ui.visuals().panel_fill)
            .stroke(egui::Stroke::new(1.0, panel_border_color))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::same(6));

        let panel_height = ui.available_height().max(120.0);

        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), panel_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                panel.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(
                                self.nodes_panel.active_tab == PanelTab::Overview,
                                "Overview",
                            )
                            .clicked()
                        {
                            self.nodes_panel.active_tab = PanelTab::Overview;
                        }
                        if ui
                            .selectable_label(
                                self.nodes_panel.active_tab == PanelTab::Ollama,
                                "Ollama",
                            )
                            .clicked()
                        {
                            self.nodes_panel.active_tab = PanelTab::Ollama;
                        }
                        if ui
                            .selectable_label(
                                self.nodes_panel.active_tab == PanelTab::Settings,
                                "Settings",
                            )
                            .clicked()
                        {
                            self.nodes_panel.active_tab = PanelTab::Settings;
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
                    });
                    ui.separator();

                    match self.nodes_panel.active_tab {
                        PanelTab::Overview | PanelTab::Agents => {
                            crate::ui::overview_chat::render_overview_panel(self, ui);
                        }
                        PanelTab::Ollama => {
                            let ctx = ui.ctx().clone();
                            self.render_ollama_settings_widgets(ui, &ctx, ui_state);
                        }
                        PanelTab::Settings => {
                            self.render_reproducibility_settings_widgets(ui);
                        }
                    }
                });
            },
        );
    }
}
