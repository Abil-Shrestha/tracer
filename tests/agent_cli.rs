use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn cli(db: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tracer"))
        .current_dir(db.parent().unwrap())
        .env("TRACE_ACTOR", "alice")
        .env("NO_COLOR", "1")
        .arg("--db")
        .arg(db)
        .args(args)
        .output()
        .unwrap()
}

fn success(output: Output) -> Vec<u8> {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output.stdout
}

fn value(output: Output) -> Value {
    serde_json::from_slice(&success(output)).unwrap()
}

fn failure(output: Output, code: &str, exit: i32) -> Value {
    assert_eq!(output.status.code(), Some(exit), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], code);
    assert!(!error["error"]["message"].as_str().unwrap().is_empty());
    error
}

fn issue(id: &str, status: &str, assignee: &str, priority: i32) -> Value {
    json!({
        "sync_version": 1, "id": id, "title": format!("Title {id}"),
        "description": "Description with\nmultiple lines", "design": "A design",
        "acceptance_criteria": "Acceptance criteria", "notes": "Resume notes",
        "status": status, "priority": priority, "issue_type": "bug",
        "assignee": assignee, "estimated_minutes": 37, "external_ref": "upstream#42",
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-02T00:00:00Z",
        "dependencies": [], "labels": ["backend", "urgent"], "events": []
    })
}

fn dependency(owner: &str, target: &str, kind: &str) -> Value {
    json!({"issue_id": owner, "depends_on_id": target, "type": kind,
        "created_at": "2026-01-01T01:00:00Z", "created_by": "original-author"})
}

fn import(db: &Path, records: &[Value]) {
    let path = db.parent().unwrap().join("incoming.jsonl");
    let jsonl: String = records.iter().map(|record| format!("{record}\n")).collect();
    fs::write(&path, jsonl).unwrap();
    let result = value(cli(
        db,
        &["import", "--input", path.to_str().unwrap(), "--json"],
    ));
    assert_eq!(result["status"], "imported");
    assert_eq!(result["dry_run"], false);
}

#[test]
fn show_preserves_populated_fields_and_recovers_history_beyond_default_windows() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    let mut detail = issue("detail", "closed", "alice", 1);
    detail["closed_at"] = json!("2026-01-01T02:00:00Z");
    detail["dependencies"] = json!([dependency("detail", "blocker", "blocks")]);
    let events: Vec<_> = (1..=30).map(|i| json!({
        "sync_id": format!("history-{i}"), "id": i, "issue_id": "detail",
        "event_type": if i <= 7 { "commented" } else { "updated" },
        "actor": "history-author", "old_value": format!("old-{i}"), "new_value": format!("new-{i}"),
        "comment": format!("entry-{i}"), "created_at": format!("2026-01-01T00:{i:02}:00Z")
    })).collect();
    detail["events"] = json!(events);
    // Exactly at the event/comment boundaries must not be called truncated.
    let mut boundary = issue("boundary", "open", "", 3);
    boundary["events"] = json!((1..=20)
        .map(|i| json!({
            "sync_id": format!("boundary-{i}"), "id": i, "issue_id": "boundary",
            "event_type": if i <= 5 { "commented" } else { "updated" },
            "actor": "other", "comment": format!("boundary-{i}"),
            "created_at": format!("2026-01-01T00:{i:02}:00Z")
        }))
        .collect::<Vec<_>>());
    import(
        &db,
        &[detail.clone(), boundary, issue("blocker", "open", "bob", 0)],
    );

    let recent = value(cli(&db, &["show", "detail", "--json"]));
    for (field, expected) in detail.as_object().unwrap() {
        if !["sync_version", "events"].contains(&field.as_str()) {
            assert_eq!(&recent[field], expected, "field {field}");
        }
    }
    assert_eq!(recent["events"].as_array().unwrap().len(), 20);
    assert_eq!(recent["events"][0]["id"], 30);
    assert_eq!(recent["events"][19]["id"], 11);
    assert_eq!(recent["comments"].as_array().unwrap().len(), 5);
    assert_eq!(recent["comments"][0]["comment"], "entry-7");
    assert_eq!(recent["comments"][4]["comment"], "entry-3");
    assert_eq!(recent["events_truncated"], true);
    assert_eq!(recent["comments_truncated"], true);

    let full = value(cli(&db, &["show", "detail", "--full", "--json"]));
    assert_eq!(full["events"].as_array().unwrap().len(), 30);
    assert_eq!(full["comments"].as_array().unwrap().len(), 7);
    assert_eq!(full["events"][29]["id"], 1);
    assert_eq!(full["events"][29]["actor"], "history-author");
    assert_eq!(full["events"][29]["old_value"], "old-1");
    assert_eq!(full["events"][29]["new_value"], "new-1");
    assert_eq!(full["events_truncated"], false);
    assert_eq!(full["comments_truncated"], false);
    let text = String::from_utf8(success(cli(&db, &["show", "detail"]))).unwrap();
    assert!(text.contains("History truncated"));
    assert!(text.contains("entry-7"));
    let full_text = String::from_utf8(success(cli(&db, &["show", "detail", "--full"]))).unwrap();
    assert!(full_text.contains("Events (30 of 30)"));
    assert!(full_text.contains("Old: old-1"));
    assert!(full_text.contains("Acceptance criteria"));
    assert!(!full_text.contains("History truncated"));

    let exact = value(cli(&db, &["show", "boundary", "--json"]));
    assert_eq!(exact["events_truncated"], false);
    assert_eq!(exact["comments_truncated"], false);
    assert_eq!(
        exact,
        value(cli(&db, &["show", "boundary", "--full", "--json"]))
    );
    let empty = value(cli(&db, &["show", "blocker", "--json"]));
    assert_eq!(empty["dependencies"], json!([]));
    assert_eq!(empty["comments"], json!([]));
    assert_eq!(empty["events"], json!([]));
}

