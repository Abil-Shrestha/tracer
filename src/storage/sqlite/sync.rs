use super::SqliteStorage;
use crate::sync::{
    self, ImportOptions, ImportSummary, Resolution, SyncEvent, SyncRecord, BASE_KEY, HASH_KEY,
};
use crate::{types::*, Storage};
use anyhow::{bail, ensure, Result};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn migrate(conn: &Connection) -> Result<()> {
    let columns = conn
        .prepare("PRAGMA table_info(events)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    let tx = conn.unchecked_transaction()?;
    if !columns.iter().any(|c| c == "sync_id") {
        tx.execute("ALTER TABLE events ADD COLUMN sync_id TEXT", [])?;
    }
    if !columns.iter().any(|c| c == "original_id") {
        tx.execute("ALTER TABLE events ADD COLUMN original_id INTEGER", [])?;
    }
    tx.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_events_sync_id ON events(sync_id)",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

impl SqliteStorage {
    pub(super) fn read_sync_records(&self) -> Result<Vec<SyncRecord>> {
        let mut records = Vec::new();
        for mut issue in self.search_issues("", &IssueFilter::default())? {
            issue.dependencies = self.get_dependency_records(&issue.id)?;
            let mut stmt = self.conn.prepare(
                "SELECT COALESCE(original_id, id), issue_id, event_type, actor, old_value,
                        new_value, comment, created_at, sync_id FROM events WHERE issue_id = ?1",
            )?;
            let rows = stmt.query_map([&issue.id], |row| {
                Ok((
                    Event {
                        id: row.get(0)?,
                        issue_id: row.get(1)?,
                        event_type: row
                            .get::<_, String>(2)?
                            .parse()
                            .map_err(|_| rusqlite::Error::InvalidQuery)?,
                        actor: row.get(3)?,
                        old_value: row.get(4)?,
                        new_value: row.get(5)?,
                        comment: row.get(6)?,
                        created_at: row.get(7)?,
                    },
                    row.get::<_, Option<String>>(8)?,
                ))
            })?;
            let mut events = Vec::new();
            for row in rows {
                let (event, identity) = row?;
                // Old database events get a deterministic identity without a write.
                // Clones upgrading the same old history produce the same identity.
                let sync_id = match identity {
                    Some(id) => id,
                    None => format!(
                        "legacy-{}",
                        crate::utils::compute_hash(&serde_json::to_vec(&event)?)
                    ),
                };
                events.push(SyncEvent { sync_id, event });
            }
            let labels = self.get_labels(&issue.id)?;
            let mut record = SyncRecord {
                sync_version: Some(1),
                issue,
                labels: Some(labels),
                events: Some(events),
            };
            record.normalize();
            records.push(record);
        }
        records.sort_by(|a, b| a.issue.id.cmp(&b.issue.id));
        Ok(records)
    }

    pub(super) fn apply_snapshot(
        &self,
        records: &[SyncRecord],
        options: ImportOptions,
        file_hash: Option<&str>,
    ) -> Result<ImportSummary> {
        sync::validate(records)?;
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let local = as_map(self.read_sync_records()?);
        let base = as_map(match self.get_metadata(BASE_KEY)? {
            Some(data) => serde_json::from_str(&data)?,
            None => Vec::new(),
        });
        let legacy_ids: BTreeSet<_> = records
            .iter()
            .filter(|r| r.sync_version.is_none())
            .map(|r| r.issue.id.as_str())
            .collect();
        let mut incoming = as_map(records.to_vec());
        for (id, record) in &mut incoming {
            // Missing legacy collections mean "not represented", not "remove".
            // Even explicit incoming resolution must retain unrepresented history.
            let previous = local.get(id).or_else(|| base.get(id));
            if record.labels.is_none() {
                record.labels = Some(previous.and_then(|r| r.labels.clone()).unwrap_or_default());
            }
            if record.events.is_none() {
                record.events = Some(previous.and_then(|r| r.events.clone()).unwrap_or_default());
            }
            record.sync_version = Some(1);
            record.normalize();
            for known in [base.get(id), local.get(id)].into_iter().flatten() {
                ensure!(known.issue.created_at == record.issue.created_at,
                    "Identity collision for {id}: created_at differs. Rename the colliding issue and its references; resolution flags cannot override identity.");
            }
        }
        // Portable event IDs cannot be reassigned, even by explicit resolution.
        let mut events = BTreeMap::new();
        for record in base.values().chain(local.values()).chain(incoming.values()) {
            for event in record.events.iter().flatten() {
                let value = serde_json::to_value(event)?;
                if let Some(previous) = events.insert(&event.sync_id, value.clone()) {
                    ensure!(
                        previous == value,
                        "Event identity collision {}",
                        event.sync_id
                    );
                }
            }
        }

        let ids: BTreeSet<_> = base
            .keys()
            .chain(local.keys())
            .chain(incoming.keys())
            .cloned()
            .collect();
        let mut merged = BTreeMap::new();
        for id in ids {
            let (b, l, r) = (base.get(&id), local.get(&id), incoming.get(&id));
            let chosen = if options.skip_existing {
                l.or(r)
            } else if same(l, r)? || (r.is_none() && b.is_none()) {
                l
            } else if l.is_none() && b.is_none() {
                r
            } else {
                // A clean local record may already contain an exported branch edit.
                // Without cross-clone ancestry, accepting a changed record based on
                // timestamps or cleanliness could silently discard that edit.
                match options.resolution {
                    Resolution::Local => l,
                    Resolution::Incoming => r,
                    Resolution::Reject if same(r, b)? => l,
                    Resolution::Reject => bail!("Sync conflict for {id}: local and incoming records diverged. Export a local backup, resolve the JSONL, then import --resolve incoming (or local)."),
                }
            };
            if let Some(record) = chosen {
                merged.insert(id, record.clone());
            }
        }
        let merged_records: Vec<_> = merged.values().cloned().collect();
        sync::validate(&merged_records)?;
        for record in merged.values().chain(incoming.values()) {
            for dep in &record.issue.dependencies {
                ensure!(
                    merged.contains_key(&dep.depends_on_id),
                    "Missing dependency target {} for {}",
                    dep.depends_on_id,
                    record.issue.id
                );
            }
        }

        let mut summary = ImportSummary::default();
        // Insert/update every scalar row before relationships: order-independent refs.
        for (id, record) in &merged {
            if !same(local.get(id), Some(record))? {
                write_issue(&tx, &record.issue)?;
                summary.changed += 1;
            }
        }
        // Clear changed children before deleting issues. Referential validation above
        // prevents ON DELETE CASCADE from silently deleting a retained relationship.
        for (id, record) in &merged {
            if !same(local.get(id), Some(record))? {
                write_children(&tx, record)?;
            }
        }
        for id in local.keys().filter(|id| !merged.contains_key(*id)) {
            tx.execute("DELETE FROM issues WHERE id = ?1", [id])?;
            summary.removed += 1;
        }
        if let Some(hash) = file_hash {
            checkpoint(&tx, &incoming.values().cloned().collect::<Vec<_>>(), hash)?;
            for (id, record) in &merged {
                // A legacy file does not actually contain the filled-in history.
                // Publish v1 before treating that local data as synchronized.
                if !legacy_ids.contains(id.as_str()) && same(Some(record), incoming.get(id))? {
                    tx.execute("DELETE FROM dirty_issues WHERE issue_id = ?1", [id])?;
                } else {
                    tx.execute(
                        "INSERT OR IGNORE INTO dirty_issues(issue_id) VALUES (?1)",
                        [id],
                    )?;
                }
            }
        } else {
            // An explicit import must publish even when it removes the last issue.
            tx.execute(
                "INSERT OR REPLACE INTO metadata(key, value) VALUES ('sync_pending', '1')",
                [],
            )?;
            for (id, record) in &merged {
                if !same(local.get(id), Some(record))? {
                    tx.execute(
                        "INSERT OR IGNORE INTO dirty_issues(issue_id) VALUES (?1)",
                        [id],
                    )?;
                }
            }
        }
        if options.dry_run {
            tx.rollback()?;
        } else {
            tx.commit()?;
        }
        Ok(summary)
    }

    pub(super) fn acknowledge_sync(&self, records: &[SyncRecord], hash: &str) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        ensure!(
            serde_json::to_value(self.read_sync_records()?)? == serde_json::to_value(records)?,
            "Database changed during publication; dirty state retained"
        );
        checkpoint(&tx, records, hash)?;
        tx.execute("DELETE FROM dirty_issues", [])?;
        tx.execute("DELETE FROM metadata WHERE key = 'sync_pending'", [])?;
        tx.commit()?;
        Ok(())
    }
}

fn as_map(records: Vec<SyncRecord>) -> BTreeMap<String, SyncRecord> {
    records
        .into_iter()
        .map(|r| (r.issue.id.clone(), r))
        .collect()
}

fn same(a: Option<&SyncRecord>, b: Option<&SyncRecord>) -> Result<bool> {
    match (a, b) {
        (None, None) => Ok(true),
        (Some(a), Some(b)) => sync::equivalent(a, b),
        _ => Ok(false),
    }
}

fn checkpoint(tx: &Transaction<'_>, records: &[SyncRecord], hash: &str) -> Result<()> {
    for (key, value) in [
        (BASE_KEY, serde_json::to_string(records)?),
        (HASH_KEY, hash.to_string()),
    ] {
        tx.execute(
            "INSERT OR REPLACE INTO metadata(key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
    }
    Ok(())
}

fn write_issue(tx: &Transaction<'_>, i: &Issue) -> Result<()> {
    tx.execute(
        "INSERT INTO issues(id, title, description, design, acceptance_criteria, notes, status,
            priority, issue_type, assignee, estimated_minutes, created_at, updated_at, closed_at, external_ref)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(id) DO UPDATE SET title=excluded.title, description=excluded.description,
            design=excluded.design, acceptance_criteria=excluded.acceptance_criteria, notes=excluded.notes,
            status=excluded.status, priority=excluded.priority, issue_type=excluded.issue_type,
            assignee=excluded.assignee, estimated_minutes=excluded.estimated_minutes,
            updated_at=excluded.updated_at, closed_at=excluded.closed_at, external_ref=excluded.external_ref",
        params![i.id, i.title, i.description, i.design, i.acceptance_criteria, i.notes,
            i.status.to_string(), i.priority, i.issue_type.to_string(), i.assignee,
            i.estimated_minutes, i.created_at, i.updated_at, i.closed_at, i.external_ref])?;
    Ok(())
}

fn write_children(tx: &Transaction<'_>, record: &SyncRecord) -> Result<()> {
    let id = &record.issue.id;
    tx.execute("DELETE FROM dependencies WHERE issue_id = ?1", [id])?;
    for d in &record.issue.dependencies {
        tx.execute("INSERT INTO dependencies(issue_id, depends_on_id, type, created_at, created_by) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![d.issue_id, d.depends_on_id, d.dep_type.to_string(), d.created_at, d.created_by])?;
    }
    tx.execute("DELETE FROM labels WHERE issue_id = ?1", [id])?;
    for label in record.labels.iter().flatten() {
        tx.execute(
            "INSERT INTO labels(issue_id, label) VALUES (?1, ?2)",
            params![id, label],
        )?;
    }
    tx.execute("DELETE FROM events WHERE issue_id = ?1", [id])?;
    for e in record.events.iter().flatten() {
        let event = &e.event;
        tx.execute("INSERT INTO events(issue_id, event_type, actor, old_value, new_value, comment, created_at, sync_id, original_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, event.event_type.to_string(), event.actor, event.old_value, event.new_value,
                event.comment, event.created_at, e.sync_id, event.id])?;
    }
    Ok(())
}
