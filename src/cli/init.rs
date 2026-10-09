use anyhow::Result;
use clap::Args;
use std::io::Write;
use std::path::PathBuf;
use tracer::Storage;

#[derive(Args)]
pub struct InitArgs {
    /// ID prefix for issues (default: bd)
    #[arg(long, default_value = "bd")]
    pub prefix: String,

    /// Database path (default: .trace/<prefix>.db)
    #[arg(long)]
    pub path: Option<PathBuf>,
}

pub fn execute(args: InitArgs, json: bool, output: &mut dyn Write) -> Result<()> {
    let db_path = if let Some(path) = args.path {
        path
    } else {
        let current = std::env::current_dir()?;
        let trace_dir = current.join(".trace");
        std::fs::create_dir_all(&trace_dir)?;
        trace_dir.join(format!("{}.db", args.prefix))
    };

    // Init shares the same local lock as all other database commands.
    let _sync = tracer::sync::SyncSession::open(&db_path)?;
    // Create the database (schema is auto-initialized)
    let mut storage = tracer::storage::sqlite::SqliteStorage::new(&db_path)?;

    // Set the prefix in config
    storage.set_config("prefix", &args.prefix)?;

    let jsonl_path = tracer::utils::find_jsonl_path(&db_path);
    if json {
        writeln!(
            output,
            "{}",
            serde_json::json!({
                "status": "initialized", "database": db_path, "prefix": args.prefix,
                "jsonl": jsonl_path
            })
        )?;
    } else {
        writeln!(
            output,
            "✓ Initialized tracer database at {}",
            db_path.display()
        )?;
        writeln!(output, "  Prefix: {}", args.prefix)?;
        writeln!(output, "  JSONL: {}", jsonl_path.display())?;
        writeln!(output)?;
        writeln!(output, "Next steps:")?;
        writeln!(
            output,
            "  1. Create your first issue: tracer create \"My first task\""
        )?;
        writeln!(output, "  2. See ready work: tracer ready")?;
        writeln!(output, "  3. Add to git: git add .trace/issues.jsonl")?;
    }

    Ok(())
}
