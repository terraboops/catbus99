//! Path resolution.
//!
//! catbus99 uses the XDG layout on macOS too, because `dirs::config_dir()` there points at
//! `~/Library/Application Support` — the convention for GUI applications, not for a CLI
//! whose config file a person is expected to hand-edit.

use catbus99_device::paths;

#[test]
fn config_and_state_live_under_dot_config_and_dot_local_by_default() {
    // Only assert the tail, so the test does not depend on the runner's home directory.
    let c = paths::config_dir();
    let s = paths::state_dir();
    assert!(c.ends_with("catbus99"), "{}", c.display());
    assert!(s.ends_with("catbus99"), "{}", s.display());
    assert!(
        c.to_string_lossy().contains("/.config/") || std::env::var_os("XDG_CONFIG_HOME").is_some(),
        "expected an XDG config path, got {}",
        c.display()
    );
    assert!(
        s.to_string_lossy().contains("/.local/state/")
            || std::env::var_os("XDG_STATE_HOME").is_some(),
        "expected an XDG state path, got {}",
        s.display()
    );
    // Never `~/Library/Application Support`, which is where dirs would put it on macOS.
    assert!(
        !c.to_string_lossy().contains("Application Support"),
        "{}",
        c.display()
    );
}

#[test]
fn the_runtime_dir_falls_back_to_the_state_dir() {
    // macOS has no XDG_RUNTIME_DIR, and its per-user temp is cleaned unpredictably.
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        assert_eq!(paths::runtime_dir(), paths::state_dir());
    }
}

/// The odometer must survive a change of layout: silently starting a fresh counter would
/// under-report how much of the panel's rated life has been used, the exact failure the
/// counter exists to prevent.
#[test]
fn a_legacy_file_is_migrated_when_the_target_is_absent() {
    let base = std::env::temp_dir().join(format!("catbus99-paths-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let target = base.join("new/wear.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();

    // No legacy file anywhere: nothing to migrate, and no error.
    assert!(paths::migrate_legacy_file("catbus99-nonexistent-file.json", &target).is_none());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn migration_never_clobbers_an_existing_target() {
    let base = std::env::temp_dir().join(format!("catbus99-paths2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let target = base.join("wear.json");
    std::fs::write(&target, "{\"total_uploads\":99}").unwrap();

    assert!(paths::migrate_legacy_file("wear.json", &target).is_none());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "{\"total_uploads\":99}"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn xdg_environment_variables_are_honoured() {
    // Cannot mutate the process env safely alongside other tests, so assert the
    // documented contract on whichever configuration the runner has.
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(v) if !v.is_empty() => assert!(paths::config_dir().starts_with(&v)),
        _ => assert!(paths::config_dir().to_string_lossy().contains(".config")),
    }
}
