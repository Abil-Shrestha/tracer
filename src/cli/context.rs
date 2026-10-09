use super::summary::IssueSummary;
use anyhow::{Context, Result};
use clap::Args;
use serde::Serialize;
use std::io::Write;
use tracer::{Issue, IssueFilter, Status, Storage, WorkFilter};

#[derive(Args)]
pub struct ContextArgs {
    /// Maximum items per section and blocker IDs per issue (at least 1)
    #[arg(long, default_value = "5", value_parser = clap::value_parser!(u32).range(1..))]
    pub limit: u32,
}

#[derive(Serialize)]
struct Section<T> {
    items: Vec<T>,
    truncated: bool,
}

#[derive(Serialize)]
struct BlockerSummary<'a> {
    #[serde(flatten)]
    issue: IssueSummary<'a>,
    blocked_by: Vec<&'a str>,
    blocked_by_truncated: bool,
}

fn section<T>(items: impl IntoIterator<Item = T>, limit: usize) -> Section<T> {
    let mut items: Vec<_> = items.into_iter().take(limit.saturating_add(1)).collect();
    let truncated = items.len() > limit;
    items.truncate(limit);
    Section { items, truncated }
}

pub fn execute(
    args: ContextArgs,
    storage: &dyn Storage,
    actor: &str,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    // Existing storage APIs keep this a CLI view, not a second scheduling system.
    let (mut owned, mut ready, blocked) = (|| -> Result<_> {
        Ok((
            storage.search_issues("", &IssueFilter { assignee: Some(actor.into()), ..Default::default() })?,
            storage.get_ready_work(&WorkFilter::default())?,
            storage.get_blocked_issues()?,
        ))
    })().context("Cannot read local cache; run tracer ready --json with the same database selection to migrate and refresh it, then retry context")?;
    owned.retain(|issue| !actor.is_empty() && issue.status != Status::Closed);
    ready.retain(|issue| issue.assignee.is_empty() || issue.assignee == actor);
    let order = |a: &Issue, b: &Issue| {
        a.priority
            .cmp(&b.priority)
            .then(b.created_at.cmp(&a.created_at))
            .then(a.id.cmp(&b.id))
    };
    owned.sort_by(order);
    ready.sort_by(order);
    let limit = args.limit as usize;
    // Compute blockers from all owned work before truncating the owned section.
    let blockers = section(
        owned.iter().filter_map(|issue| {
            let blocked_issue = blocked.iter().find(|entry| entry.issue.id == issue.id);
            if issue.status != Status::Blocked && blocked_issue.is_none() {
                return None;
            }
            let mut ids: Vec<_> = blocked_issue
                .into_iter()
                .flat_map(|entry| entry.blocked_by.iter().map(String::as_str))
                .collect();
            ids.sort_unstable();
            let blocked_by_truncated = ids.len() > limit;
            ids.truncate(limit);
            Some(BlockerSummary {
                issue: IssueSummary::from(issue),
                blocked_by: ids,
                blocked_by_truncated,
            })
        }),
        limit,
    );
    let owned = section(owned.iter().map(IssueSummary::from), limit);
    let ready = section(ready.iter().map(IssueSummary::from), limit);
    // Templates, not shell snippets containing untrusted issue IDs/actor names.
    // Every command must use the same --db and --actor options as this invocation.
    let guidance = [
        "Local cache only: incoming JSONL may be unapplied. No work was claimed.",
        "Use the same --db and --actor options for follow-up commands.",
        "Refresh/migrate the cache through normal sync: tracer ready --compact --limit 5 --json",
        "Inspect an issue or blocker: tracer show --full --json -- <ID>",
        "Claim only after inspection: tracer claim --json -- <ID> (may lose a race)",
        "If a section is truncated, rerun context with a larger --limit; show --full gives every dependency.",
    ];
    if json {
        writeln!(
            output,
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "actor": actor, "source": "local_cache", "limit": limit,
                "owned": owned, "ready": ready, "blockers": blockers,
                "next_commands": guidance
            }))?
        )?;
    } else {
        writeln!(
            output,
            "Actor: {:?}\nSource: local_cache (read-only)",
            actor
        )?;
        for (name, section) in [("Owned work", &owned), ("Ready work", &ready)] {
            writeln!(
                output,
                "\n{name} ({} shown{}):",
                section.items.len(),
                if section.truncated { ", truncated" } else { "" }
            )?;
            for issue in &section.items {
                writeln!(output, "  {issue}")?;
            }
        }
        writeln!(
            output,
            "\nBlockers for owned work ({} shown{}):",
            blockers.items.len(),
            if blockers.truncated {
                ", truncated"
            } else {
                ""
            }
        )?;
        for entry in &blockers.items {
            writeln!(output, "  {}", entry.issue)?;
            if entry.blocked_by.is_empty() {
                writeln!(
                    output,
                    "    Status is blocked; inspect history for the reason."
                )?;
            } else {
                writeln!(
                    output,
                    "    Blocked by: {:?}{}",
                    entry.blocked_by,
                    if entry.blocked_by_truncated {
                        " (truncated)"
                    } else {
                        ""
                    }
                )?;
            }
        }
        writeln!(output, "\nNext commands:")?;
        for line in guidance {
            writeln!(output, "  {line}")?;
        }
    }
    Ok(())
}