#[test]
fn compact_nonempty_results_keep_filters_order_and_legacy_json() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    let mut high = issue("high", "open", "alice", 0);
    high["title"] = json!("High\npriority");
    let mut blocked = issue("blocked", "open", "alice", 0);
    blocked["dependencies"] = json!([dependency("blocked", "high", "blocks")]);
    import(
        &db,
        &[
            high,
            issue("low", "open", "alice", 4),
            blocked,
            issue("other", "open", "bob", 1),
        ],
    );
    for command in ["list", "ready"] {
        let plain = value(cli(
            &db,
            &[command, "--assignee", "alice", "--priority", "4", "--json"],
        ));
        assert_eq!(plain.as_array().unwrap().len(), 1);
        assert_eq!(plain[0]["id"], "low");
        assert_eq!(plain[0]["description"], "Description with\nmultiple lines");
        assert!(plain[0].get("created_at").is_some());
        assert!(plain[0].get("labels").is_none()); // Legacy shape is unchanged.
        let compact = value(cli(
            &db,
            &[
                command,
                "--assignee",
                "alice",
                "--priority",
                "4",
                "--compact",
                "--json",
            ],
        ));
        assert_eq!(
            compact,
            json!([{"id": "low", "title": "Title low", "priority": 4,
            "status": "open", "issue_type": "bug", "assignee": "alice"}])
        );
        let text = String::from_utf8(success(cli(
            &db,
            &[
                command,
                "--assignee",
                "alice",
                "--priority",
                "4",
                "--compact",
            ],
        )))
        .unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.contains("low") && text.contains("P4") && text.contains("alice"));
        assert!(
            value(cli(&db, &[command, "--limit", "0", "--compact", "--json"]))
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    let ready = value(cli(
        &db,
        &[
            "ready",
            "--assignee",
            "alice",
            "--compact",
            "--limit",
            "1",
            "--json",
        ],
    ));
    assert_eq!(ready.as_array().unwrap().len(), 1);
    assert_eq!(ready[0]["id"], "high");
    let text = String::from_utf8(success(cli(
        &db,
        &["ready", "--assignee", "alice", "--compact", "--limit", "1"],
    )))
    .unwrap();
    assert_eq!(text.lines().count(), 1); // A newline in the title does not add a row.
    assert!(text.contains("High\\npriority"));
}

#[test]
fn json_errors_cover_parse_runtime_and_sync_without_breaking_help() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    for args in [
        vec!["--json", "wat"],
        vec!["show", "--json"],
        vec!["list", "--status", "nonsense", "--json"],
        vec!["--json", "ready", "--limit", "oops"],
        vec!["context", "--limit", "0", "--json"],
        vec!["--json"],
    ] {
        failure(cli(&db, &args), "invalid_arguments", 2);
    }
    assert!(!db.exists());
    for args in [
        ["--json", "--help"],
        ["--json", "--version"],
        ["help", "show"],
    ] {
        assert!(!success(cli(&db, &args)).is_empty());
    }
    let error = failure(cli(&db, &["show", "absent", "--json"]), "command_failed", 1);
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));
    value(cli(
        &db,
        &["create", "Existing", "--id", "existing", "--json"],
    ));
    failure(
        cli(
            &db,
            &["create", "Invalid priority", "--priority", "9", "--json"],
        ),
        "command_failed",
        1,
    );
    success(cli(&db, &["--actor", "bob", "claim", "existing"]));
    let denied = failure(
        cli(&db, &["claim", "existing", "--json"]),
        "command_failed",
        1,
    );
    assert!(denied["error"]["message"]
        .as_str()
        .unwrap()
        .contains("owned by 'bob'"));
    fs::write(dir.path().join("issues.jsonl"), "not JSON\n").unwrap();
    failure(cli(&db, &["ready", "--json"]), "sync_import_failed", 1);
    failure(cli(dir.path(), &["list", "--json"]), "database_error", 1);
}

