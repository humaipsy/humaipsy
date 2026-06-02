//! Writes a PHC Argon2id hash to `runs/.master_hash` using secure defaults.

use humai::vault::{VaultKdfParams, hash_master_password_phc};
use std::io::Write;
use std::path::Path;

fn main() -> std::io::Result<()> {
    let path = Path::new("runs").join(".master_hash");
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
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let confirm = rpassword::prompt_password("Again: ")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    if pw != confirm {
        eprintln!("Passwords do not match.");
        std::process::exit(1);
    }

    let phc = hash_master_password_phc(&pw, params)
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, format!("{phc}\n"))?;

    println!(
        "Wrote {} (keep runs/ out of version control; do not commit the password).",
        path.display()
    );
    Ok(())
}
