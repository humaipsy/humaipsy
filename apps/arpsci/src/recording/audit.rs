use serde::Serialize;
use uuid::Uuid;

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use crate::recording::timing::now_rfc3339;
pub use crate::recording::timing::resolve_from_optional_rfc3339 as resolve_from_optional_payload;

pub const SCHEMA_VERSION: u32 = 1;

pub fn new_id() -> String {
    Uuid::new_v4().simple().to_string()
}

#[derive(Serialize)]
pub struct AuditRecord {
    pub schema_version: u32,
    pub kind: &'static str,
    pub ts: String,
    pub conversation_id: String,
    pub request_id: String,
    pub event_id: String,
    pub details: serde_json::Value,
}

pub struct AuditHandle {
    writer: Mutex<Option<BufWriter<File>>>,
    path: PathBuf,
}

impl AuditHandle {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            writer: Mutex::new(Some(BufWriter::new(file))),
            path,
        })
    }

    pub fn disabled() -> Self {
        Self {
            writer: Mutex::new(None),
            path: PathBuf::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append_json_line(&self, record: &AuditRecord) -> io::Result<()> {
        let mut guard = self.writer.lock().map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("audit mutex poisoned: {e}"))
        })?;
        if let Some(w) = guard.as_mut() {
            serde_json::to_writer(&mut *w, record)?;
            w.write_all(b"\n")?;
            w.flush()?;
        }
        Ok(())
    }
}
