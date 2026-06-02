//! Master-password gate (Argon2id) and a small encrypted [`Vault`] container.
//!
//! Configure with `ARPSCI_MASTER_HASH` (PHC string) or a file at `runs/.master_hash` (first line).
//! Optional Argon2id tuning via env: `ARPSCI_ARGON2_M_KIB`, `ARPSCI_ARGON2_T`, `ARPSCI_ARGON2_P`.
//! Set `ENABLE_PASSWORD_LOCK` to `false` for instant unlocked startup in development.
//! Set `ARPSCI_SKIP_VAULT=1` to disable the gate (development only).

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordVerifier, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use eframe::egui::{self, TextEdit};
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;

const DEFAULT_HASH_FILE: &str = "runs/.master_hash";
/// Development toggle: keep `false` to boot unlocked; set `true` to enforce the password gate.
pub const ENABLE_PASSWORD_LOCK: bool = false;
const ARGON2_M_KIB_DEFAULT: u32 = 64 * 1024;
const ARGON2_T_DEFAULT: u32 = 3;
const ARGON2_P_MAX: u32 = 4;
const VAULT_BLOB_VERSION: u32 = 1;
const VAULT_KDF_LABEL: &[u8] = b"arp-vault-key-v1";
const VAULT_AAD: &[u8] = b"arp.vault.blob.v1";

fn parse_u32_env(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn default_parallelism() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
        .clamp(1, ARGON2_P_MAX)
}

fn argon2id(params: &VaultKdfParams) -> anyhow::Result<Argon2<'static>> {
    let p = Params::new(
        params.memory_cost_kib,
        params.time_cost,
        params.parallelism,
        Some(32),
    )
    .map_err(|e| anyhow::anyhow!("invalid Argon2 params: {e}"))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, p))
}

pub fn hash_master_password_phc(password: &str, params: VaultKdfParams) -> anyhow::Result<String> {
    use argon2::password_hash::{PasswordHasher, SaltString};

    let salt = SaltString::generate(&mut OsRng);
    let hasher = argon2id(&params)?;
    Ok(hasher
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash master password: {e}"))?
        .to_string())
}

#[derive(Clone, Copy)]
pub struct VaultKdfParams {
    pub memory_cost_kib: u32,
    pub time_cost: u32,
    pub parallelism: u32,
}

impl Default for VaultKdfParams {
    fn default() -> Self {
        Self {
            memory_cost_kib: ARGON2_M_KIB_DEFAULT,
            time_cost: ARGON2_T_DEFAULT,
            parallelism: default_parallelism(),
        }
    }
}

impl VaultKdfParams {
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            memory_cost_kib: parse_u32_env("ARPSCI_ARGON2_M_KIB").unwrap_or(defaults.memory_cost_kib),
            time_cost: parse_u32_env("ARPSCI_ARGON2_T").unwrap_or(defaults.time_cost),
            parallelism: parse_u32_env("ARPSCI_ARGON2_P").unwrap_or(defaults.parallelism),
        }
    }
}

#[derive(Clone)]
pub struct VaultCipherBlob {
    pub version: u32,
    pub kdf: String,
    pub aead: String,
    pub memory_cost_kib: u32,
    pub time_cost: u32,
    pub parallelism: u32,
    pub salt: [u8; 16],
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

fn derive_vault_key(
    master_password: &SecretString,
    params: VaultKdfParams,
    salt: &[u8],
) -> anyhow::Result<[u8; 32]> {
    let mut ikm = [0u8; 32];
    let kdf = argon2id(&params)?;
    kdf.hash_password_into(master_password.expose_secret().as_bytes(), salt, &mut ikm)
        .map_err(|e| anyhow::anyhow!("failed to derive vault key: {e}"))?;

    let hk = Hkdf::<Sha256>::new(Some(VAULT_KDF_LABEL), &ikm);
    let mut out = [0u8; 32];
    hk.expand(b"vault_enc_key", &mut out)
        .map_err(|_| anyhow::anyhow!("HKDF expand failed"))?;
    Ok(out)
}

/// In-memory encrypted blob store.
#[derive(Default)]
pub struct Vault {
    encrypted_blob: Option<VaultCipherBlob>,
}

impl Vault {
    pub fn set_encrypted_blob(
        &mut self,
        master_password: &SecretString,
        plaintext: &[u8],
        params: VaultKdfParams,
    ) -> anyhow::Result<()> {
        let mut salt = [0u8; 16];
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut nonce);

