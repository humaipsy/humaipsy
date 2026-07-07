//! Master-password gate (Argon2id) and a small encrypted [`Vault`] container.
//!
//! On first launch, the app prompts for a master password and writes `vault/.master_hash`.
//! Later launches verify against that file, or against `HUMAI_MASTER_HASH` if set.
//! Optional Argon2id tuning via env: `HUMAI_ARGON2_M_KIB`, `HUMAI_ARGON2_T`, `HUMAI_ARGON2_P`.
//! In release builds, the password lock is always enforced.
//! In debug builds, startup is unlocked by default for faster development.
//! Set `HUMAI_ENABLE_PASSWORD_LOCK=1` to enforce locking in debug builds.
//! Set `HUMAI_SKIP_VAULT=1` to bypass locking in debug builds.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordVerifier, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const DEFAULT_HASH_FILE: &str = "vault/.master_hash";
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

fn parse_bool_env(name: &str) -> Option<bool> {
    std::env::var(name).ok().and_then(|raw| {
        match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    })
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

/// Hash `password` and write a one-line PHC string to `path`, creating parent dirs as needed.
pub fn persist_master_hash(
    path: impl AsRef<Path>,
    password: &str,
    params: VaultKdfParams,
) -> anyhow::Result<String> {
    let phc = hash_master_password_phc(password, params)?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, format!("{phc}\n"))?;
    restrict_to_owner(path)?;
    Ok(phc)
}

/// Restricts a file to owner-only read/write (`0600`) on Unix, where the
/// default umask otherwise leaves it world-readable (`0644`) — letting any
/// other local user read the master-password hash and offline-brute-force
/// it. No-op on Windows, where per-user profile ACLs already isolate this.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

pub fn default_master_hash_file_path() -> PathBuf {
    PathBuf::from(DEFAULT_HASH_FILE)
}

