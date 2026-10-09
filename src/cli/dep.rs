use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Subcommand};
use std::io::Write;
use tracer::storage::Storage;
use tracer::types::*;

#[derive(Subcommand)]
pub enum DepCommands {
    /// Add a dependency
    Add(AddArgs),

    /// Remove a dependency
    Remove(RemoveArgs),

    /// Show dependency tree
    Tree(TreeArgs),

    /// Detect dependency cycles
    Cycles,
}

#[derive(Args)]
pub struct AddArgs {
    /// Issue ID that depends on another
    pub issue_id: String,

    /// Issue ID that is depended on
    pub depends_on_id: String,

    /// Dependency type
    #[arg(short = 't', long = "type", value_parser = clap::value_parser!(DependencyType), default_value = "blocks")]
    pub dep_type: DependencyType,
}

#[derive(Args)]
pub struct RemoveArgs {
    /// Issue ID that depends on another
    pub issue_id: String,

    /// Issue ID that is depended on
    pub depends_on_id: String,
}

#[derive(Args)]
pub struct TreeArgs {
    /// Root issue ID
    pub id: String,

    /// Maximum depth (default: 50)
    #[arg(long, default_value = "50")]
    pub max_depth: i32,
}

pub fn execute_add(
    args: AddArgs,
    storage: &mut Box<dyn Storage>,
    actor: &str,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    // Verify both issues exist
    storage
        .get_issue(&args.issue_id)?
        .context(format!("Issue {} not found", args.issue_id))?;
    storage
        .get_issue(&args.depends_on_id)?
        .context(format!("Issue {} not found", args.depends_on_id))?;

    let dep = Dependency {
        issue_id: args.issue_id.clone(),
        depends_on_id: args.depends_on_id.clone(),
        dep_type: args.dep_type,
        created_at: Utc::now(),
        created_by: actor.to_string(),
    };

    storage.add_dependency(&dep, actor)?;

    if json {
        writeln!(output, "{}", serde_json::to_string_pretty(&dep)?)?;
    } else {
        use colored::Colorize;
        writeln!(
            output,
            "✓ Added dependency: {} {} {}",
            args.issue_id.cyan(),
            format!("--{}-->", args.dep_type).yellow(),
            args.depends_on_id.cyan()
        )?;
    }

    Ok(())
}

pub fn execute_remove(
    args: RemoveArgs,
    storage: &mut Box<dyn Storage>,
    actor: &str,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    storage.remove_dependency(&args.issue_id, &args.depends_on_id, actor)?;

    if json {
        writeln!(output, "{{\"status\": \"removed\"}}")?;
    } else {
        use colored::Colorize;
        writeln!(
            output,
            "✓ Removed dependency: {} --x--> {}",
            args.issue_id.cyan(),
            args.depends_on_id.cyan()
        )?;
    }

    Ok(())
}

pub fn execute_tree(
    args: TreeArgs,
    storage: &dyn Storage,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let tree = storage.get_dependency_tree(&args.id, args.max_depth)?;

    if json {
        writeln!(output, "{}", serde_json::to_string_pretty(&tree)?)?;
    } else {
        if tree.is_empty() {
            writeln!(output, "No dependencies found for {}", args.id)?;
            return Ok(());
        }

        use colored::Colorize;
        writeln!(output, "Dependency tree for {}:\n", args.id.bold().cyan())?;

        for node in tree {
            let indent = "  ".repeat(node.depth as usize);
            let connector = if node.depth > 0 { "└─ " } else { "" };
            let truncated_marker = if node.truncated { " [...]" } else { "" };

            writeln!(
                output,
                "{}{}{} {} [P{}, {}]{}",
                indent,
                connector,
                node.issue.id.cyan(),
                node.issue.title,
                node.issue.priority,
                node.issue.status,
                truncated_marker
            )?;
        }
    }

    Ok(())
}

pub fn execute_cycles(storage: &dyn Storage, json: bool, output: &mut dyn Write) -> Result<()> {
    let cycles = storage.detect_cycles()?;

    if json {
        writeln!(output, "{}", serde_json::to_string_pretty(&cycles)?)?;
    } else {
        if cycles.is_empty() {
            use colored::Colorize;
            writeln!(output, "{} No dependency cycles detected", "✓".green())?;
            return Ok(());
        }

        use colored::Colorize;
        writeln!(output, "{} Found {} cycle(s):\n", "⚠".red(), cycles.len())?;

        for (i, cycle) in cycles.iter().enumerate() {
            writeln!(output, "Cycle {}:", i + 1)?;
            for issue in cycle {
                writeln!(output, "  → {} {}", issue.id.cyan(), issue.title)?;
            }
            writeln!(output)?;
        }
    }

    Ok(())
}
