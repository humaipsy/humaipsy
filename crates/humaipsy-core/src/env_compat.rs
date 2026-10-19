//! Environment lookup that still answers to the project's old `HUMAI_` prefix.
//!
//! 0.1.6 renamed the application to HumaiPsy, so every setting moved from
//! `HUMAI_<NAME>` to `HUMAIPSY_<NAME>`. A machine configured against the old name has
//! to keep reading its own configuration: an unread `HUMAI_METRICS_DIR` does not
//! fail, it quietly moves a researcher's CSV output into the default directory, and
//! an unread `HUMAI_MASTER_HASH` silently changes which vault the app unlocks.
//!
//! So the new name wins and the old one is still consulted. This is the only place
//! that knows both spellings; everything else passes the suffix.

/// Reads `HUMAIPSY_<name>`, falling back to `HUMAI_<name>`.
pub fn var(name: &str) -> Result<String, std::env::VarError> {
    match std::env::var(format!("HUMAIPSY_{name}")) {
        Ok(value) => Ok(value),
        Err(_) => std::env::var(format!("HUMAI_{name}")),
    }
}

/// Like [`var`], but absence is `None` rather than an error.
pub fn var_opt(name: &str) -> Option<String> {
    var(name).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The names are distinct per test so a concurrent test cannot see another's value.
    #[test]
    fn reads_the_new_prefix() {
        // SAFETY: single-threaded test body; no other thread reads these names.
        unsafe { std::env::set_var("HUMAIPSY_COMPAT_PROBE_A", "new") };
        assert_eq!(var("COMPAT_PROBE_A").as_deref(), Ok("new"));
        unsafe { std::env::remove_var("HUMAIPSY_COMPAT_PROBE_A") };
    }

    #[test]
    fn falls_back_to_the_legacy_prefix() {
        // SAFETY: single-threaded test body; no other thread reads these names.
        unsafe { std::env::set_var("HUMAI_COMPAT_PROBE_B", "old") };
        assert_eq!(var("COMPAT_PROBE_B").as_deref(), Ok("old"));
        unsafe { std::env::remove_var("HUMAI_COMPAT_PROBE_B") };
    }

    #[test]
    fn the_new_prefix_beats_the_legacy_one() {
        // SAFETY: single-threaded test body; no other thread reads these names.
        unsafe { std::env::set_var("HUMAI_COMPAT_PROBE_C", "old") };
        unsafe { std::env::set_var("HUMAIPSY_COMPAT_PROBE_C", "new") };
        assert_eq!(var("COMPAT_PROBE_C").as_deref(), Ok("new"));
        unsafe { std::env::remove_var("HUMAI_COMPAT_PROBE_C") };
        unsafe { std::env::remove_var("HUMAIPSY_COMPAT_PROBE_C") };
    }

    #[test]
    fn an_absent_name_is_an_error() {
        assert!(var("COMPAT_PROBE_DEFINITELY_UNSET").is_err());
        assert!(var_opt("COMPAT_PROBE_DEFINITELY_UNSET").is_none());
    }
}