        let key_material = derive_vault_key(master_password, params, &salt)?;
        let key = Key::from_slice(&key_material);
        let cipher = ChaCha20Poly1305::new(key);
        let nonce_ref = Nonce::from_slice(&nonce);
        let ciphertext = cipher
            .encrypt(
                nonce_ref,
                chacha20poly1305::aead::Payload {
                    msg: plaintext,
                    aad: VAULT_AAD,
                },
            )
            .map_err(|_| anyhow::anyhow!("vault encryption failed"))?;

        self.encrypted_blob = Some(VaultCipherBlob {
            version: VAULT_BLOB_VERSION,
            kdf: "argon2id+hkdf-sha256".to_string(),
            aead: "chacha20poly1305".to_string(),
            memory_cost_kib: params.memory_cost_kib,
            time_cost: params.time_cost,
            parallelism: params.parallelism,
            salt,
            nonce,
            ciphertext,
        });
        Ok(())
    }

    pub fn decrypt_blob(&self, master_password: &SecretString) -> anyhow::Result<Vec<u8>> {
        let blob = self
            .encrypted_blob
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("vault blob is empty"))?;
        if blob.version != VAULT_BLOB_VERSION {
            anyhow::bail!(
                "unsupported vault blob version '{}'; expected '{}'",
                blob.version,
                VAULT_BLOB_VERSION
            );
        }

        let params = VaultKdfParams {
            memory_cost_kib: blob.memory_cost_kib,
            time_cost: blob.time_cost,
            parallelism: blob.parallelism,
        };
        let key_material = derive_vault_key(master_password, params, &blob.salt)?;
        let key = Key::from_slice(&key_material);
        let cipher = ChaCha20Poly1305::new(key);
        let nonce = Nonce::from_slice(&blob.nonce);
        cipher
            .decrypt(
                nonce,
                chacha20poly1305::aead::Payload {
                    msg: blob.ciphertext.as_ref(),
                    aad: VAULT_AAD,
                },
            )
            .map_err(|_| anyhow::anyhow!("vault decryption failed"))
    }
}

/// Loads when the app starts; verifies a stored PHC Argon2id hash.
pub struct MasterVault {
    /// PHC-format Argon2 hash string, or empty if misconfigured.
    stored_hash: String,
    kdf_params: VaultKdfParams,
    master_input: SecretString,
    pub unlocked: bool,
    status: String,
    /// When true, [`Self::unlocked`] is always treated as true.
    skip_vault: bool,
    /// Future hook: populate after successful unlock.
    pub vault: Vault,
}

impl MasterVault {
    pub fn new() -> Self {
        let skip_vault = !ENABLE_PASSWORD_LOCK
            || std::env::var("ARPSCI_SKIP_VAULT").unwrap_or_default() == "1";
        let stored_hash = std::env::var("ARPSCI_MASTER_HASH")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                std::fs::read_to_string(DEFAULT_HASH_FILE)
                    .ok()
                    .map(|s| s.lines().next().unwrap_or("").trim().to_string())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_default();

        Self {
            stored_hash,
            kdf_params: VaultKdfParams::from_env(),
            master_input: SecretString::new(String::new().into_boxed_str()),
            unlocked: false,
            status: String::new(),
            skip_vault,
            vault: Vault::default(),
        }
    }

    pub fn is_unlocked(&self) -> bool {
        self.skip_vault || self.unlocked
    }

