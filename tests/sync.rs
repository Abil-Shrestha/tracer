use chrono::{Duration, TimeZone, Utc};
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;
use tracer::storage::{sqlite::SqliteStorage, IssueUpdates};
use tracer::sync::{self, ImportOptions, Resolution, SyncEvent, SyncRecord, SyncSession};
use tracer::{Dependency, DependencyType, Event, EventType, Issue, IssueType, Status, Storage};

fn record(id: &str) -> SyncRecord {
    let created = Utc.with_ymd_and_hms(2025, 2, 3, 4, 5, 6).unwrap();
    SyncRecord {
        sync_version: Some(1),
        issue: Issue {
            id: id.into(),
            title: format!("Title {id}"),
            description: "description".into(),
            design: "design".into(),
            acceptance_criteria: "acceptance".into(),
            notes: "notes".into(),
            status: Status::Closed,
            priority: 1,
            issue_type: IssueType::Bug,
            assignee: "alice".into(),
            estimated_minutes: Some(37),
            created_at: created,
            updated_at: created + Duration::hours(3),
            closed_at: Some(created + Duration::hours(2)),
            external_ref: Some("upstream#42".into()),
            dependencies: Vec::new(),
        },
        labels: Some(vec!["urgent".into(), "backend".into()]),
        events: Some(vec![SyncEvent {
            sync_id: format!("origin-{id}-42"),
            event: Event {
                id: 42,
                issue_id: id.into(),
                event_type: EventType::Commented,
                actor: "bob".into(),
                old_value: Some("old".into()),
                new_value: Some("new".into()),
                comment: Some("original comment".into()),
                created_at: created + Duration::minutes(15),
            },
        }]),
    }
}

fn dependency(owner: &str, target: &str) -> Dependency {
    Dependency {
        issue_id: owner.into(),
        depends_on_id: target.into(),
        dep_type: DependencyType::Blocks,
        created_at: Utc.with_ymd_and_hms(2025, 2, 3, 5, 0, 0).unwrap(),
        created_by: "original-author".into(),
    }
}

fn incoming() -> ImportOptions {
    ImportOptions {
        resolution: Resolution::Incoming,
        ..Default::default()
    }
}

fn canonical(records: &[SyncRecord]) -> Vec<u8> {
    sync::encode(&sync::parse(&sync::encode(records).unwrap()).unwrap()).unwrap()
}

