use crate::app_model::Humai;
use humai_core::sync_ext::MutexExt;
use humai_core::vault::MasterVault;
use eframe::egui;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

mod panel_render;
mod settings_panel;
mod overview_chat;
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
pub(crate) struct HumaiUiState {
	pub(crate) ollama: OllamaUiState,
	/// Resizable sidebar/chat layout for the Overview tab. `None` until first
	/// rendered, then built once and persisted here (it holds the user's
	/// drag-to-resize position) — see `overview_chat::panel::build_overview_tiles_tree`.
	pub(crate) overview_tiles: Option<egui_tiles::Tree<overview_chat::panel::OverviewPane>>,
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
		*phosphor_fonts_installed = true;
	}
}

fn refresh_ollama_models_on_startup(
	humai: &Humai,
	ui_state: &mut HumaiUiState,
	ctx: &egui::Context,
) {
	if ui_state.ollama.models.lock_recover().is_empty()
		&& !*ui_state.ollama.models_loading.lock_recover()
	{
		*ui_state.ollama.models_loading.lock_recover() = true;
		let models_arc = ui_state.ollama.models.clone();
		let loading_arc = ui_state.ollama.models_loading.clone();
		let ctx = ctx.clone();
		let handle = humai.rt_handle.clone();
		let ollama_host = humai.ollama_host.clone();
		handle.spawn(async move {
			let models = humai_core::ollama::fetch_ollama_models(&ollama_host)
				.await
				.unwrap_or_default();
			*models_arc.lock_recover() = models;
			*loading_arc.lock_recover() = false;
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

		// The web dashboard (humai-web) mirrors this flag on every request and
		// withholds content while locked — the vault itself lives here on
		// HumaiApp, not on Humai, so this per-frame sync is what lets the
		// dashboard find out the lock state changed at all.
		self.humai
			.vault_unlocked
			.store(self.vault.is_unlocked(), std::sync::atomic::Ordering::Relaxed);

		if !self.vault.is_unlocked() {
			egui::CentralPanel::default().show(ctx, |ui| {
				ui.vertical_centered(|ui| {
					ui.add_space(40.0);
					vault_panel::show_unlock_ui(&mut self.vault, ui);
				});
			});
			return;
		}

		egui::TopBottomPanel::top("master_vault_lock_bar").show(ctx, |ui| {
			if vault_panel::show_lock_bar(ui) {
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
		let _ = ctx.run(Default::default(), |ctx| {
			has_emoji = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::default(), SAMPLE_EMOJI));
		});
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
		let _ = ctx.run(Default::default(), |ctx| {
			prepare_shell(
				ctx,
				crate::app_model::CatppuccinTheme::Latte,
				&mut last_theme,
				&mut installed,
			);
		});
		assert!(installed);

		let mut has_emoji = false;
		let _ = ctx.run(Default::default(), |ctx| {
			has_emoji = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::default(), SAMPLE_EMOJI));
		});
		assert!(
			has_emoji,
			"bundled NotoEmoji fallback should cover common emoji on every platform"
		);
	}
}
