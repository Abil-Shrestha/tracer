use anyhow::Result;
use clap::Args;
use std::io::Write;
use tracer::storage::Storage;

#[derive(Args)]
pub struct StatsArgs {}

pub fn execute(
    _args: StatsArgs,
    storage: &dyn Storage,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let stats = storage.get_statistics()?;

    if json {
        writeln!(output, "{}", serde_json::to_string_pretty(&stats)?)?;
    } else {
        use colored::Colorize;

        writeln!(output, "{}", "Issue Statistics".bold())?;
        writeln!(output)?;
        writeln!(output, "  Total Issues:      {}", stats.total_issues)?;
        writeln!(
            output,
            "  Open:              {}",
            stats.open_issues.to_string().green()
        )?;
        writeln!(
            output,
            "  In Progress:       {}",
            stats.in_progress_issues.to_string().blue()
        )?;
        writeln!(
            output,
            "  Blocked:           {}",
            stats.blocked_issues.to_string().red()
        )?;
        writeln!(
            output,
            "  Closed:            {}",
            stats.closed_issues.to_string().dimmed()
        )?;
        writeln!(output)?;
        writeln!(
            output,
            "  Ready to Work:     {}",
            stats.ready_issues.to_string().bold().green()
        )?;
        writeln!(output)?;
        writeln!(
            output,
            "  Avg Lead Time:     {:.1} hours",
            stats.average_lead_time_hours
        )?;
    }

    Ok(())
}
