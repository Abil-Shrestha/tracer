//! Lossless JSONL snapshots and local CLI cycle serialization.
use crate::{types::*, Storage};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const BASE_KEY: &str = "sync_base_v1";
pub const HASH_KEY: &str = "sync_file_hash_v1";

/// Collections are optional only to read pre-v1 exports without erasing data
/// those exporters could not represent. V1 always writes complete collections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_version: Option<u32>,
    #[serde(flatten)]
    pub issue: Issue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<SyncEvent>>,
}

/// The original numeric ID is retained as data. sync_id is the portable identity;
/// the SQLite row ID is only a local surrogate and can differ after import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEvent {
    pub sync_id: String,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Resolution {
    #[default]
    Reject,
    Local,
    Incoming,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImportOptions {
    pub resolution: Resolution,
    pub skip_existing: bool,
    pub dry_run: bool,
}

#[derive(Debug, Default)]
pub struct ImportSummary {
    pub changed: usize,
    pub removed: usize,
}

impl SyncRecord {
    pub fn normalize(&mut self) {
        self.issue
            .dependencies
            .sort_by(|a, b| a.depends_on_id.cmp(&b.depends_on_id));
        if let Some(labels) = &mut self.labels {
            labels.sort();
        }
        if let Some(events) = &mut self.events {
            events.sort_by(|a, b| a.sync_id.cmp(&b.sync_id));
        }
    }
}

pub fn equivalent(a: &SyncRecord, b: &SyncRecord) -> Result<bool> {
    Ok(serde_json::to_value(a)? == serde_json::to_value(b)?)
}

pub fn parse(data: &[u8]) -> Result<Vec<SyncRecord>> {
    let text = std::str::from_utf8(data).context("JSONL must be UTF-8")?;
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)
            .with_context(|| format!("Invalid JSONL at line {}", index + 1))?;
        let object = value.as_object().context("JSONL records must be objects")?;
        // Unknown fields must not be silently dropped by an older client.
        for key in object.keys() {
            ensure!(
                matches!(
                    key.as_str(),
                    "sync_version"
                        | "id"
                        | "title"
                        | "description"
                        | "design"
                        | "acceptance_criteria"
                        | "notes"
                        | "status"
                        | "priority"
                        | "issue_type"
                        | "assignee"
                        | "estimated_minutes"
                        | "created_at"
                        | "updated_at"
                        | "closed_at"
                        | "external_ref"
                        | "dependencies"
                        | "labels"
                        | "events"
                ),
                "Unknown field {key} at line {}",
                index + 1
            );
        }
        for collection in ["dependencies", "events"] {
            if let Some(items) = object.get(collection).and_then(|v| v.as_array()) {
                for item in items {
                    let fields = item.as_object().context("Expected a collection object")?;
                    for key in fields.keys() {
                        let known = if collection == "dependencies" {
                            matches!(
                                key.as_str(),
                                "issue_id" | "depends_on_id" | "type" | "created_at" | "created_by"
                            )
                        } else {
                            matches!(
                                key.as_str(),
                                "sync_id"
                                    | "id"
                                    | "issue_id"
                                    | "event_type"
                                    | "actor"
                                    | "old_value"
                                    | "new_value"
                                    | "comment"
                                    | "created_at"
                            )
                        };
                        ensure!(
                            known,
                            "Unknown {collection} field {key} at line {}",
                            index + 1
                        );
                    }
                }
            }
        }
        let mut record: SyncRecord = serde_json::from_str(line)
            .with_context(|| format!("Invalid record at line {}", index + 1))?;
        record.normalize();
        records.push(record);
    }
    validate(&records)?;
    records.sort_by(|a, b| a.issue.id.cmp(&b.issue.id));
    Ok(records)
}

