use super::{IssueUpdates, Storage};
use crate::types::*;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::collections::{HashMap, HashSet};
use std::path::Path;

mod sync;

pub struct SqliteStorage {
    conn: Connection,
}

impl SqliteStorage {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        
        // Enable WAL mode for better concurrency
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        // Initialize schema
        Self::init_schema(&conn)?;
        
        Ok(Self { conn })
    }

    /// Observe an existing local cache without initialization or migrations.
    /// One read transaction keeps multi-query context views consistent.
    pub fn open_read_only<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .context("Cannot open existing local cache; run tracer ready --json with the same database selection to initialize or refresh it")?;
        conn.execute_batch("BEGIN DEFERRED")?;
        Ok(Self { conn })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(SCHEMA)?;
        Self::migrate_tables(conn)?;
        sync::migrate(conn)?;
        Ok(())
    }

    fn migrate_tables(conn: &Connection) -> Result<()> {
        // Check if dirty_issues table exists
        let dirty_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='dirty_issues'",
                [],
                |row| row.get(0),
            )
            .map(|count: i64| count > 0)?;

        if !dirty_exists {
            conn.execute_batch(
                "CREATE TABLE dirty_issues (
                    issue_id TEXT PRIMARY KEY,
                    marked_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE CASCADE
                );
                CREATE INDEX idx_dirty_issues_marked_at ON dirty_issues(marked_at);"
            )?;
        }

        // Check if issue_counters table exists and sync
        let counters_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='issue_counters'",
                [],
                |row| row.get(0),
            )
            .map(|count: i64| count > 0)?;

        if !counters_exists {
            conn.execute_batch(
                "CREATE TABLE issue_counters (
                    prefix TEXT PRIMARY KEY,
                    last_id INTEGER NOT NULL DEFAULT 0
                );"
            )?;
        }

        // Check if external_ref column exists
        let has_external_ref: bool = conn
            .prepare("PRAGMA table_info(issues)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .any(|name| name == "external_ref");

        if !has_external_ref {
            conn.execute("ALTER TABLE issues ADD COLUMN external_ref TEXT", [])?;
        }

        // Check if metadata table exists
        let metadata_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='metadata'",
                [],
                |row| row.get(0),
            )
            .map(|count: i64| count > 0)?;

        if !metadata_exists {
            conn.execute_batch(
                "CREATE TABLE metadata (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL,
                    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
                );"
            )?;
        }

        Ok(())
    }

    fn get_next_id(&mut self, prefix: &str) -> Result<String> {
        // Use 128 bits of OS randomness so independent database copies don't share IDs.
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).context("Failed to generate a random issue ID")?;
        Ok(format!("{}-{}", prefix, hex::encode(random)))
    }

    fn mark_dirty(&mut self, issue_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO dirty_issues (issue_id) VALUES (?1)",
            params![issue_id],
        )?;
        Ok(())
    }

    fn add_event(&mut self, issue_id: &str, event_type: EventType, actor: &str, old_value: Option<&str>, new_value: Option<&str>, comment: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT INTO events (issue_id, event_type, actor, old_value, new_value, comment, created_at, sync_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, lower(hex(randomblob(16))))",
            params![
                issue_id,
                event_type.to_string(),
                actor,
                old_value,
                new_value,
                comment,
                Utc::now(),
            ],
        )?;
        Ok(())
    }
}

impl Storage for SqliteStorage {
    fn create_issue(&mut self, issue: &Issue, actor: &str) -> Result<()> {
        issue.validate()?;

        self.conn.execute(
            "INSERT INTO issues (id, title, description, design, acceptance_criteria, notes, status, priority, issue_type, assignee, estimated_minutes, created_at, updated_at, external_ref)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                issue.id,
                issue.title,
                issue.description,
                issue.design,
                issue.acceptance_criteria,
                issue.notes,
                issue.status.to_string(),
                issue.priority,
                issue.issue_type.to_string(),
                if issue.assignee.is_empty() { None } else { Some(&issue.assignee) },
                issue.estimated_minutes,
                issue.created_at,
                issue.updated_at,
                issue.external_ref,
            ],
        )?;

