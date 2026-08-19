//! The data-source subprocess contract.
//!
//! Sources are arbitrary user programs, so the interesting cases are all the ways one can
//! misbehave. A broken source must produce a clear error and leave the previous readings
//! alone to age out on their own TTL — it must never take out the screen.

use catbus99_daemon::sources::{run_source, SourceError, SourceRegistry, SourceSpec};
use catbus99_model::Value;
use chrono::Utc;
use std::path::PathBuf;

struct TempScript {
    dir: PathBuf,
    path: PathBuf,
}

impl TempScript {
    fn new(name: &str, body: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("catbus99-src-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.sh"));
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { dir, path }
    }

    fn spec(&self, id: &str) -> SourceSpec {
        SourceSpec {
            id: id.into(),
            command: vec![self.path.display().to_string()],
            schedule: "0 * * * * *".into(),
            timeout_ms: 3000,
            ttl_secs: 600,
        }
    }
}

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn a_well_behaved_source_produces_stamped_points() {
    let s = TempScript::new(
        "ok",
        "#!/bin/sh\necho '{\"datapoints\":[{\"key\":\"a\",\"value\":0.5,\"unit\":\"ratio\"},{\"key\":\"b\",\"value\":\"hello\"}]}'\n",
    );
    let now = Utc::now();
    let points = run_source(&s.spec("demo"), now).await.unwrap();

    assert_eq!(points.len(), 2);
    // The daemon stamps source, time and TTL; the script supplies only the reading.
    assert!(points.iter().all(|p| p.source == "demo"));
    assert!(points.iter().all(|p| p.observed_at == now));
    assert!(points.iter().all(|p| p.ttl_secs == Some(600)));
    assert_eq!(points[0].value, Value::Number(0.5));
    assert_eq!(points[0].unit.as_deref(), Some("ratio"));
    assert_eq!(points[1].value, Value::Text("hello".into()));
}

#[tokio::test]
async fn a_point_may_override_the_source_ttl() {
    let s = TempScript::new(
        "ttl",
        "#!/bin/sh\necho '{\"datapoints\":[{\"key\":\"a\",\"value\":1,\"ttl_secs\":30}]}'\n",
    );
    let points = run_source(&s.spec("demo"), Utc::now()).await.unwrap();
    assert_eq!(points[0].ttl_secs, Some(30));
}

#[tokio::test]
async fn a_nonzero_exit_is_reported_with_its_stderr() {
    let s = TempScript::new(
        "fail",
        "#!/bin/sh\necho 'the api key expired' >&2\nexit 3\n",
    );
    match run_source(&s.spec("demo"), Utc::now()).await {
        Err(SourceError::ExitStatus { id, code, stderr }) => {
            assert_eq!(id, "demo");
            assert_eq!(code, "3");
            // stderr is what tells a user *why* their source broke.
            assert!(stderr.contains("api key expired"), "stderr was {stderr:?}");
        }
        other => panic!("expected an exit-status error, got {other:?}"),
    }
}