fn cli(db: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tracer"))
        .arg("--db")
        .arg(db)
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn complete_roundtrip_preserves_forward_refs_timestamps_and_original_event_ids() {
    let dir = TempDir::new().unwrap();
    let mut a = record("opaque-a");
    a.issue
        .dependencies
        .push(dependency("opaque-a", "z-legacy-99"));
    let b = record("z-legacy-99"); // Same numeric event ID, distinct portable identity.
    let expected = canonical(&[a, b]);
    let records = sync::parse(&expected).unwrap(); // Dependent sorts before its target.
    let mut db = SqliteStorage::new(dir.path().join("one.db")).unwrap();
    db.import_snapshot(&records, Default::default(), Some("first"))
        .unwrap();
    assert_eq!(canonical(&db.sync_snapshot().unwrap()), expected);
    let mut other = SqliteStorage::new(dir.path().join("two.db")).unwrap();
    other
        .import_snapshot(
            &db.sync_snapshot().unwrap(),
            Default::default(),
            Some("second"),
        )
        .unwrap();
    for _ in 0..3 {
        other
            .import_snapshot(&records, Default::default(), Some("second"))
            .unwrap();
    }
    assert_eq!(canonical(&other.sync_snapshot().unwrap()), expected);
    assert_eq!(other.get_events("opaque-a", 100).unwrap()[0].id, 42);
    assert_eq!(other.get_events("z-legacy-99", 100).unwrap().len(), 1);
    assert!(other.get_dirty_issues().unwrap().is_empty());
}

#[test]
fn invalid_input_collisions_and_late_sql_failure_leave_everything_unchanged() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.db");
    let mut db = SqliteStorage::new(&path).unwrap();
    db.import_snapshot(&[record("keep")], Default::default(), Some("baseline"))
        .unwrap();
    db.add_comment("keep", "local", "unpublished").unwrap();
    let before = canonical(&db.sync_snapshot().unwrap());
    let dirty = db.get_dirty_issues().unwrap();
    let base = db.get_metadata(sync::BASE_KEY).unwrap();
    let mut invalid = record("new");
    invalid.issue.priority = 8;
    assert!(db
        .import_snapshot(&[record("valid"), invalid], incoming(), Some("bad"))
        .is_err());
    let mut missing = record("new");
    missing.issue.dependencies.push(dependency("new", "absent"));
    assert!(db
        .import_snapshot(&[record("valid"), missing], incoming(), Some("bad"))
        .is_err());
    let mut collision = record("keep");
    collision.issue.created_at -= Duration::seconds(1);
    for options in [
        ImportOptions::default(),
        incoming(),
        ImportOptions {
            skip_existing: true,
            ..Default::default()
        },
    ] {
        assert!(db
            .import_snapshot(&[record("valid"), collision.clone()], options, Some("bad"))
            .unwrap_err()
            .to_string()
            .contains("Identity collision"));
    }
    assert!(sync::parse(&sync::encode(&[record("keep"), collision]).unwrap()).is_err());
    let mut bad_event = record("keep");
    bad_event.events.as_mut().unwrap()[0].event.comment = Some("rewritten history".into());
    assert!(db
        .import_snapshot(&[bad_event], incoming(), Some("bad"))
        .unwrap_err()
        .to_string()
        .contains("Event identity collision"));
    let mut malformed = sync::encode(&[record("valid")]).unwrap();
    malformed.extend_from_slice(b"<<<<<<< conflict\n");
    assert!(sync::parse(&malformed).is_err());
    // Inject a storage failure after an earlier valid row has been written.
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER fail_import BEFORE INSERT ON issues WHEN NEW.id = 'z-fail'
         BEGIN SELECT RAISE(ABORT, 'injected disk/storage failure'); END;",
        )
        .unwrap();
    assert!(db
        .import_snapshot(
            &[record("a-valid"), record("z-fail")],
            incoming(),
            Some("bad")
        )
        .is_err());
    assert_eq!(canonical(&db.sync_snapshot().unwrap()), before);
    assert_eq!(db.get_dirty_issues().unwrap(), dirty);
    assert_eq!(db.get_metadata(sync::BASE_KEY).unwrap(), base);
    assert_eq!(
        db.get_metadata(sync::HASH_KEY).unwrap().as_deref(),
        Some("baseline")
    );
}

#[test]
fn explicit_resolution_replaces_collections_fields_and_removes_issues() {
    let dir = TempDir::new().unwrap();
    let mut db = SqliteStorage::new(dir.path().join("test.db")).unwrap();
    let mut a = record("a");
    a.issue.dependencies.push(dependency("a", "b"));
    db.import_snapshot(&[a.clone(), record("b")], Default::default(), Some("base"))
        .unwrap();
    a.issue.dependencies.clear();
    a.labels = Some(Vec::new());
    a.events = Some(Vec::new());
    a.issue.estimated_minutes = None;
    a.issue.external_ref = None;
    a.issue.closed_at = None;
    a.issue.status = Status::Open;
    a.issue.updated_at += Duration::hours(1);
    let before = canonical(&db.sync_snapshot().unwrap());
    let summary = db
        .import_snapshot(
            &[a.clone()],
            ImportOptions {
                dry_run: true,
                ..incoming()
            },
            None,
        )
        .unwrap();
    assert_eq!((summary.changed, summary.removed), (1, 1));
    assert_eq!(canonical(&db.sync_snapshot().unwrap()), before);
    assert!(db.get_metadata("sync_pending").unwrap().is_none());
    db.import_snapshot(&[a.clone()], incoming(), None).unwrap();
    assert_eq!(canonical(&db.sync_snapshot().unwrap()), canonical(&[a]));
    assert!(db.get_issue("b").unwrap().is_none());
    // Publish the deletion as the new baseline, then remove the last issue.
    let snapshot = db.sync_snapshot().unwrap();
    db.acknowledge_snapshot(&snapshot, "published").unwrap();
    db.import_snapshot(&[], incoming(), None).unwrap();
    let mut session = SyncSession::open(&dir.path().join("test.db")).unwrap();
    session.publish(&mut db, false).unwrap();
    assert!(fs::read(session.path).unwrap().is_empty());
}