        self.add_event(&issue.id, EventType::Created, actor, None, None, None)?;
        self.mark_dirty(&issue.id)?;
        Ok(())
    }

    fn get_issue(&self, id: &str) -> Result<Option<Issue>> {
        let issue = self.conn
            .query_row(
                "SELECT id, title, description, design, acceptance_criteria, notes, status, priority, issue_type, assignee, estimated_minutes, created_at, updated_at, closed_at, external_ref
                 FROM issues WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Issue {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        description: row.get(2)?,
                        design: row.get(3)?,
                        acceptance_criteria: row.get(4)?,
                        notes: row.get(5)?,
                        status: row.get::<_, String>(6)?.parse().expect("Invalid status in database"),
                        priority: row.get(7)?,
                        issue_type: row.get::<_, String>(8)?.parse().expect("Invalid issue type in database"),
                        assignee: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                        estimated_minutes: row.get(10)?,
                        created_at: row.get(11)?,
                        updated_at: row.get(12)?,
                        closed_at: row.get(13)?,
                        external_ref: row.get(14)?,
                        dependencies: Vec::new(),
                    })
                },
            )
            .optional()?;
        Ok(issue)
    }

    fn claim_issue(&mut self, id: &str, actor: &str) -> Result<()> {
        if actor.trim().is_empty() {
            anyhow::bail!("Claim requires a nonempty actor");
        }

        // Reserve the write lock before checking ownership or blockers. All checks,
        // the claim, its audit event, and dirty tracking commit together.
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (status, owner): (String, String) = tx.query_row(
            "SELECT status, COALESCE(assignee, '') FROM issues WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or_else(|| anyhow::anyhow!("Issue {} not found", id))?;

        if !owner.is_empty() && owner != actor {
            anyhow::bail!("Claim conflict for issue {}: owned by '{}'", id, owner);
        }
        if status != "open" && !(status == "in_progress" && owner == actor) {
            anyhow::bail!("Claim conflict for issue {}: status '{}' is not ready", id, status);
        }

        let blocker: Option<String> = tx.query_row(
            "SELECT blocker.id FROM dependencies d
             JOIN issues blocker ON blocker.id = d.depends_on_id
             WHERE d.issue_id = ?1 AND d.type = 'blocks' AND blocker.status != 'closed'
             ORDER BY blocker.id LIMIT 1",
            params![id],
            |row| row.get(0),
        ).optional()?;
        if let Some(blocker) = blocker {
            anyhow::bail!("Claim conflict for issue {}: blocked by {}", id, blocker);
        }

        // Idempotent retries do not change timestamps or emit duplicate events.
        if status == "in_progress" && owner == actor {
            return Ok(());
        }

        let now = Utc::now();
        tx.execute(
            "UPDATE issues SET assignee = ?1, status = 'in_progress', updated_at = ?2,
             closed_at = NULL WHERE id = ?3",
            params![actor, now, id],
        )?;
        tx.execute(
            "INSERT INTO events (issue_id, event_type, actor, old_value, new_value, comment, created_at)
             VALUES (?1, 'updated', ?2, ?3, ?2, 'Claimed issue', ?4)",
            params![id, actor, owner, now],
        )?;
        tx.execute("INSERT OR IGNORE INTO dirty_issues (issue_id) VALUES (?1)", params![id])?;
        tx.commit()?;
        Ok(())
    }

    fn release_issue(&mut self, id: &str, actor: &str, force: bool) -> Result<()> {
        if actor.trim().is_empty() {
            anyhow::bail!("Release requires a nonempty actor");
        }

        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner: String = tx.query_row(
            "SELECT COALESCE(assignee, '') FROM issues WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ).optional()?.ok_or_else(|| anyhow::anyhow!("Issue {} not found", id))?;
        if owner.is_empty() {
            anyhow::bail!("Release conflict for issue {}: no owner", id);
        }
        if owner != actor && !force {
            anyhow::bail!(
                "Release conflict for issue {}: owned by '{}'; use --force only for manual recovery",
                id, owner
            );
        }

        let now = Utc::now();
        tx.execute(
            "UPDATE issues SET assignee = NULL,
             status = CASE WHEN status = 'in_progress' THEN 'open' ELSE status END,
             updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        tx.execute(
            "INSERT INTO events (issue_id, event_type, actor, old_value, new_value, comment, created_at)
             VALUES (?1, 'updated', ?2, ?3, '', ?4, ?5)",
            params![id, actor, owner, if force { "Force-released issue" } else { "Released issue" }, now],
        )?;
        tx.execute("INSERT OR IGNORE INTO dirty_issues (issue_id) VALUES (?1)", params![id])?;
        tx.commit()?;
        Ok(())
    }

    fn update_issue(&mut self, id: &str, updates: &IssueUpdates, actor: &str) -> Result<()> {
        let mut sql = String::from("UPDATE issues SET updated_at = ?1");
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(Utc::now())];
        let mut param_idx = 2;

        if let Some(title) = &updates.title {
            sql.push_str(&format!(", title = ?{}", param_idx));
            params.push(Box::new(title.clone()));
            param_idx += 1;
        }
        if let Some(desc) = &updates.description {
            sql.push_str(&format!(", description = ?{}", param_idx));
            params.push(Box::new(desc.clone()));
            param_idx += 1;
        }
        if let Some(design) = &updates.design {
            sql.push_str(&format!(", design = ?{}", param_idx));
            params.push(Box::new(design.clone()));
            param_idx += 1;
        }
        if let Some(ac) = &updates.acceptance_criteria {
            sql.push_str(&format!(", acceptance_criteria = ?{}", param_idx));
            params.push(Box::new(ac.clone()));
            param_idx += 1;
        }
        if let Some(notes) = &updates.notes {
            sql.push_str(&format!(", notes = ?{}", param_idx));
            params.push(Box::new(notes.clone()));
            param_idx += 1;
        }
        if let Some(status) = updates.status {
            sql.push_str(&format!(", status = ?{}", param_idx));
            params.push(Box::new(status.to_string()));
            param_idx += 1;
            self.add_event(id, EventType::StatusChanged, actor, None, Some(&status.to_string()), None)?;
        }
        if let Some(priority) = updates.priority {
            sql.push_str(&format!(", priority = ?{}", param_idx));
            params.push(Box::new(priority));
            param_idx += 1;
        }
        if let Some(issue_type) = updates.issue_type {
            sql.push_str(&format!(", issue_type = ?{}", param_idx));
            params.push(Box::new(issue_type.to_string()));
            param_idx += 1;
        }
        if let Some(assignee) = &updates.assignee {
            sql.push_str(&format!(", assignee = ?{}", param_idx));
            params.push(Box::new(assignee.clone()));
            param_idx += 1;
        }

        sql.push_str(&format!(" WHERE id = ?{}", param_idx));
        params.push(Box::new(id.to_string()));

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        self.conn.execute(&sql, params_refs.as_slice())?;

        self.add_event(id, EventType::Updated, actor, None, None, None)?;
        self.mark_dirty(id)?;
        Ok(())
    }

    fn close_issue(&mut self, id: &str, reason: &str, actor: &str) -> Result<()> {
        let now = Utc::now();
        self.conn.execute(
            "UPDATE issues SET status = 'closed', closed_at = ?1, updated_at = ?2 WHERE id = ?3",
            params![now, now, id],
        )?;
        self.add_event(id, EventType::Closed, actor, None, None, Some(reason))?;
        self.mark_dirty(id)?;
        Ok(())
    }

    fn search_issues(&self, _query: &str, filter: &IssueFilter) -> Result<Vec<Issue>> {
        let mut sql = String::from(
            "SELECT DISTINCT i.id, i.title, i.description, i.design, i.acceptance_criteria, i.notes, i.status, i.priority, i.issue_type, i.assignee, i.estimated_minutes, i.created_at, i.updated_at, i.closed_at, i.external_ref
             FROM issues i"
        );

        let mut conditions = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if filter.status.is_some() || filter.priority.is_some() || filter.issue_type.is_some() || filter.assignee.is_some() {
            if let Some(status) = filter.status {
                conditions.push(format!("i.status = ?{}", params.len() + 1));
                params.push(Box::new(status.to_string()));
            }
            if let Some(priority) = filter.priority {
                conditions.push(format!("i.priority = ?{}", params.len() + 1));
                params.push(Box::new(priority));
            }
            if let Some(issue_type) = filter.issue_type {
                conditions.push(format!("i.issue_type = ?{}", params.len() + 1));
                params.push(Box::new(issue_type.to_string()));
            }
            if let Some(assignee) = &filter.assignee {
                conditions.push(format!("i.assignee = ?{}", params.len() + 1));
                params.push(Box::new(assignee.clone()));
            }
        }

        if !filter.labels.is_empty() {
            sql.push_str(" LEFT JOIN labels l ON i.id = l.issue_id");
            let placeholders: Vec<String> = filter.labels.iter().enumerate()
                .map(|(idx, _)| format!("?{}", params.len() + idx + 1))
                .collect();
            conditions.push(format!("l.label IN ({})", placeholders.join(", ")));
            for label in &filter.labels {
                params.push(Box::new(label.clone()));
            }
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        sql.push_str(" ORDER BY i.priority ASC, i.created_at DESC");

        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let issues = stmt.query_map(params_refs.as_slice(), |row| {
            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                design: row.get(3)?,
                acceptance_criteria: row.get(4)?,
                notes: row.get(5)?,
                status: row.get::<_, String>(6)?.parse().expect("Invalid enum value in database"),
                priority: row.get(7)?,
                issue_type: row.get::<_, String>(8)?.parse().expect("Invalid enum value in database"),
                assignee: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                estimated_minutes: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                closed_at: row.get(13)?,
                external_ref: row.get(14)?,
                dependencies: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    fn add_dependency(&mut self, dep: &Dependency, actor: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO dependencies (issue_id, depends_on_id, type, created_at, created_by)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                dep.issue_id,
                dep.depends_on_id,
                dep.dep_type.to_string(),
                dep.created_at,
                actor,
            ],
        )?;
        self.add_event(&dep.issue_id, EventType::DependencyAdded, actor, None, Some(&dep.depends_on_id), None)?;
        self.mark_dirty(&dep.issue_id)?;
        Ok(())
    }

    fn remove_dependency(&mut self, issue_id: &str, depends_on_id: &str, actor: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM dependencies WHERE issue_id = ?1 AND depends_on_id = ?2",
            params![issue_id, depends_on_id],
        )?;
        self.add_event(issue_id, EventType::DependencyRemoved, actor, None, Some(depends_on_id), None)?;
        self.mark_dirty(issue_id)?;
        Ok(())
    }

    fn get_dependencies(&self, issue_id: &str) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.title, i.description, i.design, i.acceptance_criteria, i.notes, i.status, i.priority, i.issue_type, i.assignee, i.estimated_minutes, i.created_at, i.updated_at, i.closed_at, i.external_ref
             FROM issues i
             JOIN dependencies d ON i.id = d.depends_on_id
             WHERE d.issue_id = ?1"
        )?;
        
        let issues = stmt.query_map(params![issue_id], |row| {
            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                design: row.get(3)?,
                acceptance_criteria: row.get(4)?,
                notes: row.get(5)?,
                status: row.get::<_, String>(6)?.parse().expect("Invalid enum value in database"),
                priority: row.get(7)?,
                issue_type: row.get::<_, String>(8)?.parse().expect("Invalid enum value in database"),
                assignee: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                estimated_minutes: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                closed_at: row.get(13)?,
                external_ref: row.get(14)?,
                dependencies: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    fn get_dependents(&self, issue_id: &str) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.title, i.description, i.design, i.acceptance_criteria, i.notes, i.status, i.priority, i.issue_type, i.assignee, i.estimated_minutes, i.created_at, i.updated_at, i.closed_at, i.external_ref
             FROM issues i
             JOIN dependencies d ON i.id = d.issue_id
             WHERE d.depends_on_id = ?1"
        )?;
        
        let issues = stmt.query_map(params![issue_id], |row| {
            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                design: row.get(3)?,
                acceptance_criteria: row.get(4)?,
                notes: row.get(5)?,
                status: row.get::<_, String>(6)?.parse().expect("Invalid enum value in database"),
                priority: row.get(7)?,
                issue_type: row.get::<_, String>(8)?.parse().expect("Invalid enum value in database"),
                assignee: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                estimated_minutes: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                closed_at: row.get(13)?,
                external_ref: row.get(14)?,
                dependencies: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    fn get_dependency_records(&self, issue_id: &str) -> Result<Vec<Dependency>> {
        let mut stmt = self.conn.prepare(
            "SELECT issue_id, depends_on_id, type, created_at, created_by
             FROM dependencies WHERE issue_id = ?1"
        )?;
        
        let deps = stmt.query_map(params![issue_id], |row| {
            Ok(Dependency {
                issue_id: row.get(0)?,
                depends_on_id: row.get(1)?,
                dep_type: row.get::<_, String>(2)?.parse().expect("Invalid enum value in database"),
                created_at: row.get(3)?,
                created_by: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(deps)
    }

    fn get_all_dependency_records(&self) -> Result<Vec<Dependency>> {
        let mut stmt = self.conn.prepare(
            "SELECT issue_id, depends_on_id, type, created_at, created_by FROM dependencies"
        )?;
        
        let deps = stmt.query_map([], |row| {
            Ok(Dependency {
                issue_id: row.get(0)?,
                depends_on_id: row.get(1)?,
                dep_type: row.get::<_, String>(2)?.parse().expect("Invalid enum value in database"),
                created_at: row.get(3)?,
                created_by: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(deps)
    }

    fn get_dependency_tree(&self, issue_id: &str, max_depth: i32) -> Result<Vec<TreeNode>> {
        let mut nodes = Vec::new();
        let mut visited = HashSet::new();
        self.build_tree(issue_id, 0, max_depth, &mut nodes, &mut visited)?;
        Ok(nodes)
    }

    fn detect_cycles(&self) -> Result<Vec<Vec<Issue>>> {
        // Simple cycle detection using DFS
        let all_deps = self.get_all_dependency_records()?;
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        
        for dep in all_deps {
            graph.entry(dep.issue_id.clone()).or_default().push(dep.depends_on_id);
        }

        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        for node in graph.keys() {
            if !visited.contains(node) {
                self.dfs_cycle(node, &graph, &mut visited, &mut rec_stack, &mut Vec::new(), &mut cycles)?;
            }
        }

        Ok(cycles)
    }

    fn add_label(&mut self, issue_id: &str, label: &str, actor: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO labels (issue_id, label) VALUES (?1, ?2)",
            params![issue_id, label],
        )?;
        self.add_event(issue_id, EventType::LabelAdded, actor, None, Some(label), None)?;
        self.mark_dirty(issue_id)?;
        Ok(())
    }

    fn remove_label(&mut self, issue_id: &str, label: &str, actor: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM labels WHERE issue_id = ?1 AND label = ?2",
            params![issue_id, label],
        )?;
        self.add_event(issue_id, EventType::LabelRemoved, actor, None, Some(label), None)?;
        self.mark_dirty(issue_id)?;
        Ok(())
    }

    fn get_labels(&self, issue_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT label FROM labels WHERE issue_id = ?1")?;
        let labels = stmt.query_map(params![issue_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(labels)
    }

    fn get_issues_by_label(&self, label: &str) -> Result<Vec<Issue>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.title, i.description, i.design, i.acceptance_criteria, i.notes, i.status, i.priority, i.issue_type, i.assignee, i.estimated_minutes, i.created_at, i.updated_at, i.closed_at, i.external_ref
             FROM issues i
             JOIN labels l ON i.id = l.issue_id
             WHERE l.label = ?1"
        )?;
        
        let issues = stmt.query_map(params![label], |row| {
            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                design: row.get(3)?,
                acceptance_criteria: row.get(4)?,
                notes: row.get(5)?,
                status: row.get::<_, String>(6)?.parse().expect("Invalid enum value in database"),
                priority: row.get(7)?,
                issue_type: row.get::<_, String>(8)?.parse().expect("Invalid enum value in database"),
                assignee: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                estimated_minutes: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                closed_at: row.get(13)?,
                external_ref: row.get(14)?,
                dependencies: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    fn get_ready_work(&self, filter: &WorkFilter) -> Result<Vec<Issue>> {
        let mut sql = String::from(
            "SELECT DISTINCT i.id, i.title, i.description, i.design, i.acceptance_criteria, i.notes, i.status, i.priority, i.issue_type, i.assignee, i.estimated_minutes, i.created_at, i.updated_at, i.closed_at, i.external_ref
             FROM issues i
             WHERE i.status = 'open'
             AND i.id NOT IN (
                 SELECT d.issue_id
                 FROM dependencies d
                 JOIN issues blocker ON d.depends_on_id = blocker.id
                 WHERE d.type = 'blocks' AND blocker.status != 'closed'
             )"
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(priority) = filter.priority {
            sql.push_str(&format!(" AND i.priority = ?{}", params.len() + 1));
            params.push(Box::new(priority));
        }
        if let Some(assignee) = &filter.assignee {
            sql.push_str(&format!(" AND i.assignee = ?{}", params.len() + 1));
            params.push(Box::new(assignee.clone()));
        }

        sql.push_str(" ORDER BY i.priority ASC, i.created_at DESC");

        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {}", limit));
        }

        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let issues = stmt.query_map(params_refs.as_slice(), |row| {
            Ok(Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                design: row.get(3)?,
                acceptance_criteria: row.get(4)?,
                notes: row.get(5)?,
                status: row.get::<_, String>(6)?.parse().expect("Invalid enum value in database"),
                priority: row.get(7)?,
                issue_type: row.get::<_, String>(8)?.parse().expect("Invalid enum value in database"),
                assignee: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                estimated_minutes: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                closed_at: row.get(13)?,
                external_ref: row.get(14)?,
                dependencies: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    fn get_blocked_issues(&self) -> Result<Vec<BlockedIssue>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.title, i.description, i.design, i.acceptance_criteria, i.notes, i.status, i.priority, i.issue_type, i.assignee, i.estimated_minutes, i.created_at, i.updated_at, i.closed_at, i.external_ref,
                    COUNT(d.depends_on_id) as blocked_by_count
             FROM issues i
             JOIN dependencies d ON i.id = d.issue_id
             JOIN issues blocker ON d.depends_on_id = blocker.id
             WHERE d.type = 'blocks' AND blocker.status != 'closed'
             GROUP BY i.id"
        )?;
        
        let blocked = stmt.query_map([], |row| {
            let issue = Issue {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                design: row.get(3)?,
                acceptance_criteria: row.get(4)?,
                notes: row.get(5)?,
                status: row.get::<_, String>(6)?.parse().expect("Invalid enum value in database"),
                priority: row.get(7)?,
                issue_type: row.get::<_, String>(8)?.parse().expect("Invalid enum value in database"),
                assignee: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                estimated_minutes: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
                closed_at: row.get(13)?,
                external_ref: row.get(14)?,
                dependencies: Vec::new(),
            };
            let count: i32 = row.get(15)?;
            
            Ok(BlockedIssue {
                issue,
                blocked_by_count: count,
                blocked_by: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        // Get blockers for each blocked issue
        let mut result = Vec::new();
        for mut bi in blocked {
            let blockers: Vec<String> = self.conn.prepare(
                "SELECT d.depends_on_id
                 FROM dependencies d
                 JOIN issues blocker ON d.depends_on_id = blocker.id
                 WHERE d.issue_id = ?1 AND d.type = 'blocks' AND blocker.status != 'closed'"
            )?
            .query_map(params![&bi.issue.id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
            
            bi.blocked_by = blockers;
            result.push(bi);
        }

        Ok(result)
    }

    fn add_comment(&mut self, issue_id: &str, actor: &str, comment: &str) -> Result<()> {
        self.add_event(issue_id, EventType::Commented, actor, None, None, Some(comment))?;
        self.mark_dirty(issue_id)?;
        Ok(())
    }

    fn get_events(&self, issue_id: &str, limit: usize) -> Result<Vec<Event>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(original_id, id), issue_id, event_type, actor, old_value, new_value, comment, created_at
             FROM events
             WHERE issue_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2"
        )?;
        
        let events = stmt.query_map(params![issue_id, limit], |row| {
            Ok(Event {
                id: row.get(0)?,
                issue_id: row.get(1)?,
                event_type: row.get::<_, String>(2)?.parse().map_err(|_| rusqlite::Error::InvalidQuery)?,
                actor: row.get(3)?,
                old_value: row.get(4)?,
                new_value: row.get(5)?,
                comment: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(events)
    }

    fn get_statistics(&self) -> Result<Statistics> {
        let total: i32 = self.conn.query_row("SELECT COUNT(*) FROM issues", [], |row| row.get(0))?;
        let open: i32 = self.conn.query_row("SELECT COUNT(*) FROM issues WHERE status = 'open'", [], |row| row.get(0))?;
        let in_progress: i32 = self.conn.query_row("SELECT COUNT(*) FROM issues WHERE status = 'in_progress'", [], |row| row.get(0))?;
        let closed: i32 = self.conn.query_row("SELECT COUNT(*) FROM issues WHERE status = 'closed'", [], |row| row.get(0))?;
        let blocked: i32 = self.conn.query_row(
            "SELECT COUNT(DISTINCT i.id) FROM issues i
             JOIN dependencies d ON i.id = d.issue_id
             JOIN issues blocker ON d.depends_on_id = blocker.id
             WHERE d.type = 'blocks' AND blocker.status != 'closed'",
            [],
            |row| row.get(0)
        )?;
        
        let ready: i32 = self.conn.query_row(
            "SELECT COUNT(*) FROM issues i
             WHERE i.status = 'open'
             AND i.id NOT IN (
                 SELECT d.issue_id FROM dependencies d
                 JOIN issues blocker ON d.depends_on_id = blocker.id
                 WHERE d.type = 'blocks' AND blocker.status != 'closed'
             )",
            [],
            |row| row.get(0)
        )?;

        let avg_lead_time: f64 = self.conn.query_row(
            "SELECT AVG((julianday(closed_at) - julianday(created_at)) * 24)
             FROM issues WHERE closed_at IS NOT NULL",
            [],
            |row| row.get(0)
        ).unwrap_or(0.0);

        Ok(Statistics {
            total_issues: total,
            open_issues: open,
            in_progress_issues: in_progress,
            closed_issues: closed,
            blocked_issues: blocked,
            ready_issues: ready,
            average_lead_time_hours: avg_lead_time,
        })
    }

    fn sync_snapshot(&self) -> Result<Vec<crate::sync::SyncRecord>> {
        let tx = self.conn.unchecked_transaction()?;
        let records = self.read_sync_records()?;
        tx.commit()?;
        Ok(records)
    }

    fn import_snapshot(&mut self, records: &[crate::sync::SyncRecord], options: crate::sync::ImportOptions, file_hash: Option<&str>) -> Result<crate::sync::ImportSummary> {
        self.apply_snapshot(records, options, file_hash)
    }

    fn acknowledge_snapshot(&mut self, records: &[crate::sync::SyncRecord], file_hash: &str) -> Result<()> {
        self.acknowledge_sync(records, file_hash)
    }

    fn get_dirty_issues(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT issue_id FROM dirty_issues ORDER BY marked_at")?;
        let ids = stmt.query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    fn clear_dirty_issues(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM dirty_issues", [])?;
        Ok(())
    }

    fn clear_dirty_issues_by_id(&mut self, issue_ids: &[String]) -> Result<()> {
        if issue_ids.is_empty() {
            return Ok(());
        }
        let placeholders: Vec<String> = (1..=issue_ids.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!("DELETE FROM dirty_issues WHERE issue_id IN ({})", placeholders.join(", "));
        let params: Vec<&dyn rusqlite::ToSql> = issue_ids.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        self.conn.execute(&sql, params.as_slice())?;
        Ok(())
    }

    fn set_config(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    fn get_config(&self, key: &str) -> Result<Option<String>> {
        let value = self.conn
            .query_row("SELECT value FROM config WHERE key = ?1", params![key], |row| row.get(0))
            .optional()?;
        Ok(value)
    }

    fn set_metadata(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value, updated_at) VALUES (?1, ?2, ?3)",
            params![key, value, Utc::now()],
        )?;
        Ok(())
    }

    fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let value = self.conn
            .query_row("SELECT value FROM metadata WHERE key = ?1", params![key], |row| row.get(0))
            .optional()?;
        Ok(value)
    }

    fn generate_id(&mut self, prefix: &str) -> Result<String> {
        self.get_next_id(prefix)
    }
}

impl SqliteStorage {
    fn build_tree(&self, issue_id: &str, depth: i32, max_depth: i32, nodes: &mut Vec<TreeNode>, visited: &mut HashSet<String>) -> Result<()> {
        if depth >= max_depth || visited.contains(issue_id) {
            return Ok(());
        }

        visited.insert(issue_id.to_string());

        if let Some(issue) = self.get_issue(issue_id)? {
            nodes.push(TreeNode {
                issue: issue.clone(),
                depth,
                truncated: depth >= max_depth - 1,
            });

            let deps = self.get_dependencies(issue_id)?;
            for dep in deps {
                self.build_tree(&dep.id, depth + 1, max_depth, nodes, visited)?;
            }
        }

        Ok(())
    }

    fn dfs_cycle(&self, node: &str, graph: &HashMap<String, Vec<String>>, visited: &mut HashSet<String>, rec_stack: &mut HashSet<String>, path: &mut Vec<String>, cycles: &mut Vec<Vec<Issue>>) -> Result<()> {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(neighbors) = graph.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    self.dfs_cycle(neighbor, graph, visited, rec_stack, path, cycles)?;
                } else if rec_stack.contains(neighbor) {
                    // Cycle detected
                    let cycle_start = path.iter().position(|n| n == neighbor).expect("Cycle node must be in path");
                    let cycle_ids: Vec<String> = path[cycle_start..].to_vec();
                    let mut cycle_issues = Vec::new();
                    for id in cycle_ids {
                        if let Some(issue) = self.get_issue(&id)? {
                            cycle_issues.push(issue);
                        }
                    }
                    if !cycle_issues.is_empty() {
                        cycles.push(cycle_issues);
                    }
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
        Ok(())
    }

    pub fn generate_id(&mut self, prefix: &str) -> Result<String> {
        self.get_next_id(prefix)
    }
}

impl std::str::FromStr for EventType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "created" => Ok(EventType::Created),
            "updated" => Ok(EventType::Updated),
            "status_changed" => Ok(EventType::StatusChanged),
            "commented" => Ok(EventType::Commented),
            "closed" => Ok(EventType::Closed),
            "reopened" => Ok(EventType::Reopened),
            "dependency_added" => Ok(EventType::DependencyAdded),
            "dependency_removed" => Ok(EventType::DependencyRemoved),
            "label_added" => Ok(EventType::LabelAdded),
            "label_removed" => Ok(EventType::LabelRemoved),
            _ => anyhow::bail!("invalid event type: {}", s),
        }
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS issues (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    design TEXT NOT NULL DEFAULT '',
    acceptance_criteria TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open',
    priority INTEGER NOT NULL DEFAULT 2,
    issue_type TEXT NOT NULL DEFAULT 'task',
    assignee TEXT,
    estimated_minutes INTEGER,
    created_at DATETIME NOT NULL,
    updated_at DATETIME NOT NULL,
    closed_at DATETIME,
    external_ref TEXT
);

CREATE TABLE IF NOT EXISTS dependencies (
    issue_id TEXT NOT NULL,
    depends_on_id TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'blocks',
    created_at DATETIME NOT NULL,
    created_by TEXT NOT NULL,
    PRIMARY KEY (issue_id, depends_on_id),
    FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE CASCADE,
    FOREIGN KEY (depends_on_id) REFERENCES issues(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS labels (
    issue_id TEXT NOT NULL,
    label TEXT NOT NULL,
    PRIMARY KEY (issue_id, label),
    FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issue_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    old_value TEXT,
    new_value TEXT,
    comment TEXT,
    created_at DATETIME NOT NULL,
    FOREIGN KEY (issue_id) REFERENCES issues(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_issues_status ON issues(status);
CREATE INDEX IF NOT EXISTS idx_issues_priority ON issues(priority);
CREATE INDEX IF NOT EXISTS idx_issues_assignee ON issues(assignee);
CREATE INDEX IF NOT EXISTS idx_dependencies_issue ON dependencies(issue_id);
CREATE INDEX IF NOT EXISTS idx_dependencies_depends ON dependencies(depends_on_id);
CREATE INDEX IF NOT EXISTS idx_dependencies_type ON dependencies(type);
CREATE INDEX IF NOT EXISTS idx_labels_issue ON labels(issue_id);
CREATE INDEX IF NOT EXISTS idx_labels_label ON labels(label);
CREATE INDEX IF NOT EXISTS idx_events_issue ON events(issue_id);
"#;
