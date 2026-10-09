use chrono::Utc;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::tempdir;
use tracer::storage::{sqlite::SqliteStorage, IssueUpdates, Storage};
use tracer::{Dependency, DependencyType, Issue, Status};

fn create(storage: &mut dyn Storage, id: &str, status: Status, assignee: &str) {
    let issue: Issue = serde_json::from_value(json!({
        "id": id, "title": "Ownership test", "status": status,
        "assignee": assignee, "priority": 2, "issue_type": "task",
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap();
    storage.create_issue(&issue, "setup").unwrap();
}

fn snapshot(storage: &dyn Storage, id: &str) -> Value {
    json!({
        "issue": storage.get_issue(id).unwrap(),
        "events": storage.get_events(id, 100).unwrap(),
        "dirty": storage.get_dirty_issues().unwrap()
    })
}

fn cli(db: &Path, actor: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tracer"))
        .arg("--db")
        .arg(db)
        .args(["--actor", actor, "--json"])
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> Issue {
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

fn conflict(output: Output, message: &str) {
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{output:?}"
    );
}

#[test]
fn competing_connections_have_one_winner_and_safe_retries() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("claims.db");
    let mut storage = SqliteStorage::new(&db).unwrap();
    create(&mut storage, "task", Status::Open, "");
    storage.clear_dirty_issues().unwrap();

    // Every independent connection sees unowned work before any may claim it.
    let barrier = Arc::new(Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|index| {
            let mut connection = SqliteStorage::new(&db).unwrap();
            let barrier = barrier.clone();
            thread::spawn(move || {
                assert!(connection
                    .get_issue("task")
                    .unwrap()
                    .unwrap()
                    .assignee
                    .is_empty());
                barrier.wait();
                let actor = format!("worker-{index}");
                let result = connection
                    .claim_issue("task", &actor)
                    .map_err(|e| e.to_string());
                (actor, result)
            })
        })
        .collect();
    let results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    let winners: Vec<_> = results
        .iter()
        .filter(|(_, result)| result.is_ok())
        .collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    let winner = &winners[0].0;
    let issue = storage.get_issue("task").unwrap().unwrap();
    assert_eq!(&issue.assignee, winner);
    assert_eq!(issue.status, Status::InProgress);
    assert_eq!(storage.get_dirty_issues().unwrap(), vec!["task"]);
    for (_, result) in &results {
        if let Err(message) = result {
            assert!(
                message.contains(&format!("owned by '{winner}'")),
                "{message}"
            );
        }
    }

    storage.clear_dirty_issues().unwrap();
    let before = snapshot(&storage, "task");
    storage.claim_issue("task", winner).unwrap();
    assert_eq!(snapshot(&storage, "task"), before);
    assert!(storage.claim_issue("task", "late-worker").is_err());
    assert_eq!(snapshot(&storage, "task"), before);
}

#[test]
fn claim_rejects_nonready_statuses_and_other_assignments_without_changes() {
    let dir = tempdir().unwrap();
    let mut storage = SqliteStorage::new(dir.path().join("claims.db")).unwrap();
    for (id, status, owner, message) in [
        ("closed", Status::Closed, "alice", "not ready"),
        ("blocked", Status::Blocked, "alice", "not ready"),
        ("orphan", Status::InProgress, "", "not ready"),
        ("assigned", Status::Open, "bob", "owned by 'bob'"),
    ] {
        create(&mut storage, id, status, owner);
        storage.clear_dirty_issues().unwrap();
        let before = snapshot(&storage, id);
        assert!(storage
            .claim_issue(id, "alice")
            .unwrap_err()
            .to_string()
            .contains(message));
        assert_eq!(snapshot(&storage, id), before);
    }

    create(&mut storage, "mine", Status::Open, "alice");
    storage.clear_dirty_issues().unwrap();
    let before = snapshot(&storage, "mine");
    for actor in ["", "   "] {
        assert!(storage
            .claim_issue("mine", actor)
            .unwrap_err()
            .to_string()
            .contains("nonempty actor"));
        assert_eq!(snapshot(&storage, "mine"), before);
    }
    storage.claim_issue("mine", "alice").unwrap();
    let issue = storage.get_issue("mine").unwrap().unwrap();
    assert_eq!(issue.assignee, "alice");
    assert_eq!(issue.status, Status::InProgress);
    assert!(storage
        .claim_issue("missing", "alice")
        .unwrap_err()
        .to_string()
        .contains("not found"));
    assert!(storage.get_issue("missing").unwrap().is_none());
}

#[test]
fn claim_checks_blocking_dependencies_even_on_retry() {
    let dir = tempdir().unwrap();
    let mut storage = SqliteStorage::new(dir.path().join("claims.db")).unwrap();
    create(&mut storage, "task", Status::Open, "");
    for (id, dep_type) in [
        ("blocker", DependencyType::Blocks),
        ("related", DependencyType::Related),
        ("parent", DependencyType::ParentChild),
        ("discovery", DependencyType::DiscoveredFrom),
    ] {
        create(&mut storage, id, Status::Open, "");
        storage
            .add_dependency(
                &Dependency {
                    issue_id: "task".into(),
                    depends_on_id: id.into(),
                    dep_type,
                    created_at: Utc::now(),
                    created_by: "setup".into(),
                },
                "setup",
            )
            .unwrap();
    }

    for status in [Status::Open, Status::InProgress, Status::Blocked] {
        storage
            .update_issue(
                "blocker",
                &IssueUpdates {
                    status: Some(status),
                    ..Default::default()
                },
                "setup",
            )
            .unwrap();
        storage.clear_dirty_issues().unwrap();
        let before = snapshot(&storage, "task");
        assert!(storage
            .claim_issue("task", "alice")
            .unwrap_err()
            .to_string()
            .contains("blocked by blocker"));
        assert_eq!(snapshot(&storage, "task"), before);
    }

    storage.close_issue("blocker", "done", "setup").unwrap();
    storage.claim_issue("task", "alice").unwrap();
    assert_eq!(
        storage.get_issue("task").unwrap().unwrap().assignee,
        "alice"
    );

    // Readiness must be checked again, even for an existing owner's retry.
    storage
        .update_issue(
            "blocker",
            &IssueUpdates {
                status: Some(Status::Open),
                ..Default::default()
            },
            "setup",
        )
        .unwrap();
    storage.clear_dirty_issues().unwrap();
    let before = snapshot(&storage, "task");
    assert!(storage
        .claim_issue("task", "alice")
        .unwrap_err()
        .to_string()
        .contains("blocked by blocker"));
    assert_eq!(snapshot(&storage, "task"), before);
}

#[test]
fn release_requires_owner_or_explicit_recovery_and_preserves_terminal_state() {
    let dir = tempdir().unwrap();
    let mut storage = SqliteStorage::new(dir.path().join("claims.db")).unwrap();
    create(&mut storage, "task", Status::Open, "");
    storage.claim_issue("task", "alice").unwrap();
    storage.clear_dirty_issues().unwrap();
    let before = snapshot(&storage, "task");
    assert!(storage
        .release_issue("task", "bob", false)
        .unwrap_err()
        .to_string()
        .contains("owned by 'alice'"));
    assert_eq!(snapshot(&storage, "task"), before);
    assert!(storage.release_issue("task", " ", true).is_err());
    assert_eq!(snapshot(&storage, "task"), before);

    storage.release_issue("task", "coordinator", true).unwrap();
    let recovered = storage.get_issue("task").unwrap().unwrap();
    assert!(recovered.assignee.is_empty());
    assert_eq!(recovered.status, Status::Open);
    let events = storage.get_events("task", 1).unwrap();
    assert_eq!(events[0].actor, "coordinator");
    assert_eq!(events[0].old_value.as_deref(), Some("alice"));
    assert_eq!(storage.get_dirty_issues().unwrap(), vec!["task"]);

    storage.claim_issue("task", "bob").unwrap();
    assert!(storage.release_issue("task", "alice", false).is_err());
    assert_eq!(storage.get_issue("task").unwrap().unwrap().assignee, "bob");
    storage.release_issue("task", "bob", false).unwrap();

    for status in [
        Status::Open,
        Status::InProgress,
        Status::Blocked,
        Status::Closed,
    ] {
        let id = status.to_string();
        create(&mut storage, &id, status, "alice");
        if status == Status::Closed {
            storage.close_issue(&id, "done", "alice").unwrap();
        }
        let closed_at = storage.get_issue(&id).unwrap().unwrap().closed_at;
        storage.release_issue(&id, "alice", false).unwrap();
        let released = storage.get_issue(&id).unwrap().unwrap();
        let expected = if status == Status::InProgress {
            Status::Open
        } else {
            status
        };
        assert_eq!(released.status, expected);
        assert!(released.assignee.is_empty());
        assert_eq!(released.closed_at, closed_at);
        storage.clear_dirty_issues().unwrap();
        let before = snapshot(&storage, &id);
        for force in [false, true] {
            assert!(storage
                .release_issue(&id, "alice", force)
                .unwrap_err()
                .to_string()
                .contains("no owner"));
            assert_eq!(snapshot(&storage, &id), before);
        }
    }
    assert!(storage
        .release_issue("missing", "alice", true)
        .unwrap_err()
        .to_string()
        .contains("not found"));
}

#[test]
fn cli_claim_alias_conflicts_release_and_explicit_administration() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("claims.db");
    let mut storage = SqliteStorage::new(&db).unwrap();
    create(&mut storage, "task", Status::Open, "");
    create(&mut storage, "closed", Status::Closed, "");
    create(&mut storage, "blocked", Status::Blocked, "");
    drop(storage);

    let claimed = success(cli(
        &db,
        "alice",
        &["update", "task", "--status", "in_progress"],
    ));
    assert_eq!(claimed.assignee, "alice");
    assert_eq!(claimed.status, Status::InProgress);
    assert_eq!(
        success(cli(&db, "alice", &["claim", "task"])).assignee,
        "alice"
    );
    conflict(cli(&db, "bob", &["claim", "task"]), "owned by 'alice'");
    conflict(
        cli(&db, "bob", &["update", "task", "--status", "in_progress"]),
        "owned by 'alice'",
    );
    conflict(
        cli(&db, "bob", &["update", "task", "--assignee", "bob"]),
        "requires update --force",
    );
    conflict(
        cli(
            &db,
            "alice",
            &[
                "update",
                "task",
                "--status",
                "in_progress",
                "--title",
                "changed",
            ],
        ),
        "Claim first",
    );
    let unchanged = success(cli(&db, "bob", &["show", "task"]));
    assert_eq!(unchanged.title, claimed.title);
    assert_eq!(unchanged.status, Status::InProgress);
    assert_eq!(unchanged.assignee, "alice");

    for id in ["closed", "blocked"] {
        conflict(cli(&db, "alice", &["claim", id]), "not ready");
        conflict(
            cli(&db, "alice", &["update", id, "--status", "in_progress"]),
            "not ready",
        );
    }
    conflict(cli(&db, "bob", &["release", "task"]), "owned by 'alice'");
    let released = success(cli(&db, "coordinator", &["release", "task", "--force"]));
    assert!(released.assignee.is_empty());
    assert_eq!(released.status, Status::Open);
    assert_eq!(success(cli(&db, "bob", &["claim", "task"])).assignee, "bob");
    let released = success(cli(&db, "bob", &["release", "task"]));
    assert!(released.assignee.is_empty());
    conflict(cli(&db, "bob", &["release", "task"]), "no owner");
    conflict(cli(&db, "", &["claim", "task"]), "nonempty actor");

    let reassigned = success(cli(
        &db,
        "coordinator",
        &[
            "update",
            "blocked",
            "--assignee",
            "carol",
            "--status",
            "in_progress",
            "--force",
        ],
    ));
    assert_eq!(reassigned.assignee, "carol");
    assert_eq!(reassigned.status, Status::InProgress);
    conflict(cli(&db, "bob", &["claim", "blocked"]), "owned by 'carol'");
}

#[test]
fn competing_cli_processes_report_one_winner_and_nonzero_conflict() {
    let dir = tempdir().unwrap();
    let db = dir.path().join("claims.db");
    let mut storage = SqliteStorage::new(&db).unwrap();
    create(&mut storage, "task", Status::Open, "");
    drop(storage);

    // No JSONL exists at startup: this isolates claims from stale-file sync races.
    // Both the explicit command and compatibility alias must use the same guard.
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = ["alice", "bob"]
        .into_iter()
        .map(|actor| {
            let db = db.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                let args: &[&str] = if actor == "alice" {
                    &["claim", "task"]
                } else {
                    &["update", "task", "--status", "in_progress"]
                };
                (actor, cli(&db, actor, args))
            })
        })
        .collect();
    let mut results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(
        results
            .iter()
            .filter(|(_, out)| out.status.success())
            .count(),
        1,
        "{results:?}"
    );
    let winner_index = results
        .iter()
        .position(|(_, out)| out.status.success())
        .unwrap();
    let (winner, output) = results.remove(winner_index);
    let claimed = success(output);
    assert_eq!(claimed.assignee, winner);
    assert_eq!(claimed.status, Status::InProgress);
    conflict(results.pop().unwrap().1, &format!("owned by '{winner}'"));
    let storage = SqliteStorage::new(&db).unwrap();
    assert_eq!(storage.get_issue("task").unwrap().unwrap().assignee, winner);
}
