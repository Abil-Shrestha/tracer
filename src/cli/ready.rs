use super::summary::IssueSummary;
use anyhow::Result;
use clap::Args;
use std::io::Write;
use tracer::storage::Storage;
use tracer::types::*;

#[derive(Args)]
pub struct ReadyArgs {
    /// Filter by priority
    #[arg(long)]
    pub priority: Option<i32>,

    /// Filter by assignee
    #[arg(long)]
    pub assignee: Option<String>,

    /// Maximum number of results
    #[arg(long)]
    pub limit: Option<usize>,

    /// Return concise summaries instead of full issue records
    #[arg(long)]
    pub compact: bool,
}

#[derive(Args)]
pub struct BlockedArgs {}

pub fn execute_ready(
    args: ReadyArgs,
    storage: &dyn Storage,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let filter = WorkFilter {
        status: Status::Open,
        priority: args.priority,
        assignee: args.assignee,
        limit: args.limit,
    };

    let issues = storage.get_ready_work(&filter)?;

    if args.compact {
        let summaries: Vec<_> = issues.iter().map(IssueSummary::from).collect();
        if json {
            writeln!(output, "{}", serde_json::to_string(&summaries)?)?;
        } else if summaries.is_empty() {
            writeln!(output, "No ready work found")?;
        } else {
            for summary in summaries {
                writeln!(output, "{summary}")?;
            }
        }
    } else if json {
        writeln!(output, "{}", serde_json::to_string_pretty(&issues)?)?;
    } else {
        if issues.is_empty() {
            writeln!(output, "No ready work found")?;
            return Ok(());
        }

        use colored::Colorize;
        writeln!(
            output,
            "{} Ready work: {} issue(s)\n",
            "✓".green(),
            issues.len()
        )?;
        for issue in issues {
            write!(output, "{}", tracer::utils::format_issue(&issue, false))?;
            writeln!(output)?;
        }
    }

    Ok(())
}

pub fn execute_blocked(
    _args: BlockedArgs,
    storage: &dyn Storage,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let blocked = storage.get_blocked_issues()?;

    if json {
        writeln!(output, "{}", serde_json::to_string_pretty(&blocked)?)?;
    } else {
        if blocked.is_empty() {
            writeln!(output, "No blocked issues found")?;
            return Ok(());
        }

        use colored::Colorize;
        writeln!(
            output,
            "{} Blocked: {} issue(s)\n",
            "⚠".yellow(),
            blocked.len()
        )?;
        for bi in blocked {
            write!(output, "{}", tracer::utils::format_issue(&bi.issue, false))?;
            writeln!(
                output,
                "  {} Blocked by: {}",
                "⚠".red(),
                bi.blocked_by.join(", ")
            )?;
            writeln!(output)?;
        }
    }

    Ok(())
}