/// Validate the whole input before touching storage. References are checked
/// against the resulting snapshot in the storage transaction (forward refs work).
pub fn validate(records: &[SyncRecord]) -> Result<()> {
    let mut ids = HashSet::new();
    let mut event_ids = HashSet::new();
    for record in records {
        let issue = &record.issue;
        issue
            .validate()
            .with_context(|| format!("Invalid issue {}", issue.id))?;
        ensure!(!issue.id.trim().is_empty(), "Empty issue ID");
        ensure!(ids.insert(&issue.id), "Duplicate issue ID {}", issue.id);
        ensure!(
            record.sync_version.is_none() || record.sync_version == Some(1),
            "Unsupported sync version"
        );
        if record.sync_version == Some(1) {
            ensure!(
                record.labels.is_some() && record.events.is_some(),
                "Incomplete v1 record {}",
                issue.id
            );
        }
        ensure!(
            issue.updated_at >= issue.created_at,
            "updated_at precedes created_at for {}",
            issue.id
        );
        if let Some(closed) = issue.closed_at {
            ensure!(
                closed >= issue.created_at,
                "closed_at precedes created_at for {}",
                issue.id
            );
        }
        let mut targets = HashSet::new();
        for dep in &issue.dependencies {
            ensure!(
                dep.issue_id == issue.id,
                "Dependency owner mismatch for {}",
                issue.id
            );
            ensure!(
                dep.depends_on_id != issue.id,
                "Self dependency for {}",
                issue.id
            );
            ensure!(
                targets.insert(&dep.depends_on_id),
                "Duplicate dependency for {}",
                issue.id
            );
        }
        if let Some(labels) = &record.labels {
            let mut seen = HashSet::new();
            for label in labels {
                ensure!(
                    !label.trim().is_empty() && seen.insert(label),
                    "Empty or duplicate label for {}",
                    issue.id
                );
            }
        }
        if let Some(events) = &record.events {
            for event in events {
                ensure!(
                    event.event.issue_id == issue.id,
                    "Event owner mismatch for {}",
                    issue.id
                );
                ensure!(
                    !event.sync_id.is_empty() && event.event.id > 0,
                    "Invalid event identity for {}",
                    issue.id
                );
                ensure!(
                    event_ids.insert(&event.sync_id),
                    "Duplicate event identity {}",
                    event.sync_id
                );
            }
        }
    }
    Ok(())
}

pub fn encode(records: &[SyncRecord]) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    for record in records {
        serde_json::to_writer(&mut data, record)?;
        data.push(b'\n');
    }
    Ok(data)
}

/// Same-directory temporary file, fsync, atomic rename, then directory fsync.
/// Failures leave the old file intact or the complete new file, never a prefix.
pub fn publish_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure!(
        !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()),
        "Refusing to replace JSONL symlink {}",
        path.display()
    );
    let mut attempt = 0;
    let (temp, mut file) = loop {
        let temp = parent.join(format!(
            ".tracer-export-{}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => attempt += 1,
            Err(e) => return Err(e.into()),
        }
    };
    let result = (|| {
        if let Ok(metadata) = fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn file_hash(path: &Path) -> Result<Option<String>> {
    match fs::read(path) {
        Ok(data) => Ok(Some(crate::utils::compute_hash(&data))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// OS advisory lock for cooperating local CLI processes, never a cross-clone lock.
/// The lock file must not be unlinked: the kernel releases the lock on exit/crash.
pub struct SyncSession {
    _lock: File,
    pub path: PathBuf,
    observed_hash: Option<String>,
}

impl SyncSession {
    pub fn open(db_path: &Path) -> Result<Self> {
        let db_path = if db_path.exists() {
            fs::canonicalize(db_path)?
        } else {
            db_path.to_path_buf()
        };
        let parent = db_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(parent.join(".tracer-sync.lock"))?;
        lock.lock().context("Cannot lock local tracer sync cycle")?;
        let path = crate::find_jsonl_path(&db_path);
        let observed_hash = file_hash(&path)?;
        Ok(Self {
            _lock: lock,
            path,
            observed_hash,
        })
    }

    pub fn import(&self, storage: &mut dyn Storage) -> Result<()> {
        if storage.get_metadata(HASH_KEY)? == self.observed_hash {
            return Ok(());
        }
        let Some(expected) = &self.observed_hash else {
            bail!("Managed JSONL is missing; restore it or explicitly import a resolved snapshot");
        };
        let data = fs::read(&self.path)?;
        ensure!(
            crate::utils::compute_hash(&data) == *expected,
            "JSONL changed during sync; retry"
        );
        let records = parse(&data)?;
        storage.import_snapshot(&records, ImportOptions::default(), Some(expected))?;
        Ok(())
    }

    pub fn publish(&mut self, storage: &mut dyn Storage, force: bool) -> Result<()> {
        if !force
            && storage.get_dirty_issues()?.is_empty()
            && storage.get_metadata("sync_pending")?.is_none()
        {
            return Ok(());
        }
        ensure!(file_hash(&self.path)? == self.observed_hash, "JSONL changed during command; local changes remain dirty. Retry after resolving the file.");
        let records = storage.sync_snapshot()?;
        validate(&records)?;
        let data = encode(&records)?;
        publish_atomic(&self.path, &data)
            .context("JSONL publication failed; local changes remain in SQLite")?;
        let hash = crate::utils::compute_hash(&data);
        // If this fails or the process dies, the next import sees the same data;
        // no mutation events are synthesized, and dirty flags remain recoverable.
        storage.acknowledge_snapshot(&records, &hash)?;
        self.observed_hash = Some(hash);
        Ok(())
    }
}