pub fn write_master_hash_file(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    let params = VaultKdfParams::from_env();
    println!(
        "This writes a one-line Argon2id hash to {} (m={} KiB, t={}, p={}).",
        path.display(),
        params.memory_cost_kib,
        params.time_cost,
        params.parallelism,
    );
    if path.exists() {
        eprint!("File already exists. Overwrite? [y/N]: ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !line.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    let pw = rpassword::prompt_password("Master password: ")
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let confirm = rpassword::prompt_password("Again: ")
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    if pw != confirm {
        eprintln!("Passwords do not match.");
        std::process::exit(1);
    }

    let phc = persist_master_hash(path, &pw, params)
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    println!(
        "Wrote {} (keep runs/ out of version control; do not commit the password).",
        path.display()
    );
    let _ = phc;
    Ok(())
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
            memory_cost_kib: parse_u32_env("HUMAI_ARGON2_M_KIB").unwrap_or(defaults.memory_cost_kib),
            time_cost: parse_u32_env("HUMAI_ARGON2_T").unwrap_or(defaults.time_cost),
            parallelism: parse_u32_env("HUMAI_ARGON2_P").unwrap_or(defaults.parallelism),
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
    confirm_input: SecretString,
    pub unlocked: bool,
    status: String,
    /// When true, [`Self::unlocked`] is always treated as true.
    skip_vault: bool,
    failed_attempts: u32,
    next_attempt_at: Option<Instant>,
    /// Future hook: populate after successful unlock.
    pub vault: Vault,
}

impl Default for MasterVault {
    fn default() -> Self {
        Self::new()
    }
}

impl MasterVault {
    pub fn new() -> Self {
        let is_release = !cfg!(debug_assertions);
        let debug_lock_enabled = parse_bool_env("HUMAI_ENABLE_PASSWORD_LOCK").unwrap_or(false);
        let debug_skip_requested = parse_bool_env("HUMAI_SKIP_VAULT").unwrap_or(false);

        // Security stance:
        // - release: always enforce lock, ignore skip env.
        // - debug: unlocked by default unless lock explicitly enabled.
        let skip_vault = if is_release {
            false
        } else {
            !debug_lock_enabled || debug_skip_requested
        };

        let stored_hash = std::env::var("HUMAI_MASTER_HASH")
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
            confirm_input: SecretString::new(String::new().into_boxed_str()),
            unlocked: false,
            status: String::new(),
            skip_vault,
            failed_attempts: 0,
            next_attempt_at: None,
            vault: Vault::default(),
        }
    }

    pub fn is_unlocked(&self) -> bool {
        self.skip_vault || self.unlocked
    }

    pub fn has_configured_hash(&self) -> bool {
        self.skip_vault || !self.stored_hash.is_empty()
    }

    pub(crate) fn try_unlock(&mut self) {
        self.status.clear();
        if self.skip_vault {
            self.unlocked = true;
            return;
        }

        if let Some(deadline) = self.next_attempt_at {
            if Instant::now() < deadline {
                let secs_left = deadline.saturating_duration_since(Instant::now()).as_secs();
                self.status = format!("Too many failed attempts. Try again in {secs_left}s.");
                self.unlocked = false;
                return;
            }
            self.next_attempt_at = None;
        }

        if self.stored_hash.is_empty() {
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
            self.status = "Stored hash is not Argon2id; regenerate vault/.master_hash with secure defaults.".to_string();
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
                self.failed_attempts = 0;
                self.next_attempt_at = None;
                self.master_input = SecretString::new(String::new().into_boxed_str());
                self.status = "Unlocked.".to_string();
            }
            Err(_) => {
                self.unlocked = false;
                self.failed_attempts = self.failed_attempts.saturating_add(1);
                let shift = self.failed_attempts.saturating_sub(1).min(5);
                let backoff_secs = (1u64 << shift).min(30);
                self.next_attempt_at = Some(Instant::now() + Duration::from_secs(backoff_secs));
                self.master_input = SecretString::new(String::new().into_boxed_str());
                self.status = "Wrong master password.".to_string();
            }
        }
    }

    pub(crate) fn try_setup_password(&mut self) {
        self.status.clear();

        let password = self.master_input.expose_secret();
        let confirm = self.confirm_input.expose_secret();

        if password.is_empty() {
            self.status = "Enter a master password.".to_string();
            return;
        }
        if password != confirm {
            self.status = "Passwords do not match.".to_string();
            return;
        }

        match persist_master_hash(
            default_master_hash_file_path(),
            password,
            self.kdf_params,
        ) {
            Ok(phc) => {
                self.stored_hash = phc;
                self.unlocked = true;
                self.master_input = SecretString::new(String::new().into_boxed_str());
                self.confirm_input = SecretString::new(String::new().into_boxed_str());
                self.status = "Password created.".to_string();
            }
            Err(e) => {
                self.unlocked = false;
                self.status = format!("Could not save master password: {e}");
            }
        }
    }

    pub fn lock(&mut self) {
        // Allow explicit runtime lock in debug/dev mode even if startup skipped the vault.
        self.skip_vault = false;
        self.unlocked = false;
        self.master_input = SecretString::new(String::new().into_boxed_str());
        self.confirm_input = SecretString::new(String::new().into_boxed_str());
        self.status.clear();
    }

    /// Whether the vault gate is disabled (debug-mode default; see module docs).
    pub(crate) fn is_skip_vault(&self) -> bool {
        self.skip_vault
    }

    /// Whether no master password has been configured yet, i.e. first-run setup is needed.
    pub(crate) fn needs_password_setup(&self) -> bool {
        self.stored_hash.is_empty()
    }

    pub(crate) fn status(&self) -> &str {
        &self.status
    }

    pub(crate) fn master_input_mut(&mut self) -> &mut SecretString {
        &mut self.master_input
    }

    pub(crate) fn confirm_input_mut(&mut self) -> &mut SecretString {
        &mut self.confirm_input
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
    fn persist_master_hash_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("vault/.master_hash");
        let params = VaultKdfParams::default();
        let phc =
            persist_master_hash(&path, "setup-password", params).expect("persist master hash");
        assert!(path.is_file());
        assert!(phc.starts_with("$argon2id$"));

        let argon2 = argon2id(&params).expect("argon2");
        let parsed = PasswordHash::new(&phc).expect("parse phc");
        assert!(argon2
            .verify_password(b"setup-password", &parsed)
            .is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn persist_master_hash_restricts_permissions_to_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("vault/.master_hash");
        persist_master_hash(&path, "setup-password", VaultKdfParams::default())
            .expect("persist master hash");

        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "master hash file should be owner-only read/write, got {:o}",
            mode & 0o777
        );
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
