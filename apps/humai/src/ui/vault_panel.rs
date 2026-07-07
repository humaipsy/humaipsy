//! Rendering for the master-password gate. Kept separate from [`crate::vault`] so the
//! crypto/state module has no dependency on the UI framework — see [`crate::vault::MasterVault`]
//! for the logic this renders.

use eframe::egui::{self, TextEdit};
use secrecy::{ExposeSecret, SecretString};

use crate::vault::MasterVault;

fn sync_password_field(ui: &mut egui::Ui, value: &mut SecretString, hint: &str) -> egui::Response {
	let mut plain: String = value.expose_secret().to_owned();
	let resp = ui.add(
		TextEdit::singleline(&mut plain)
			.password(true)
			.hint_text(hint),
	);
	*value = SecretString::new(plain.into_boxed_str());
	resp
}

/// Full-screen unlock UI (CentralPanel).
pub(crate) fn show_unlock_ui(vault: &mut MasterVault, ui: &mut egui::Ui) {
	ui.heading("Humai Vault");

	if vault.is_skip_vault() {
		ui.label("Vault gate is disabled in debug mode.");
		ui.label("Set HUMAI_ENABLE_PASSWORD_LOCK=1 to test lock behavior during development.");
		return;
	}

	if vault.needs_password_setup() {
		ui.label("Create a master password for this installation.");
		ui.add_space(6.0);
		ui.label("Password:");
		let password_resp =
			sync_password_field(ui, vault.master_input_mut(), "••••••••••••••••");
		ui.label("Confirm password:");
		let confirm_resp =
			sync_password_field(ui, vault.confirm_input_mut(), "••••••••••••••••");

		if ui.button("Create password").clicked()
			|| ((password_resp.lost_focus() || confirm_resp.lost_focus())
				&& ui.input(|i| i.key_pressed(egui::Key::Enter)))
		{
			vault.try_setup_password();
		}

		if !vault.status().is_empty() {
			ui.label(egui::RichText::new(vault.status()).weak());
		}
		return;
	}

	ui.label("Enter Password:");
	let resp = sync_password_field(ui, vault.master_input_mut(), "••••••••••••••••");

	if ui.button("Unlock").clicked()
		|| (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
	{
		vault.try_unlock();
	}

	if !vault.status().is_empty() {
		ui.label(egui::RichText::new(vault.status()).weak());
	}
}

/// Thin top strip with Lock; call from a `TopBottomPanel::top`. Returns true if user locked.
pub(crate) fn show_lock_bar(ui: &mut egui::Ui) -> bool {
	let mut clicked = false;
	ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
		if ui.button("Lock").clicked() {
			clicked = true;
		}
	});
	clicked
}
