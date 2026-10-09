pub mod claim;
pub mod context;
pub mod create;
pub mod dep;
pub mod export;
pub mod init;
pub mod learn;
pub mod list;
pub mod ready;
pub mod show;
pub mod stats;
mod summary;
pub mod update;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tracer")]
#[command(about = "Lightweight issue tracker for AI agents", long_about = None)]
#[command(version)]
pub struct Cli {
    /// Path to database file (overrides $TRACE_DB and auto-discovery)
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    /// Actor name for audit trail (overrides $TRACE_ACTOR and $USER)
    #[arg(long, global = true)]
    pub actor: Option<String>,

    /// Output in JSON format
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new trace database
    Init(init::InitArgs),

    /// Learn how to use tracer for AI agents
    Learn(learn::LearnArgs),

    /// Create a new issue
    Create(create::CreateArgs),

    /// List issues
    List(list::ListArgs),

    /// Show issue details
    Show(show::ShowArgs),

    /// Update an issue
    Update(update::UpdateArgs),

    /// Atomically claim ready work for the current actor
    Claim(claim::ClaimArgs),

    /// Release ownership (owner-only unless --force is used for recovery)
    Release(claim::ReleaseArgs),

    /// Close an issue
    Close(update::CloseArgs),

    /// Show ready work (no blockers)
    Ready(ready::ReadyArgs),

    /// Read-only session resume from the existing local cache (does not sync or claim)
    Context(context::ContextArgs),

    /// Show blocked issues
    Blocked(ready::BlockedArgs),

    /// Manage dependencies
    #[command(subcommand)]
    Dep(dep::DepCommands),

    /// Export issues to JSONL
    Export(export::ExportArgs),

    /// Import issues from JSONL
    Import(export::ImportArgs),

    /// Show statistics
    Stats(stats::StatsArgs),

    /// Add a comment to an issue
    Comment(update::CommentArgs),
}
