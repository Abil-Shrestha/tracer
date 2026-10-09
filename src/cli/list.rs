use super::summary::IssueSummary;
use anyhow::Result;
use clap::Args;
use std::io::Write;
use tracer::storage::Storage;
use tracer::types::*;

#[derive(Args)]
pub struct ListArgs {
    /// Filter by status
    #[arg(long, value_parser = clap::value_parser!(Status))]
    pub status: Option<Status>,

    /// Filter by priority
    #[arg(long)]
    pub priority: Option<i32>,

    /// Filter by issue type
    #[arg(long, value_parser = clap::value_parser!(IssueType))]
    pub issue_type: Option<IssueType>,

    /// Filter by assignee
    #[arg(long)]
    pub assignee: Option<String>,

    /// Filter by labels (comma-separated)
    #[arg(short, long, value_delimiter = ',')]
    pub labels: Vec<String>,

    /// Maximum number of results
    #[arg(long)]
    pub limit: Option<usize>,

    /// Return concise summaries instead of full issue records
    #[arg(long)]
    pub compact: bool,
}

pub fn execute(
    args: ListArgs,
    storage: &dyn Storage,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let filter = IssueFilter {
        status: args.status,
        priority: args.priority,
        issue_type: args.issue_type,
        assignee: args.assignee,
        labels: args.labels,
        limit: args.limit,
    };

    let issues = storage.search_issues("", &filter)?;

    if args.compact {
        let summaries: Vec<_> = issues.iter().map(IssueSummary::from).collect();
        if json {
            writeln!(output, "{}", serde_json::to_string(&summaries)?)?;
        } else if summaries.is_empty() {
            writeln!(output, "No issues found")?;
        } else {
            for summary in summaries {
                writeln!(output, "{summary}")?;
            }
        }
    } else if json {
        writeln!(output, "{}", serde_json::to_string_pretty(&issues)?)?;
    } else {
        if issues.is_empty() {
            writeln!(output, "No issues found")?;
            return Ok(());
        }

        writeln!(output, "Found {} issue(s):\n", issues.len())?;
        for issue in issues {
            write!(output, "{}", tracer::utils::format_issue(&issue, false))?;
            writeln!(output)?;
        }
    }

    Ok(())
}