#[test]
fn init_and_import_json_successes_are_objects_and_export_stays_jsonl() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    let init = value(cli(
        &db,
        &[
            "init",
            "--path",
            db.to_str().unwrap(),
            "--prefix",
            "work",
            "--json",
        ],
    ));
    assert_eq!(
        init,
        json!({"status": "initialized", "database": db,
        "prefix": "work", "jsonl": dir.path().join("issues.jsonl")})
    );
    import(
        &db,
        &[
            issue("first", "open", "", 1),
            issue("second", "open", "", 2),
        ],
    );
    let before = fs::read(dir.path().join("issues.jsonl")).unwrap();
    let db_before = fs::read(&db).unwrap();
    let result = value(cli(
        &db,
        &[
            "import",
            "--input",
            dir.path().join("incoming.jsonl").to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    ));
    assert_eq!(
        result,
        json!({"status": "dry_run", "dry_run": true, "changed": 0, "removed": 0})
    );
    assert_eq!(fs::read(&db).unwrap(), db_before);
    assert_eq!(fs::read(dir.path().join("issues.jsonl")).unwrap(), before);
    let exported = String::from_utf8(success(cli(&db, &["export", "--json"]))).unwrap();
    assert_eq!(exported.lines().count(), 2);
    for line in exported.lines() {
        assert_eq!(
            serde_json::from_str::<Value>(line).unwrap()["sync_version"],
            1
        );
    }
}

#[cfg(unix)]
#[test]
fn publication_failure_has_no_success_output_and_recovery_does_not_repeat_mutations() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    value(cli(&db, &["create", "A task", "--id", "task", "--json"]));
    let jsonl = dir.path().join("issues.jsonl");
    let saved = dir.path().join("saved.jsonl");
    fs::rename(&jsonl, &saved).unwrap();
    std::os::unix::fs::symlink(&saved, &jsonl).unwrap();
    let before = fs::read(&saved).unwrap();
    failure(
        cli(&db, &["comment", "task", "JSON survives", "--json"]),
        "sync_publish_failed",
        1,
    );
    let text = cli(&db, &["comment", "task", "Text survives"]);
    assert!(!text.status.success());
    assert!(text.stdout.is_empty());
    assert!(String::from_utf8_lossy(&text.stderr).contains("sync_publish_failed"));
    assert_eq!(fs::read(&saved).unwrap(), before);
    // A read-only resume can see dirty work without attempting publication.
    value(cli(&db, &["context", "--json"]));
    assert_eq!(fs::read(&saved).unwrap(), before);
    let backup: Value = serde_json::from_slice(&success(cli(&db, &["export"]))).unwrap();
    assert_eq!(backup["events"].as_array().unwrap().len(), 3);
    fs::remove_file(&jsonl).unwrap();
    fs::rename(&saved, &jsonl).unwrap();
    value(cli(&db, &["ready", "--json"])); // Publish dirty work, not another comment.
    let details = value(cli(&db, &["show", "task", "--full", "--json"]));
    assert_eq!(details["comments"].as_array().unwrap().len(), 2);
}