    pub fn has_configured_hash(&self) -> bool {
        self.skip_vault || !self.stored_hash.is_empty()
    }

    fn try_unlock(&mut self) {
        self.status.clear();
        if self.skip_vault {
            self.unlocked = true;
            return;
        }
        if self.stored_hash.is_empty() {
            self.status =
                "No master hash configured. Set ARPSCI_MASTER_HASH or create runs/.master_hash."
                    .to_string();
            self.unlocked = false;
            return;
        }

        let argon2 = match argon2id(&self.kdf_params) {
            Ok(a) => a,
            Err(e) => {
                self.status = format!("Invalid Argon2id parameters: {e}");
                self.unlocked = false;
                return;
            }
        };
        let parsed_hash = match PasswordHash::new(&self.stored_hash) {
            Ok(h) => h,
            Err(e) => {
                self.status = format!("Invalid stored hash: {e}");
                return;
            }
        };

        if parsed_hash.algorithm.as_str() != "argon2id" {
            self.status = "Stored hash is not Argon2id; regenerate runs/.master_hash with secure defaults.".to_string();
            self.unlocked = false;
            return;
        }

        let password = self.master_input.expose_secret();
        match argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| anyhow::anyhow!("invalid password"))
        {
            Ok(()) => {
                self.unlocked = true;
                self.status = "Unlocked.".to_string();
            }
            Err(_) => {
                self.unlocked = false;
                self.status = "Wrong master password.".to_string();
            }
        }
    }

    pub fn lock(&mut self) {
        self.unlocked = false;
        self.master_input = SecretString::new(String::new().into_boxed_str());
        self.status.clear();
    }

    /// Full-screen unlock UI (CentralPanel).
    pub fn show_unlock_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Unlock");
        if self.skip_vault {
            ui.label("Vault gate is disabled (ARPSCI_SKIP_VAULT=1).");
            return;
        }
        if self.stored_hash.is_empty() {
            ui.label(self.status.clone());
            ui.label(format!(
                "Create `{}` with one line (PHC Argon2id hash), or set ARPSCI_MASTER_HASH.",
                DEFAULT_HASH_FILE
            ));
            return;
        }

        ui.label("Master password:");
        let mut plain: String = self.master_input.expose_secret().to_owned();
        let resp = ui.add(
            TextEdit::singleline(&mut plain)
                .password(true)
                .hint_text("••••••••"),
        );
        // Always mirror the text field into `master_input`. Relying only on `changed()` is unsafe:
        // password fields can desync from `changed()` across focus/Enter/lock-retry frames.
        self.master_input = SecretString::new(plain.into_boxed_str());

        if ui.button("Unlock").clicked()
            || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            self.try_unlock();
        }

        if !self.status.is_empty() {
            ui.label(egui::RichText::new(&self.status).weak());
        }

    }

    /// Thin top strip with Lock; call from a `TopBottomPanel::top`. Returns true if user locked.
    pub fn show_lock_bar(&mut self, ui: &mut egui::Ui) -> bool {
        if self.skip_vault {
            return false;
        }
        let mut clicked = false;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Lock").clicked() {
                clicked = true;
            }
        });
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_non_toy_parameters() {
        let params = VaultKdfParams::default();
        assert!(params.memory_cost_kib >= 64 * 1024);
        assert!(params.time_cost >= 3);
        assert!((1..=4).contains(&params.parallelism));
    }

    #[test]
    fn vault_blob_roundtrip() {
        let mut vault = Vault::default();
        let secret = SecretString::new("test-password".to_string().into_boxed_str());
        let plaintext = b"secret payload";
        let params = VaultKdfParams::default();

        vault
            .set_encrypted_blob(&secret, plaintext, params)
            .expect("encrypt blob");
        let out = vault.decrypt_blob(&secret).expect("decrypt blob");
        assert_eq!(out, plaintext);
    }
}
