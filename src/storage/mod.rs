pub mod sqlite;

use crate::types::*;
use anyhow::Result;

/// Storage defines the interface for issue storage backends
pub trait Storage {
    // Issues
    fn create_issue(&mut self, issue: &Issue, actor: &str) -> Result<()>;
    fn get_issue(&self, id: &str) -> Result<Option<Issue>>;
    /// Atomically claim ready work for a nonempty actor. Another owner, a closed/blocked
    /// status, or an unfinished blocking dependency causes an error without changes.
    /// Retrying an in-progress claim by the same actor is a no-op if still unblocked.
    fn claim_issue(&mut self, id: &str, actor: &str) -> Result<()>;
    /// Clear ownership, returning in-progress work to open but preserving other statuses.
    /// Only the owner may release unless force is explicitly requested for recovery.
    fn release_issue(&mut self, id: &str, actor: &str, force: bool) -> Result<()>;
    /// Administrative mutation, including import/reassignment; not a claim operation.
    /// Workers must use claim_issue/release_issue for exclusive local ownership.
    fn update_issue(&mut self, id: &str, updates: &IssueUpdates, actor: &str) -> Result<()>;
    fn close_issue(&mut self, id: &str, reason: &str, actor: &str) -> Result<()>;
    fn search_issues(&self, query: &str, filter: &IssueFilter) -> Result<Vec<Issue>>;

    // Dependencies
    fn add_dependency(&mut self, dep: &Dependency, actor: &str) -> Result<()>;
    fn remove_dependency(&mut self, issue_id: &str, depends_on_id: &str, actor: &str) -> Result<()>;
    fn get_dependencies(&self, issue_id: &str) -> Result<Vec<Issue>>;
    fn get_dependents(&self, issue_id: &str) -> Result<Vec<Issue>>;
    fn get_dependency_records(&self, issue_id: &str) -> Result<Vec<Dependency>>;
    fn get_all_dependency_records(&self) -> Result<Vec<Dependency>>;
    fn get_dependency_tree(&self, issue_id: &str, max_depth: i32) -> Result<Vec<TreeNode>>;
    fn detect_cycles(&self) -> Result<Vec<Vec<Issue>>>;

    // Labels
    fn add_label(&mut self, issue_id: &str, label: &str, actor: &str) -> Result<()>;
    fn remove_label(&mut self, issue_id: &str, label: &str, actor: &str) -> Result<()>;
    fn get_labels(&self, issue_id: &str) -> Result<Vec<String>>;
    fn get_issues_by_label(&self, label: &str) -> Result<Vec<Issue>>;

    // Ready Work & Blocking
    fn get_ready_work(&self, filter: &WorkFilter) -> Result<Vec<Issue>>;
    fn get_blocked_issues(&self) -> Result<Vec<BlockedIssue>>;

    // Events
    fn add_comment(&mut self, issue_id: &str, actor: &str, comment: &str) -> Result<()>;
    fn get_events(&self, issue_id: &str, limit: usize) -> Result<Vec<Event>>;

    // Statistics
    fn get_statistics(&self) -> Result<Statistics>;

    // Dirty tracking (for incremental JSONL export)
    fn get_dirty_issues(&self) -> Result<Vec<String>>;
    fn clear_dirty_issues(&mut self) -> Result<()>;
    fn clear_dirty_issues_by_id(&mut self, issue_ids: &[String]) -> Result<()>;

    // Config
    fn set_config(&mut self, key: &str, value: &str) -> Result<()>;
    fn get_config(&self, key: &str) -> Result<Option<String>>;

    // Metadata (for internal state like import hashes)
    fn set_metadata(&mut self, key: &str, value: &str) -> Result<()>;
    fn get_metadata(&self, key: &str) -> Result<Option<String>>;

    // ID generation
    fn generate_id(&mut self, prefix: &str) -> Result<String>;
}

/// IssueUpdates represents fields that can be updated on an issue
#[derive(Debug, Clone, Default)]
pub struct IssueUpdates {
    pub title: Option<String>,
    pub description: Option<String>,
    pub design: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub notes: Option<String>,
    pub status: Option<Status>,
    pub priority: Option<i32>,
    pub issue_type: Option<IssueType>,
    pub assignee: Option<String>,
    pub estimated_minutes: Option<Option<i32>>, // None = don't update, Some(None) = clear field
    pub external_ref: Option<Option<String>>,
}