#[test]
fn context_is_actor_specific_bounded_read_only_and_explicitly_local() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("test.db");
    let mut blocked = issue("owned-blocked", "open", "alice", 1);
    blocked["dependencies"] = json!([
        dependency("owned-blocked", "bob-active", "blocks"),
        dependency("owned-blocked", "free-high", "blocks"),
        dependency("owned-blocked", "closed-target", "blocks"),
        dependency("owned-blocked", "free-low", "related")
    ]);
    import(
        &db,
        &[
            issue("owned-active", "in_progress", "alice", 0),
            blocked,
            issue("owned-manual", "blocked", "alice", 2),
            issue("owned-open", "open", "alice", 3),
            issue("owned-closed", "closed", "alice", 0),
            issue("bob-active", "in_progress", "bob", 0),
            issue("bob-open", "open", "bob", 0),
            issue("bob-blocked", "blocked", "bob", 1),
            issue("closed-target", "closed", "bob", 1),
            issue("free-high", "open", "", 1),
            issue("free-low", "open", "", 4),
        ],
    );
    let db_before = fs::read(&db).unwrap();
    let managed = dir.path().join("issues.jsonl");
    // Prove context does not attempt automatic import or publication, even
    // when the incoming file is invalid and normal commands refuse to run.
    fs::write(&managed, "<<<<<<< unresolved incoming JSONL\n").unwrap();
    let small = value(cli(&db, &["context", "--limit", "1", "--json"]));
    assert_eq!(small["actor"], "alice");
    assert_eq!(small["source"], "local_cache");
    assert_eq!(small["limit"], 1);
    assert_eq!(small["owned"]["items"][0]["id"], "owned-active");
    assert_eq!(small["ready"]["items"][0]["id"], "free-high");
    assert_eq!(small["blockers"]["items"][0]["id"], "owned-blocked");
    assert_eq!(
        small["blockers"]["items"][0]["blocked_by"],
        json!(["bob-active"])
    );
    assert_eq!(small["blockers"]["items"][0]["blocked_by_truncated"], true);
    for key in ["owned", "ready", "blockers"] {
        assert_eq!(small[key]["items"].as_array().unwrap().len(), 1);
        assert_eq!(small[key]["truncated"], true);
    }
    let full = value(cli(&db, &["context", "--json"]));
    assert_eq!(full["limit"], 5);
    let ids = |section: &Value| {
        section["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&full["owned"]),
        [
            "owned-active",
            "owned-blocked",
            "owned-manual",
            "owned-open"
        ]
    );
    assert_eq!(ids(&full["ready"]), ["free-high", "owned-open", "free-low"]);
    assert_eq!(ids(&full["blockers"]), ["owned-blocked", "owned-manual"]);
    assert_eq!(
        full["blockers"]["items"][0]["blocked_by"],
        json!(["bob-active", "free-high"])
    );
    assert_eq!(full["blockers"]["items"][0]["blocked_by_truncated"], false);
    assert_eq!(full["blockers"]["items"][1]["blocked_by"], json!([]));
    for key in ["owned", "ready", "blockers"] {
        assert_eq!(full[key]["truncated"], false);
    }
    let guidance = full["next_commands"].to_string();
    for text in [
        "tracer ready",
        "tracer show",
        "tracer claim",
        "--limit",
        "unapplied",
        "--actor",
        "--db",
    ] {
        assert!(guidance.contains(text), "{guidance}");
    }
    let bob = value(cli(&db, &["--actor", "bob", "context", "--json"]));
    assert_eq!(bob["actor"], "bob");
    assert!(ids(&bob["owned"]).iter().all(|id| id.starts_with("bob-")));
    assert_eq!(bob["ready"]["items"][0]["id"], "bob-open");
    let text = String::from_utf8(success(cli(&db, &["context", "--limit", "1"]))).unwrap();
    for expected in [
        "alice",
        "local_cache",
        "owned-active",
        "owned-blocked",
        "truncated",
        "tracer claim",
        "No work was claimed",
    ] {
        assert!(text.contains(expected), "{text}");
    }
    assert_eq!(fs::read(&db).unwrap(), db_before);
    assert_eq!(
        fs::read(&managed).unwrap(),
        b"<<<<<<< unresolved incoming JSONL\n"
    );
}

#[test]
fn context_never_initializes_or_migrates_and_gives_recovery_guidance() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("missing.db");
    let missing = failure(cli(&db, &["context", "--json"]), "database_error", 1);
    assert!(!db.exists());
    assert!(!dir.path().join(".tracer-sync.lock").exists());
    assert!(missing["error"]["message"]
        .as_str()
        .unwrap()
        .contains("tracer ready"));
    // Simulate an older/uninitialized cache without silently upgrading it.
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch("CREATE TABLE old_cache (id TEXT)")
        .unwrap();
    let before = fs::read(&db).unwrap();
    let old = failure(cli(&db, &["context", "--json"]), "command_failed", 1);
    assert!(old["error"]["message"]
        .as_str()
        .unwrap()
        .contains("migrate and refresh"));
    assert_eq!(fs::read(&db).unwrap(), before);
    value(cli(&db, &["ready", "--json"]));
    let empty = value(cli(&db, &["context", "--json"]));
    assert_eq!(empty["owned"], json!({"items": [], "truncated": false}));
}
