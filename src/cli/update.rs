use anyhow::{Context, Result};
use clap::Args;
use std::io::Write;
use tracer::storage::{IssueUpdates, Storage};
use tracer::types::*;

#[derive(Args)]
pub struct UpdateArgs {
    /// Issue ID
    pub id: String,

    /// New title
    #[arg(long)]
    pub title: Option<String>,

    /// New description
    #[arg(long)]
    pub description: Option<String>,

    /// New status (in_progress alone uses atomic claim)
    #[arg(long, value_parser = clap::value_parser!(Status))]
    pub status: Option<Status>,

    /// New priority
    #[arg(long)]
    pub priority: Option<i32>,

    /// New issue type
    #[arg(long, value_parser = clap::value_parser!(IssueType))]
    pub issue_type: Option<IssueType>,

    /// Administrative reassignment (requires --force; use claim/release for ownership)
    #[arg(long)]
    pub assignee: Option<String>,

    /// Explicit administrative override of claim/release checks
    #[arg(long)]
    pub force: bool,
}

#[derive(Args)]
pub struct CloseArgs {
    /// Issue IDs to close
    pub ids: Vec<String>,

    /// Reason for closing
    #[arg(long, default_value = "Completed")]
    pub reason: String,
}

#[derive(Args)]
pub struct CommentArgs {
    /// Issue ID
    pub id: String,

    /// Comment text
    pub comment: String,
}

pub fn execute_update(
    args: UpdateArgs,
    storage: &mut Box<dyn Storage>,
    actor: &str,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    if !args.force {
        if args.assignee.is_some() {
            anyhow::bail!("Use claim/release for ownership; administrative reassignment requires update --force");
        }
        if args.status == Some(Status::InProgress) {
            if args.title.is_some()
                || args.description.is_some()
                || args.priority.is_some()
                || args.issue_type.is_some()
            {
                anyhow::bail!("Claim first, then update other fields; a mixed in_progress update requires --force");
            }
            return super::claim::execute_claim(
                super::claim::ClaimArgs { id: args.id },
                storage,
                actor,
                json,
                output,
            );
        }
    }

    // Verify issue exists
    storage
        .get_issue(&args.id)?
        .context(format!("Issue {} not found", args.id))?;

    let updates = IssueUpdates {
        title: args.title,
        description: args.description,
        design: None,
        acceptance_criteria: None,
        notes: None,
        status: args.status,
        priority: args.priority,
        issue_type: args.issue_type,
        assignee: args.assignee,
        estimated_minutes: None,
        external_ref: None,
    };

    storage.update_issue(&args.id, &updates, actor)?;

    if json {
        let updated = storage
            .get_issue(&args.id)?
            .expect("Issue should exist after update");
        writeln!(output, "{}", serde_json::to_string_pretty(&updated)?)?;
    } else {
        use colored::Colorize;
        writeln!(output, "✓ Updated issue {}", args.id.bold().cyan())?;
    }

    Ok(())
}

pub fn execute_close(
    args: CloseArgs,
    storage: &mut Box<dyn Storage>,
    actor: &str,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let mut closed = Vec::new();

    for id in &args.ids {
        // Verify issue exists
        storage
            .get_issue(id)?
            .context(format!("Issue {} not found", id))?;

        storage.close_issue(id, &args.reason, actor)?;
        closed.push(id.clone());
    }

    if json {
        let issues: Vec<_> = closed
            .iter()
            .map(|id| {
                storage
                    .get_issue(id)?
                    .context(format!("Issue {id} not found after close"))
            })
            .collect::<Result<_>>()?;
        writeln!(output, "{}", serde_json::to_string_pretty(&issues)?)?;
    } else {
        use colored::Colorize;
        for id in closed {
            writeln!(output, "✓ Closed issue {}", id.bold().cyan())?;
        }
    }

    Ok(())
}

pub fn execute_comment(
    args: CommentArgs,
    storage: &mut Box<dyn Storage>,
    actor: &str,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    // Verify issue exists
    storage
        .get_issue(&args.id)?
        .context(format!("Issue {} not found", args.id))?;

    storage.add_comment(&args.id, actor, &args.comment)?;

    if json {
        let issue = storage.get_issue(&args.id)?.expect("Issue should exist");
        writeln!(output, "{}", serde_json::to_string_pretty(&issue)?)?;
    } else {
        use colored::Colorize;
        writeln!(output, "✓ Added comment to {}", args.id.bold().cyan())?;
    }

    Ok(())
}
