use serde::Serialize;
use std::fmt;
use tracer::{Issue, IssueType, Status};

/// Shared projection for opt-in compact output and bounded resume context.
#[derive(Serialize)]
pub struct IssueSummary<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub status: Status,
    pub priority: i32,
    pub issue_type: IssueType,
    pub assignee: &'a str,
}

impl<'a> From<&'a Issue> for IssueSummary<'a> {
    fn from(issue: &'a Issue) -> Self {
        Self {
            id: &issue.id,
            title: &issue.title,
            status: issue.status,
            priority: issue.priority,
            issue_type: issue.issue_type,
            assignee: &issue.assignee,
        }
    }
}

impl fmt::Display for IssueSummary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Escape newlines and control characters: one issue is one text line.
        write!(
            f,
            "{} {:?} [P{}, {}, {}] assignee={:?}",
            self.id.escape_debug(),
            self.title,
            self.priority,
            self.issue_type,
            self.status,
            self.assignee
        )
    }
}
