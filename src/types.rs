use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Issue represents a trackable work item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub design: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub acceptance_criteria: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    pub status: Status,
    pub priority: i32,
    pub issue_type: IssueType,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub assignee: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<Dependency>,
}

impl Issue {
    /// Validate checks if the issue has valid field values
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.title.is_empty() {
            anyhow::bail!("title is required");
        }
        if self.title.len() > 500 {
            anyhow::bail!(
                "title must be 500 characters or less (got {})",
                self.title.len()
            );
        }
        if !(0..=4).contains(&self.priority) {
            anyhow::bail!("priority must be between 0 and 4 (got {})", self.priority);
        }
        if let Some(est) = self.estimated_minutes {
            if est < 0 {
                anyhow::bail!("estimated_minutes cannot be negative");
            }
        }
        Ok(())
    }
}

/// Status represents the current state of an issue
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Open,
    InProgress,
    Blocked,
    Closed,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Open => write!(f, "open"),
            Status::InProgress => write!(f, "in_progress"),
            Status::Blocked => write!(f, "blocked"),
            Status::Closed => write!(f, "closed"),
        }
    }
}

impl std::str::FromStr for Status {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "open" => Ok(Status::Open),
            "in_progress" => Ok(Status::InProgress),
            "blocked" => Ok(Status::Blocked),
            "closed" => Ok(Status::Closed),
            _ => anyhow::bail!("invalid status: {}", s),
        }
    }
}

/// IssueType categorizes the kind of work
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IssueType {
    Bug,
    Feature,
    #[default]
    Task,
    Epic,
    Chore,
}

impl fmt::Display for IssueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IssueType::Bug => write!(f, "bug"),
            IssueType::Feature => write!(f, "feature"),
            IssueType::Task => write!(f, "task"),
            IssueType::Epic => write!(f, "epic"),
            IssueType::Chore => write!(f, "chore"),
        }
    }
}

impl std::str::FromStr for IssueType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bug" => Ok(IssueType::Bug),
            "feature" => Ok(IssueType::Feature),
            "task" => Ok(IssueType::Task),
            "epic" => Ok(IssueType::Epic),
            "chore" => Ok(IssueType::Chore),
            _ => anyhow::bail!("invalid issue type: {}", s),
        }
    }
}

/// Dependency represents a relationship between issues
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub issue_id: String,
    pub depends_on_id: String,
    #[serde(rename = "type")]
    pub dep_type: DependencyType,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// DependencyType categorizes the relationship
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyType {
    Blocks,
    Related,
    ParentChild,
    DiscoveredFrom,
}

impl fmt::Display for DependencyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DependencyType::Blocks => write!(f, "blocks"),
            DependencyType::Related => write!(f, "related"),
            DependencyType::ParentChild => write!(f, "parent-child"),
            DependencyType::DiscoveredFrom => write!(f, "discovered-from"),
        }
    }
}

impl std::str::FromStr for DependencyType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "blocks" => Ok(DependencyType::Blocks),
            "related" => Ok(DependencyType::Related),
            "parent-child" => Ok(DependencyType::ParentChild),
            "discovered-from" => Ok(DependencyType::DiscoveredFrom),
            _ => anyhow::bail!("invalid dependency type: {}", s),
        }
    }
}

/// Label represents a tag on an issue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub issue_id: String,
    pub label: String,
}

/// Event represents an audit trail entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub issue_id: String,
    pub event_type: EventType,
    pub actor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// EventType categorizes audit trail events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Created,
    Updated,
    StatusChanged,
    Commented,
    Closed,
    Reopened,
    DependencyAdded,
    DependencyRemoved,
    LabelAdded,
    LabelRemoved,
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventType::Created => write!(f, "created"),
            EventType::Updated => write!(f, "updated"),
            EventType::StatusChanged => write!(f, "status_changed"),
            EventType::Commented => write!(f, "commented"),
            EventType::Closed => write!(f, "closed"),
            EventType::Reopened => write!(f, "reopened"),
            EventType::DependencyAdded => write!(f, "dependency_added"),
            EventType::DependencyRemoved => write!(f, "dependency_removed"),
            EventType::LabelAdded => write!(f, "label_added"),
            EventType::LabelRemoved => write!(f, "label_removed"),
        }
    }
}

/// BlockedIssue extends Issue with blocking information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedIssue {
    #[serde(flatten)]
    pub issue: Issue,
    pub blocked_by_count: i32,
    pub blocked_by: Vec<String>,
}

/// TreeNode represents a node in a dependency tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    #[serde(flatten)]
    pub issue: Issue,
    pub depth: i32,
    pub truncated: bool,
}

/// Statistics provides aggregate metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Statistics {
    pub total_issues: i32,
    pub open_issues: i32,
    pub in_progress_issues: i32,
    pub closed_issues: i32,
    pub blocked_issues: i32,
    pub ready_issues: i32,
    pub average_lead_time_hours: f64,
}

/// IssueFilter is used to filter issue queries
#[derive(Debug, Clone, Default)]
pub struct IssueFilter {
    pub status: Option<Status>,
    pub priority: Option<i32>,
    pub issue_type: Option<IssueType>,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub limit: Option<usize>,
}

/// WorkFilter is used to filter ready work queries
#[derive(Debug, Clone, Default)]
pub struct WorkFilter {
    pub status: Status,
    pub priority: Option<i32>,
    pub assignee: Option<String>,
    pub limit: Option<usize>,
}