#[test]
fn legacy_missing_collections_preserve_local_history_without_import_events() {
    let dir = TempDir::new().unwrap();
    let mut db = SqliteStorage::new(dir.path().join("test.db")).unwrap();
    db.import_snapshot(&[record("a")], Default::default(), Some("base"))
        .unwrap();
    let legacy = serde_json::to_vec(&record("a").issue).unwrap();
    db.import_snapshot(
        &sync::parse(&legacy).unwrap(),
        Default::default(),
        Some("legacy"),
    )
    .unwrap();
    assert_eq!(db.get_labels("a").unwrap().len(), 2);
    assert_eq!(db.get_events("a", 100).unwrap().len(), 1);
    assert_eq!(db.get_dirty_issues().unwrap(), ["a"]);
    let mut new_db = SqliteStorage::new(dir.path().join("fresh.db")).unwrap();
    new_db
        .import_snapshot(&sync::parse(&legacy).unwrap(), Default::default(), None)
        .unwrap();
    assert!(new_db.get_events("a", 100).unwrap().is_empty());
    assert!(new_db.get_labels("a").unwrap().is_empty());
    db.add_comment("a", "local", "keep unpublished comment")
        .unwrap();
    db.add_label("a", "local-label", "local").unwrap();
    let mut edited = record("a").issue;
    edited.title = "legacy scalar edit".into();
    db.import_snapshot(
        &sync::parse(&serde_json::to_vec(&edited).unwrap()).unwrap(),
        incoming(),
        None,
    )
    .unwrap();
    assert_eq!(db.get_events("a", 100).unwrap().len(), 3);
    assert!(db
        .get_labels("a")
        .unwrap()
        .contains(&"local-label".to_string()));
}

#[test]
fn upgrading_old_sqlite_keeps_stable_event_identities_across_copies() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("old.db");
    let mut old = SqliteStorage::new(&path).unwrap();
    old.create_issue(&record("legacy").issue, "author").unwrap();
    old.add_comment("legacy", "author", "old comment").unwrap();
    let events = serde_json::to_value(old.get_events("legacy", 100).unwrap()).unwrap();
    drop(old);
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP INDEX idx_events_sync_id;
         ALTER TABLE events DROP COLUMN sync_id;
         ALTER TABLE events DROP COLUMN original_id;",
        )
        .unwrap();
    fs::copy(&path, dir.path().join("copy.db")).unwrap();
    let mut upgraded = SqliteStorage::new(&path).unwrap();
    let copy = SqliteStorage::new(dir.path().join("copy.db")).unwrap();
    let snapshot = upgraded.sync_snapshot().unwrap();
    assert_eq!(
        canonical(&snapshot),
        canonical(&copy.sync_snapshot().unwrap())
    );
    upgraded
        .import_snapshot(&snapshot, Default::default(), Some("upgraded"))
        .unwrap();
    assert_eq!(
        serde_json::to_value(upgraded.get_events("legacy", 100).unwrap()).unwrap(),
        events
    );
    assert_eq!(upgraded.get_events("legacy", 100).unwrap().len(), 2);
}