#[tokio::test]
async fn stderr_is_truncated_so_a_chatty_failure_cannot_flood_the_log() {
    let s = TempScript::new(
        "noisy",
        "#!/bin/sh\nhead -c 100000 /dev/zero | tr '\\0' 'x' >&2\nexit 1\n",
    );
    match run_source(&s.spec("demo"), Utc::now()).await {
        Err(SourceError::ExitStatus { stderr, .. }) => {
            assert!(stderr.len() <= 200, "stderr was {} chars", stderr.len());
        }
        other => panic!("expected an exit-status error, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_output_is_reported_as_a_json_error() {
    let s = TempScript::new("bad", "#!/bin/sh\necho 'not json at all'\n");
    assert!(matches!(
        run_source(&s.spec("demo"), Utc::now()).await,
        Err(SourceError::Json { .. })
    ));
}

#[tokio::test]
async fn output_missing_the_datapoints_key_is_rejected() {
    let s = TempScript::new("nokey", "#!/bin/sh\necho '{\"values\":[]}'\n");
    assert!(matches!(
        run_source(&s.spec("demo"), Utc::now()).await,
        Err(SourceError::Json { .. })
    ));
}

/// A hung source must not hang the daemon. `kill_on_drop` should reap it too.
#[tokio::test]
async fn a_hanging_source_times_out() {
    let s = TempScript::new("hang", "#!/bin/sh\nsleep 30\n");
    let mut spec = s.spec("demo");
    spec.timeout_ms = 300;

    let started = std::time::Instant::now();
    match run_source(&spec, Utc::now()).await {
        Err(SourceError::Timeout { id, timeout_ms }) => {
            assert_eq!(id, "demo");
            assert_eq!(timeout_ms, 300);
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "timeout was not enforced promptly"
    );
}

#[tokio::test]
async fn a_missing_executable_is_reported_clearly() {
    let spec = SourceSpec {
        id: "gone".into(),
        command: vec!["/definitely/not/here".into()],
        schedule: "0 * * * * *".into(),
        timeout_ms: 1000,
        ttl_secs: 60,
    };
    match run_source(&spec, Utc::now()).await {
        Err(SourceError::Spawn { id, command, .. }) => {
            assert_eq!(id, "gone");
            assert!(command.contains("not/here"));
        }
        other => panic!("expected a spawn error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_empty_datapoints_array_is_valid() {
    let s = TempScript::new("empty", "#!/bin/sh\necho '{\"datapoints\":[]}'\n");
    assert!(run_source(&s.spec("demo"), Utc::now())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn arguments_are_passed_through() {
    let s = TempScript::new(
        "args",
        "#!/bin/sh\necho \"{\\\"datapoints\\\":[{\\\"key\\\":\\\"$1\\\",\\\"value\\\":1}]}\"\n",
    );
    let mut spec = s.spec("demo");
    spec.command.push("passed".into());
    let points = run_source(&spec, Utc::now()).await.unwrap();
    assert_eq!(points[0].key, "passed");
}

// --- registry ---

#[test]
fn a_missing_registry_is_empty_rather_than_an_error() {
    let path = std::env::temp_dir().join("catbus99-no-such-sources.toml");
    let _ = std::fs::remove_file(&path);
    assert!(SourceRegistry::load(&path).unwrap().sources.is_empty());
}

#[test]
fn a_malformed_registry_is_an_error_not_a_silent_empty() {
    // Silently ignoring a broken config would leave the screen mysteriously static.
    let path = std::env::temp_dir().join(format!("catbus99-bad-{}.toml", std::process::id()));
    std::fs::write(&path, "this is not toml [[[").unwrap();
    assert!(matches!(
        SourceRegistry::load(&path),
        Err(SourceError::Parse { .. })
    ));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn registry_defaults_are_applied() {
    let path = std::env::temp_dir().join(format!("catbus99-defaults-{}.toml", std::process::id()));
    std::fs::write(
        &path,
        "[[source]]\nid = \"x\"\ncommand = [\"/bin/true\"]\nschedule = \"0 * * * * *\"\n",
    )
    .unwrap();
    let reg = SourceRegistry::load(&path).unwrap();
    let s = reg.get("x").unwrap();
    assert_eq!(s.timeout_ms, 10_000);
    assert_eq!(s.ttl_secs, 900);
    assert!(reg.get("nope").is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_invalid_cron_expression_names_the_source() {
    let spec = SourceSpec {
        id: "bad-sched".into(),
        command: vec!["/bin/true".into()],
        schedule: "not a cron".into(),
        timeout_ms: 1000,
        ttl_secs: 60,
    };
    match spec.parsed_schedule() {
        Err(SourceError::Schedule { id, schedule, .. }) => {
            assert_eq!(id, "bad-sched");
            assert_eq!(schedule, "not a cron");
        }
        other => panic!("expected a schedule error, got {other:?}"),
    }
}
