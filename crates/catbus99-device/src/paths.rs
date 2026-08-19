//! Where catbus99 keeps its files.
//!
//! XDG layout, on macOS too. `dirs::config_dir()` would give
//! `~/Library/Application Support` there, which is the convention for GUI applications;
//! command-line tools on macOS overwhelmingly use `~/.config` (git, gh, nvim, starship),
//! and that is where someone will look for a config file they are meant to hand-edit.

use std::path::PathBuf;

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(std::env::temp_dir)
}

fn from_env_or(var: &str, fallback: PathBuf) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => fallback,
    }
}

/// `$XDG_CONFIG_HOME/catbus99`, else `~/.config/catbus99`.
pub fn config_dir() -> PathBuf {
    from_env_or("XDG_CONFIG_HOME", home().join(".config")).join("catbus99")
}

/// `$XDG_STATE_HOME/catbus99`, else `~/.local/state/catbus99`.
pub fn state_dir() -> PathBuf {
    from_env_or("XDG_STATE_HOME", home().join(".local").join("state")).join("catbus99")
}

/// `$XDG_RUNTIME_DIR/catbus99`, else the state directory.
///
/// macOS has no `XDG_RUNTIME_DIR`, and its per-user temp directory is cleaned
/// unpredictably, so the socket lives beside the state files there.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("catbus99"),
        _ => state_dir(),
    }
}

/// Legacy locations used before the XDG layout, checked once for migration.
pub fn legacy_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = dirs::config_dir() {
        v.push(d.join("catbus99"));
    }
    if let Some(d) = dirs::state_dir() {
        v.push(d.join("catbus99"));
    }
    if let Some(d) = dirs::data_local_dir() {
        v.push(d.join("catbus99"));
    }
    v
}

/// Move `name` from a legacy directory into `target` if it is not already there.
///
/// Used for the wear odometer above all: silently starting a fresh counter would
/// under-report how much of the display's rated life has been spent, which is the exact
/// failure the odometer exists to prevent.
pub fn migrate_legacy_file(name: &str, target: &std::path::Path) -> Option<PathBuf> {
    if target.exists() {
        return None;
    }
    for dir in legacy_dirs() {
        let candidate = dir.join(name);
        if candidate == target || !candidate.exists() {
            continue;
        }
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(&candidate, target).is_ok()
            || (std::fs::copy(&candidate, target).is_ok()
                && std::fs::remove_file(&candidate).is_ok())
        {
            return Some(candidate);
        }
    }
    None
}
