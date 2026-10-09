use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::Args;
use std::io::Write;
use tracer::{storage::Storage, types::EventType};

#[derive(Args)]
pub struct ShowArgs {
    /// Issue ID
    pub id: String,

    /// Include all events and comments instead of recent history
    #[arg(long)]
    pub full: bool,
}

pub fn execute(
    args: ShowArgs,
    storage: &dyn Storage,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let mut issue = storage
        .get_issue(&args.id)?
        .context(format!("Issue {} not found", args.id))?;
    issue.dependencies = storage.get_dependency_records(&args.id)?;
    let labels = storage.get_labels(&args.id)?;
    // SQLite LIMIT takes a signed integer. Read all history so comments are not
    // silently hidden by newer non-comment events, and --full has no 20-row cap.
    let mut history = storage.get_events(&args.id, i64::MAX as usize)?;
    history.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    let all_comments: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::Commented)
        .collect();
    let events = &history[..if args.full {
        history.len()
    } else {
        history.len().min(20)
    }];
    let comments = &all_comments[..if args.full {
        all_comments.len()
    } else {
        all_comments.len().min(5)
    }];
    let events_truncated = events.len() < history.len();
    let comments_truncated = comments.len() < all_comments.len();

    if json {
        // Keep the existing issue fields at the top level. Empty detail
        // collections are explicit, unlike the legacy Issue serialization.
        let mut details = serde_json::to_value(&issue)?;
        details["dependencies"] = serde_json::to_value(&issue.dependencies)?;
        details["labels"] = serde_json::to_value(&labels)?;
        details["events"] = serde_json::to_value(events)?;
        details["comments"] = serde_json::to_value(comments)?;
        details["events_truncated"] = events_truncated.into();
        details["comments_truncated"] = comments_truncated.into();
        writeln!(output, "{}", serde_json::to_string_pretty(&details)?)?;
    } else {
        write!(output, "{}", tracer::utils::format_issue(&issue, true))?;
        for (label, value) in [
            ("Design", &issue.design),
            ("Acceptance criteria", &issue.acceptance_criteria),
            ("Notes", &issue.notes),
        ] {
            if !value.is_empty() {
                writeln!(output, "\n  {label}: {value}")?;
            }
        }
        if let Some(reference) = &issue.external_ref {
            writeln!(output, "  External reference: {reference}")?;
        }
        if !labels.is_empty() {
            writeln!(output, "\n  Labels: {}", labels.join(", "))?;
        }
        if !issue.dependencies.is_empty() {
            writeln!(output, "\n  Dependencies:")?;
            for dep in &issue.dependencies {
                writeln!(
                    output,
                    "    {} → {} ({})",
                    dep.issue_id, dep.depends_on_id, dep.dep_type
                )?;
            }
        }
        if !comments.is_empty() {
            use colored::Colorize;
            writeln!(
                output,
                "\n  Comments ({} of {}):",
                comments.len(),
                all_comments.len()
            )?;
            for event in comments {
                writeln!(
                    output,
                    "    {} ({}): {:?}",
                    event.actor.cyan(),
                    format_time_ago(&event.created_at),
                    event.comment.as_deref().unwrap_or_default()
                )?;
            }
        }
        if !events.is_empty() {
            writeln!(
                output,
                "\n  Events ({} of {}):",
                events.len(),
                history.len()
            )?;
            for event in events {
                writeln!(
                    output,
                    "    [{}] {} by {}",
                    event.created_at.format("%Y-%m-%d %H:%M"),
                    event.event_type,
                    event.actor
                )?;
                if let Some(old) = &event.old_value {
                    writeln!(output, "      Old: {old}")?;
                }
                if let Some(new) = &event.new_value {
                    writeln!(output, "      New: {new}")?;
                }
                if let Some(comment) = &event.comment {
                    writeln!(output, "      {comment}")?;
                }
            }
        }
        if events_truncated || comments_truncated {
            writeln!(output, "\n  History truncated; use show --full with this issue ID for all events and comments.")?;
        }
    }
    Ok(())
}

fn format_time_ago(dt: &DateTime<Utc>) -> String {
    let duration = Utc::now().signed_duration_since(*dt);
    if duration.num_days() > 0 {
        format!("{} days ago", duration.num_days())
    } else if duration.num_hours() > 0 {
        format!("{} hours ago", duration.num_hours())
    } else if duration.num_minutes() > 0 {
        format!("{} min ago", duration.num_minutes())
    } else {
        "just now".to_string()
    }
}
