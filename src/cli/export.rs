use anyhow::Result;
use clap::{Args, ValueEnum};
use std::io::{Read, Write};
use std::path::PathBuf;
use tracer::storage::Storage;
use tracer::sync::{self, ImportOptions, Resolution};
use tracer::types::Status;

#[derive(Args)]
pub struct ExportArgs {
    /// Export the local database without auto-import (default: stdout)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Filter by status (partial exports are not full sync snapshots)
    #[arg(long, value_parser = clap::value_parser!(Status))]
    pub status: Option<Status>,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ResolveArg {
    Local,
    Incoming,
}

#[derive(Args)]
pub struct ImportArgs {
    /// Input snapshot (default: stdin); bypasses automatic import for recovery
    #[arg(short, long)]
    pub input: Option<PathBuf>,

    /// Add new issues only; keep existing issues and ignore omissions
    #[arg(long, conflicts_with = "resolve")]
    pub skip_existing: bool,

    /// Explicitly choose a side for conflicting whole records (never ID collisions)
    #[arg(long, value_enum)]
    pub resolve: Option<ResolveArg>,

    /// Validate the complete import and roll it back; no JSONL publication
    #[arg(long)]
    pub dry_run: bool,
}

pub fn execute_export(
    args: ExportArgs,
    storage: &dyn Storage,
    output: &mut dyn Write,
) -> Result<()> {
    let mut records = storage.sync_snapshot()?;
    if let Some(status) = args.status {
        records.retain(|r| r.issue.status == status);
    }
    sync::validate(&records)?;
    let data = sync::encode(&records)?;
    if let Some(path) = args.output {
        sync::publish_atomic(&path, &data)?;
    } else {
        output.write_all(&data)?;
    }
    Ok(())
}

pub fn execute_import(
    args: ImportArgs,
    storage: &mut Box<dyn Storage>,
    json: bool,
    output: &mut dyn Write,
) -> Result<()> {
    let data = if let Some(path) = args.input {
        std::fs::read(path)?
    } else {
        let mut data = Vec::new();
        std::io::stdin().read_to_end(&mut data)?;
        data
    };
    let records = sync::parse(&data)?;
    let options = ImportOptions {
        resolution: match args.resolve {
            None => Resolution::Reject,
            Some(ResolveArg::Local) => Resolution::Local,
            Some(ResolveArg::Incoming) => Resolution::Incoming,
        },
        skip_existing: args.skip_existing,
        dry_run: args.dry_run,
    };
    let summary = storage.import_snapshot(&records, options, None)?;
    if json {
        writeln!(
            output,
            "{}",
            serde_json::json!({
                "status": if args.dry_run { "dry_run" } else { "imported" },
                "changed": summary.changed, "removed": summary.removed,
                "dry_run": args.dry_run
            })
        )?;
    } else {
        writeln!(
            output,
            "{}: {} changed, {} removed",
            if args.dry_run {
                "Dry run (rolled back)"
            } else {
                "Imported"
            },
            summary.changed,
            summary.removed
        )?;
    }
    Ok(())
}
