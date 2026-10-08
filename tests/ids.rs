use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(db: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tracer"))
        .arg("--db")
        .arg(db)
        .args(["--actor", "id-test", "--json"])
        .args(args)
        .current_dir(db.parent().unwrap())
        .output()
        .unwrap()
}

fn succeed(db: &Path, args: &[&str]) -> Output {
    let output = run(db, args);
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn json(db: &Path, args: &[&str]) -> Value {
    serde_json::from_slice(&succeed(db, args).stdout).unwrap()
}

fn init(dir: &Path, prefix: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let db = dir.join("issues.db");
    succeed(
        &db,
        &["init", "--path", db.to_str().unwrap(), "--prefix", prefix],
    );
    db
}

fn assert_generated_id(id: &str, prefix: &str) {
    let suffix = id.strip_prefix(&format!("{prefix}-")).unwrap();
    assert_eq!(suffix.len(), 32, "expected 128-bit hex ID: {id}");
    assert!(suffix
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
}

fn assert_identity(actual: &Value, expected: &Value) {
    for field in ["id", "title", "created_at"] {
        assert_eq!(actual[field], expected[field], "{field} changed");
    }
}

#[test]
fn independent_databases_create_distinct_ids() {
    let root = tempfile::tempdir().unwrap();
    let first = init(&root.path().join("first"), "team-api");
    let second = init(&root.path().join("second"), "team-api");
    let mut ids = HashSet::new();

    for _ in 0..16 {
        for db in [&first, &second] {
            let issue = json(db, &["create", "Independent task"]);
            let id = issue["id"].as_str().unwrap();
            assert!(ids.insert(id.to_owned()), "duplicate ID: {id}");
            assert_identity(&json(db, &["show", id]), &issue);
        }
    }
    assert_eq!(ids.len(), 32);
    for id in &ids {
        assert_generated_id(id, "team-api");
    }
}

#[test]
fn database_copies_create_distinct_ids_without_remapping_shared_issues() {
    let root = tempfile::tempdir().unwrap();
    let original = init(&root.path().join("original"), "bd");
    let legacy = json(&original, &["create", "Shared legacy task", "--id", "bd-7"]);
    let shared = json(&original, &["create", "Shared generated task"]);

    // Each CLI process has exited, so SQLite has checkpointed and closed the DB.
    let mut databases = vec![original.clone()];
    for name in ["copy-a", "copy-b"] {
        let dir = root.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        for file in ["issues.db", "issues.jsonl"] {
            std::fs::copy(original.parent().unwrap().join(file), dir.join(file)).unwrap();
        }
        databases.push(dir.join("issues.db"));
    }

    let shared_id = shared["id"].as_str().unwrap();
    let mut ids = HashSet::from(["bd-7".to_owned(), shared_id.to_owned()]);
    for _ in 0..8 {
        for db in &databases {
            let issue = json(db, &["create", "New task after copying"]);
            let id = issue["id"].as_str().unwrap();
            assert!(
                ids.insert(id.to_owned()),
                "duplicate ID after copying: {id}"
            );
            assert_identity(&json(db, &["show", id]), &issue);
        }
    }
    for db in &databases {
        assert_identity(&json(db, &["show", "bd-7"]), &legacy);
        assert_identity(&json(db, &["show", shared_id]), &shared);
    }
    assert_eq!(ids.len(), 26);
    for id in ids.iter().filter(|id| *id != "bd-7") {
        assert_generated_id(id, "bd");
    }
}

#[test]
fn legacy_and_explicit_ids_remain_usable_and_duplicates_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let db = init(root.path(), "bd");

    for id in ["bd-1", "CUSTOM-legacy-42"] {
        let original = json(&db, &["create", "Explicit task", "--id", id]);
        assert_eq!(original["id"], id);
        assert_identity(&json(&db, &["show", id]), &original);

        let duplicate = run(&db, &["create", "Must not replace", "--id", id]);
        assert!(!duplicate.status.success(), "duplicate ID was accepted");
        assert_identity(&json(&db, &["show", id]), &original);

        let updated = json(&db, &["update", id, "--title", "Updated explicit task"]);
        assert_eq!(updated["id"], id);
        assert_eq!(updated["title"], "Updated explicit task");
        assert_eq!(updated["created_at"], original["created_at"]);

        let dependent = json(
            &db,
            &[
                "create",
                "Dependent task",
                "--deps",
                &format!("blocks:{id}"),
            ],
        );
        let dependent_id = dependent["id"].as_str().unwrap();
        assert_generated_id(dependent_id, "bd");
        let blocked = json(&db, &["blocked"]);
        assert!(blocked.as_array().unwrap().iter().any(|issue| {
            issue["id"] == dependent_id && issue["blocked_by"] == serde_json::json!([id])
        }));

        let closed = json(&db, &["close", id]);
        assert_eq!(closed[0]["id"], id);
        assert_eq!(closed[0]["status"], "closed");
        assert_eq!(closed[0]["created_at"], original["created_at"]);
        let ready = json(&db, &["ready"]);
        assert!(ready
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue["id"] == dependent_id));
    }
}
