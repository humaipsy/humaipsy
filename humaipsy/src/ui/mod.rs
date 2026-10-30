use crate::app_model::HumaiPsy;
use humaipsy_core::sync_ext::MutexExt;
use humaipsy_core::vault::MasterVault;
use eframe::egui;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

mod panel_render;
mod settings_panel;
mod overview_chat;
mod theme;
mod vault_panel;

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

#[derive(Default)]
pub(crate) struct HumaiPsyUiState {
	pub(crate) ollama: OllamaUiState,
	/// Experiment/chat orchestration state for the Overview and Design tabs
	/// (current trial, chat rooms, participant ID, modal state). Owned here
	/// instead of process-global statics — see `overview_chat::panel::OverviewChatUiState`.
	pub(crate) overview_chat: overview_chat::panel::OverviewChatUiState,
}

/// Bundled so emoji in Ollama responses render identically on Windows and Linux,
/// instead of depending on OS-specific font files that may not be installed.
/// egui's text renderer only supports single-color glyph outlines (no COLR/CBDT
/// color tables), so this is the monochrome "Noto Emoji" family, not "Noto Color
/// Emoji" — glyphs are tinted with the surrounding text color rather than shown
/// in full color, but every standard emoji renders as its actual pictograph
/// instead of a missing-glyph box. License: SIL OFL 1.1, see assets/NotoEmoji-OFL.txt.
const EMOJI_FONT_BYTES: &[u8] = include_bytes!("../../assets/NotoEmoji-Regular.ttf");

fn prepare_shell(
	ctx: &egui::Context,
	selected_theme: crate::app_model::CatppuccinTheme,
	last_applied_theme: &mut Option<crate::app_model::CatppuccinTheme>,
	custom_fonts_installed: &mut bool,
) {
	if *last_applied_theme != Some(selected_theme) {
		theme::apply(ctx, selected_theme);
		*last_applied_theme = Some(selected_theme);
	}
	if !*custom_fonts_installed {
		let mut fonts = ctx.fonts(|f| f.definitions().clone());

		let emoji_font_name = "emoji-fallback".to_string();
		fonts.font_data.insert(
			emoji_font_name.clone(),
			egui::FontData::from_static(EMOJI_FONT_BYTES).into(),
		);
		if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
			proportional.push(emoji_font_name.clone());
		}
		if let Some(monospace) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
			monospace.push(emoji_font_name);
		}
		ctx.set_fonts(fonts);
		*custom_fonts_installed = true;
	}
}

fn refresh_ollama_models_on_startup(
	humaipsy: &HumaiPsy,
	ui_state: &mut HumaiPsyUiState,
	ctx: &egui::Context,
) {
	if ui_state.ollama.models.lock_recover().is_empty()
		&& !*ui_state.ollama.models_loading.lock_recover()
	{
		*ui_state.ollama.models_loading.lock_recover() = true;
		let models_arc = ui_state.ollama.models.clone();
		let loading_arc = ui_state.ollama.models_loading.clone();
		let ctx = ctx.clone();
		let handle = humaipsy.rt_handle.clone();
		let ollama_host = humaipsy.ollama_host.clone();
		handle.spawn(async move {
			let models = humaipsy_core::ollama::fetch_ollama_models(&ollama_host)
				.await
				.unwrap_or_default();
			*models_arc.lock_recover() = models;
			*loading_arc.lock_recover() = false;
			ctx.request_repaint();
		});
	}
}

pub struct HumaiPsyApp {
	vault: MasterVault,
	humaipsy: HumaiPsy,
	ui_state: HumaiPsyUiState,
	last_applied_theme: Option<crate::app_model::CatppuccinTheme>,
	custom_fonts_installed: bool,
}

impl HumaiPsyApp {
	pub fn new(rt_handle: Handle) -> Self {
		Self {
			vault: MasterVault::new(),
			humaipsy: HumaiPsy::new(rt_handle),
			ui_state: HumaiPsyUiState::default(),
			last_applied_theme: None,
			custom_fonts_installed: false,
		}
	}
}

impl eframe::App for HumaiPsyApp {
	fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		prepare_shell(
			&ctx,
			self.humaipsy.catppuccin_theme,
			&mut self.last_applied_theme,
			&mut self.custom_fonts_installed,
		);

		// The web dashboard (humaipsy-web) mirrors this flag on every request and
		// withholds content while locked — the vault itself lives here on
		// HumaiPsyApp, not on HumaiPsy, so this per-frame sync is what lets the
		// dashboard find out the lock state changed at all.
		self.humaipsy
			.vault_unlocked
			.store(self.vault.is_unlocked(), std::sync::atomic::Ordering::Relaxed);

		if !self.vault.is_unlocked() {
			egui::CentralPanel::default().show(ui, |ui| {
				ui.vertical_centered(|ui| {
					ui.add_space(40.0);
					vault_panel::show_unlock_ui(&mut self.vault, ui);
				});
			});
			return;
		}

		refresh_ollama_models_on_startup(&self.humaipsy, &mut self.ui_state, &ctx);
		egui::CentralPanel::default().show(ui, |ui| {
			ui.vertical(|ui| {
				ui.set_min_height(ui.available_height());
				if self.humaipsy.render_main_panel(ui, &mut self.ui_state) {
					self.vault.lock();
				}
			});
		});
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const SAMPLE_EMOJI: &str = "😀🚀❤👍🎉🤔";

	#[test]
	fn default_egui_fonts_are_missing_emoji_glyphs() {
		// Establishes the baseline this fix addresses: egui's stock fonts (used
		// before `prepare_shell` runs) don't cover Unicode emoji at all, so
		// without our bundled fallback these would render as missing-glyph boxes.
		let ctx = egui::Context::default();
		let mut has_emoji = false;
		let mut output = ctx.run_ui(Default::default(), |ui| {
			has_emoji = ui.ctx().fonts_mut(|f| f.has_glyphs(&egui::FontId::default(), SAMPLE_EMOJI));
		});
		output.textures_delta.clear();
		assert!(!has_emoji, "test fixture assumption changed: stock egui fonts now cover emoji");
	}

	#[test]
	fn bundled_emoji_font_covers_common_emoji_after_prepare_shell() {
		let ctx = egui::Context::default();
		let mut last_theme = None;
		let mut installed = false;

		// `Context::set_fonts` (called inside `prepare_shell`) only queues the new
		// font definitions; they're applied at the *next* pass's `begin_pass`. So
		// this takes two passes: one to install the fonts, one to observe them.
		let mut output = ctx.run_ui(Default::default(), |ui| {
			prepare_shell(
				ui.ctx(),
				crate::app_model::CatppuccinTheme::Latte,
				&mut last_theme,
				&mut installed,
			);
		});
		output.textures_delta.clear();
		assert!(installed);

		let mut has_emoji = false;
		let mut output = ctx.run_ui(Default::default(), |ui| {
			has_emoji = ui.ctx().fonts_mut(|f| f.has_glyphs(&egui::FontId::default(), SAMPLE_EMOJI));
		});
		output.textures_delta.clear();
		assert!(
			has_emoji,
			"bundled NotoEmoji fallback should cover common emoji on every platform"
		);
	}
}