#[test]
fn dirty_and_already_exported_divergence_never_silently_choose_a_side() {
    let dir = TempDir::new().unwrap();
    let mut db = SqliteStorage::new(dir.path().join("test.db")).unwrap();
    db.import_snapshot(&[record("a")], Default::default(), Some("base"))
        .unwrap();
    let mut remote = record("a");
    remote.issue.title = "remote branch edit".into();
    db.update_issue(
        "a",
        &IssueUpdates {
            title: Some("local branch edit".into()),
            ..Default::default()
        },
        "local",
    )
    .unwrap();
    let local = db.sync_snapshot().unwrap();
    assert!(db
        .import_snapshot(&[remote.clone()], Default::default(), Some("remote"))
        .is_err());
    assert_eq!(canonical(&db.sync_snapshot().unwrap()), canonical(&local));
    assert_eq!(db.get_dirty_issues().unwrap(), ["a"]);
    db.acknowledge_snapshot(&local, "local-export").unwrap();
    assert!(db.get_dirty_issues().unwrap().is_empty());
    // Clean does NOT mean unmodified on this branch.
    assert!(db
        .import_snapshot(&[remote.clone()], Default::default(), Some("remote"))
        .is_err());
    db.import_snapshot(
        &[remote.clone(), record("new-remote")],
        ImportOptions {
            resolution: Resolution::Local,
            ..Default::default()
        },
        None,
    )
    .unwrap();
    assert_eq!(
        db.get_issue("a").unwrap().unwrap().title,
        "local branch edit"
    );
    assert!(db.get_issue("new-remote").unwrap().is_some());
    db.import_snapshot(&[remote], incoming(), None).unwrap();
    assert_eq!(
        db.get_issue("a").unwrap().unwrap().title,
        "remote branch edit"
    );
}

#[test]
fn publication_races_fail_closed_and_publish_before_ack_recovers() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.db");
    let mut session = SyncSession::open(&path).unwrap();
    let mut db = SqliteStorage::new(&path).unwrap();
    db.import_snapshot(&[record("a")], Default::default(), None)
        .unwrap();
    session.publish(&mut db, false).unwrap();
    db.add_comment("a", "local", "not yet published").unwrap();
    let before = canonical(&db.sync_snapshot().unwrap());
    let old_file = fs::read(&session.path).unwrap();
    fs::write(&session.path, b"malformed concurrent writer\n").unwrap();
    assert!(session.publish(&mut db, false).is_err());
    assert_eq!(
        fs::read(&session.path).unwrap(),
        b"malformed concurrent writer\n"
    );
    assert_eq!(db.get_dirty_issues().unwrap(), ["a"]);
    fs::write(&session.path, &old_file).unwrap();
    // Simulate a crash after durable rename but before DB acknowledgement.
    sync::publish_atomic(&session.path, &before).unwrap();
    drop(session);
    let mut reopened = SyncSession::open(&path).unwrap();
    reopened.import(&mut db).unwrap();
    reopened.publish(&mut db, false).unwrap();
    assert_eq!(canonical(&db.sync_snapshot().unwrap()), before);
    assert_eq!(fs::read(&reopened.path).unwrap(), before);
    assert!(db.get_dirty_issues().unwrap().is_empty());
    // A failed rename leaves the existing destination intact and removes its temp.
    let directory = dir.path().join("not-a-file");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep"), "intact").unwrap();
    assert!(sync::publish_atomic(&directory, b"new data").is_err());
    assert_eq!(fs::read(directory.join("keep")).unwrap(), b"intact");
    assert!(!fs::read_dir(dir.path()).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn cli_reads_are_unchanged_and_invalid_auto_import_blocks_mutations() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    let jsonl = dir.path().join("issues.jsonl");
    success(cli(&db, &["create", "keep", "--json"]));
    let before_file = fs::read(&jsonl).unwrap();
    let before_db = fs::read(&db).unwrap();
    for command in ["list", "ready", "stats"] {
        success(cli(&db, &[command, "--json"]));
    }
    assert_eq!(fs::read(&jsonl).unwrap(), before_file);
    assert_eq!(fs::read(&db).unwrap(), before_db);
    let mut invalid = before_file.clone();
    invalid.extend_from_slice(b"not JSON\n");
    fs::write(&jsonl, &invalid).unwrap();
    assert!(!cli(&db, &["create", "must not exist"]).status.success());
    assert_eq!(fs::read(&jsonl).unwrap(), invalid);
    assert_eq!(success(cli(&db, &["export"])), before_file);
    // A validated explicit import bypasses the broken auto-import for recovery.
    let backup = dir.path().join("backup.jsonl");
    fs::write(&backup, &before_file).unwrap();
    success(cli(
        &db,
        &["import", "--input", backup.to_str().unwrap(), "--dry-run"],
    ));
    assert_eq!(fs::read(&jsonl).unwrap(), invalid);
    success(cli(&db, &["import", "--input", backup.to_str().unwrap()]));
    assert_eq!(fs::read(&jsonl).unwrap(), before_file);
    // Numeric identity collisions and duplicate incoming IDs fail in the CLI too.
    let mut collision = sync::parse(&before_file).unwrap();
    collision[0].issue.created_at -= Duration::days(1);
    fs::write(&jsonl, sync::encode(&collision).unwrap()).unwrap();
    assert!(!cli(&db, &["list"]).status.success());
    assert!(!cli(
        &db,
        &[
            "import",
            "--input",
            jsonl.to_str().unwrap(),
            "--resolve",
            "incoming"
        ]
    )
    .status
    .success());
    assert_eq!(success(cli(&db, &["export"])), before_file);
    let mut duplicates = sync::parse(&before_file).unwrap();
    duplicates.extend(collision);
    fs::write(&jsonl, sync::encode(&duplicates).unwrap()).unwrap();
    assert!(!cli(&db, &["list"]).status.success());
    assert!(!cli(&db, &["import", "--input", jsonl.to_str().unwrap()])
        .status
        .success());
    fs::remove_file(&jsonl).unwrap();
    assert!(!cli(&db, &["list"]).status.success());
    assert_eq!(success(cli(&db, &["export"])), before_file);
}

