//! Writes a PHC Argon2id hash to `vault/.master_hash` using secure defaults.

use humai_core::vault::{default_master_hash_file_path, write_master_hash_file};

fn main() -> std::io::Result<()> {
    write_master_hash_file(default_master_hash_file_path())
}
