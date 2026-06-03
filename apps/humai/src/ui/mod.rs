use crate::app_model::Humai;
use crate::vault::MasterVault;
use eframe::egui;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

mod panel_render;
mod settings_panel;
mod overview_chat;

pub(crate) struct OllamaUiState {
	pub(crate) models: Arc<Mutex<Vec<String>>>,
	pub(crate) models_loading: Arc<Mutex<bool>>,
	pub(crate) model_selection_draft: String,
	pub(crate) model_set_status: String,
	pub(crate) test_status: Arc<Mutex<String>>,
	pub(crate) test_running: Arc<AtomicBool>,
}

impl Default for OllamaUiState {
	fn default() -> Self {
		Self {
			models: Arc::new(Mutex::new(Vec::new())),
			models_loading: Arc::new(Mutex::new(false)),
			model_selection_draft: String::new(),
			model_set_status: String::new(),
			test_status: Arc::new(Mutex::new(String::new())),
			test_running: Arc::new(AtomicBool::new(false)),
		}
	}
}

pub(crate) struct HumaiUiState {
	pub(crate) ollama: OllamaUiState,
}

impl Default for HumaiUiState {
	fn default() -> Self {
		Self {
			ollama: OllamaUiState::default(),
		}
	}
}

fn load_first_font_bytes(candidates: &[&str]) -> Option<Vec<u8>> {
	for path in candidates {
		if let Ok(bytes) = std::fs::read(path) {
			return Some(bytes);
		}
	}
	None
}

fn prepare_shell(
	ctx: &egui::Context,
	selected_theme: crate::app_model::CatppuccinTheme,
	last_applied_theme: &mut Option<crate::app_model::CatppuccinTheme>,
	phosphor_fonts_installed: &mut bool,
) {
	if *last_applied_theme != Some(selected_theme) {
		match selected_theme {
			crate::app_model::CatppuccinTheme::Latte => {
				catppuccin_egui::set_theme(ctx, catppuccin_egui::LATTE)
			}
			crate::app_model::CatppuccinTheme::Frappe => {
				catppuccin_egui::set_theme(ctx, catppuccin_egui::FRAPPE)
			}
			crate::app_model::CatppuccinTheme::Macchiato => {
				catppuccin_egui::set_theme(ctx, catppuccin_egui::MACCHIATO)
			}
			crate::app_model::CatppuccinTheme::Mocha => {
				catppuccin_egui::set_theme(ctx, catppuccin_egui::MOCHA)
			}
		}
		*last_applied_theme = Some(selected_theme);
	}
	if !*phosphor_fonts_installed {
		let mut fonts = ctx.fonts(|f| f.definitions().clone());
		egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

		let mut appended_fallbacks: Vec<String> = Vec::new();
		let emoji_color_candidates = [
			"/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
			"/usr/share/fonts/noto/NotoColorEmoji.ttf",
			"/usr/share/fonts/opentype/noto/NotoColorEmoji.ttf",
			"/usr/local/share/fonts/NotoColorEmoji.ttf",
		];
		if let Some(bytes) = load_first_font_bytes(&emoji_color_candidates) {
			let name = "emoji-fallback-color".to_string();
			fonts
				.font_data
				.insert(name.clone(), egui::FontData::from_owned(bytes).into());
			appended_fallbacks.push(name);
		}

		let emoji_text_candidates = [
			"/usr/share/fonts/truetype/noto/NotoEmoji-Regular.ttf",
			"/usr/share/fonts/truetype/noto/NotoEmoji-VariableFont_wght.ttf",
			"/usr/share/fonts/noto/NotoEmoji-Regular.ttf",
			"/usr/local/share/fonts/NotoEmoji-Regular.ttf",
		];
		if let Some(bytes) = load_first_font_bytes(&emoji_text_candidates) {
			let name = "emoji-fallback-text".to_string();
			fonts
				.font_data
				.insert(name.clone(), egui::FontData::from_owned(bytes).into());
			appended_fallbacks.push(name);
		}

		let symbol_candidates = [
			"/usr/share/fonts/truetype/ancient-scripts/Symbola_hint.ttf",
			"/usr/share/fonts/truetype/ancient-scripts/Symbola.ttf",
			"/usr/share/fonts/truetype/unifont/unifont.ttf",
			"/usr/share/fonts/truetype/freefont/FreeSans.ttf",
		];
		if let Some(bytes) = load_first_font_bytes(&symbol_candidates) {
			let name = "emoji-fallback-symbols".to_string();
			fonts
				.font_data
				.insert(name.clone(), egui::FontData::from_owned(bytes).into());
			appended_fallbacks.push(name);
		}

		if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
			proportional.extend(appended_fallbacks.iter().cloned());
		}
		if let Some(monospace) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
			monospace.extend(appended_fallbacks);
		}
		ctx.set_fonts(fonts);
		*phosphor_fonts_installed = true;
	}
}

fn refresh_ollama_models_on_startup(
	humai: &Humai,
	ui_state: &mut HumaiUiState,
	ctx: &egui::Context,
) {
	if ui_state.ollama.models.lock().unwrap().is_empty()
		&& !*ui_state.ollama.models_loading.lock().unwrap()
	{
		*ui_state.ollama.models_loading.lock().unwrap() = true;
		let models_arc = ui_state.ollama.models.clone();
		let loading_arc = ui_state.ollama.models_loading.clone();
		let ctx = ctx.clone();
		let handle = humai.rt_handle.clone();
		let ollama_host = humai.ollama_host.clone();
		handle.spawn(async move {
			let models = crate::ollama::fetch_ollama_models(&ollama_host)
				.await
				.unwrap_or_default();
			*models_arc.lock().unwrap() = models;
			*loading_arc.lock().unwrap() = false;
			ctx.request_repaint();
		});
	}
}

pub struct HumaiApp {
	vault: MasterVault,
	humai: Humai,
	ui_state: HumaiUiState,
	last_applied_theme: Option<crate::app_model::CatppuccinTheme>,
	phosphor_fonts_installed: bool,
}

impl HumaiApp {
	pub fn new(rt_handle: Handle) -> Self {
		Self {
			vault: MasterVault::new(),
			humai: Humai::new(rt_handle),
			ui_state: HumaiUiState::default(),
			last_applied_theme: None,
			phosphor_fonts_installed: false,
		}
	}
}

impl eframe::App for HumaiApp {
	fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
		prepare_shell(
			ctx,
			self.humai.catppuccin_theme,
			&mut self.last_applied_theme,
			&mut self.phosphor_fonts_installed,
		);

		if !self.vault.is_unlocked() {
			egui::CentralPanel::default().show(ctx, |ui| {
				ui.vertical_centered(|ui| {
					ui.add_space(40.0);
					self.vault.show_unlock_ui(ui);
				});
			});
			return;
		}

		egui::TopBottomPanel::top("master_vault_lock_bar").show(ctx, |ui| {
			if self.vault.show_lock_bar(ui) {
				self.vault.lock();
			}
		});

		refresh_ollama_models_on_startup(&self.humai, &mut self.ui_state, ctx);
		egui::CentralPanel::default().show(ctx, |ui| {
			ui.vertical(|ui| {
				ui.set_min_height(ui.available_height());
				self.humai.render_main_panel(ui, &mut self.ui_state);
			});
		});
	}
}