#[cfg(unix)]
#[test]
fn failed_cli_publication_returns_failure_and_keeps_dirty_comment_for_retry() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.db");
    let jsonl = dir.path().join("issues.jsonl");
    let backup = dir.path().join("saved.jsonl");
    let issue: Issue =
        serde_json::from_slice(&success(cli(&path, &["create", "one", "--json"]))).unwrap();
    fs::rename(&jsonl, &backup).unwrap();
    std::os::unix::fs::symlink(&backup, &jsonl).unwrap();
    // Reading is possible, but publication deliberately refuses a symlink.
    assert!(!cli(&path, &["comment", &issue.id, "survives failure"])
        .status
        .success());
    let storage = SqliteStorage::new(&path).unwrap();
    assert_eq!(storage.get_dirty_issues().unwrap(), [issue.id.clone()]);
    assert_eq!(storage.get_events(&issue.id, 100).unwrap().len(), 2);
    drop(storage);
    fs::remove_file(&jsonl).unwrap();
    fs::rename(&backup, &jsonl).unwrap();
    success(cli(&path, &["list"]));
    let records = sync::parse(&fs::read(&jsonl).unwrap()).unwrap();
    assert!(records[0]
        .events
        .as_ref()
        .unwrap()
        .iter()
        .any(|e| e.event.comment.as_deref() == Some("survives failure")));
    assert!(SqliteStorage::new(&path)
        .unwrap()
        .get_dirty_issues()
        .unwrap()
        .is_empty());
}

#[test]
fn concurrent_cli_cycles_keep_all_issues_comments_and_clean_exports() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    // A held lock blocks even the first schema creation; release admits the CLI.
    let lock = SyncSession::open(&db).unwrap();
    let mut children = Vec::new();
    for i in 0..12 {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_tracer"))
                .arg("--db")
                .arg(&db)
                .args(["create", &format!("parallel {i}"), "--json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(children.iter_mut().all(|c| c.try_wait().unwrap().is_none()));
    assert!(!db.exists());
    drop(lock);
    let mut ids = Vec::new();
    for child in children {
        let issue: Issue =
            serde_json::from_slice(&success(child.wait_with_output().unwrap())).unwrap();
        ids.push(issue.id);
    }
    let mut comments = Vec::new();
    for i in 0..12 {
        comments.push(
            Command::new(env!("CARGO_BIN_EXE_tracer"))
                .arg("--db")
                .arg(&db)
                .args(["comment", &ids[0], &format!("parallel comment {i}")])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in comments {
        success(child.wait_with_output().unwrap());
    }
    let storage = SqliteStorage::new(&db).unwrap();
    let snapshot = storage.sync_snapshot().unwrap();
    assert_eq!(snapshot.len(), 12);
    assert_eq!(storage.get_events(&ids[0], 100).unwrap().len(), 13);
    assert_eq!(
        fs::read(dir.path().join("issues.jsonl")).unwrap(),
        canonical(&snapshot)
    );
    assert!(storage.get_dirty_issues().unwrap().is_empty());
}
