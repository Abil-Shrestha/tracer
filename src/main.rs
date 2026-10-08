mod cli;

use anyhow::Result;
use clap::Parser;
use tracer::{find_database_path, find_jsonl_path, storage::sqlite::SqliteStorage};

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    // Handle init command separately (doesn't need existing database)
    if let cli::Commands::Init(args) = cli.command {
        return cli::init::execute(args);
    }

    // Handle learn command (doesn't need database)
    if let cli::Commands::Learn(args) = cli.command {
        return cli::learn::execute(args);
    }

    // Find database path
    let db_path = if let Some(path) = cli.db {
        path
    } else {
        find_database_path()?
    };

    // Open storage
    let mut storage: Box<dyn tracer::Storage> = Box::new(SqliteStorage::new(&db_path)?);

    // Get actor name
    let actor = if let Some(actor) = cli.actor {
        actor
    } else {
        tracer::get_actor()
    };

    // Get prefix from config or default to "bd"
    let prefix = storage.get_config("prefix")?.unwrap_or_else(|| "bd".to_string());

    // Auto-import if JSONL is newer
    let jsonl_path = find_jsonl_path(&db_path);
    if jsonl_path.exists() {
        let _ = cli::export::auto_import(&mut storage, &jsonl_path, &actor);
    }

    // Execute command
    let result = match cli.command {
        cli::Commands::Init(_) => unreachable!(), // Handled above
        cli::Commands::Learn(_) => unreachable!(), // Handled above
        
        cli::Commands::Create(args) => {
            cli::create::execute(args, &mut storage, &actor, &prefix, cli.json)
        }
        
        cli::Commands::List(args) => {
            cli::list::execute(args, storage.as_ref(), cli.json)
        }
        
        cli::Commands::Show(args) => {
            cli::show::execute(args, storage.as_ref(), cli.json)
        }
        
        cli::Commands::Update(args) => {
            cli::update::execute_update(args, &mut storage, &actor, cli.json)
        }

        cli::Commands::Claim(args) => {
            cli::claim::execute_claim(args, &mut storage, &actor, cli.json)
        }

        cli::Commands::Release(args) => {
            cli::claim::execute_release(args, &mut storage, &actor, cli.json)
        }
        
        cli::Commands::Close(args) => {
            cli::update::execute_close(args, &mut storage, &actor, cli.json)
        }
        
        cli::Commands::Comment(args) => {
            cli::update::execute_comment(args, &mut storage, &actor, cli.json)
        }
        
        cli::Commands::Ready(args) => {
            cli::ready::execute_ready(args, storage.as_ref(), cli.json)
        }
        
        cli::Commands::Blocked(args) => {
            cli::ready::execute_blocked(args, storage.as_ref(), cli.json)
        }
        
        cli::Commands::Dep(dep_cmd) => {
            match dep_cmd {
                cli::dep::DepCommands::Add(args) => {
                    cli::dep::execute_add(args, &mut storage, &actor, cli.json)
                }
                cli::dep::DepCommands::Remove(args) => {
                    cli::dep::execute_remove(args, &mut storage, &actor, cli.json)
                }
                cli::dep::DepCommands::Tree(args) => {
                    cli::dep::execute_tree(args, storage.as_ref(), cli.json)
                }
                cli::dep::DepCommands::Cycles => {
                    cli::dep::execute_cycles(storage.as_ref(), cli.json)
                }
            }
        }
        
        cli::Commands::Export(args) => {
            cli::export::execute_export(args, storage.as_ref())
        }
        
        cli::Commands::Import(args) => {
            cli::export::execute_import(args, &mut storage, &actor)
        }
        
        cli::Commands::Stats(args) => {
            cli::stats::execute(args, storage.as_ref(), cli.json)
        }
    };

    // Auto-export if there are dirty issues
    if result.is_ok() {
        let dirty = storage.get_dirty_issues()?;
        if !dirty.is_empty() {
            if let Err(e) = cli::export::auto_export(storage.as_ref(), &jsonl_path) {
                eprintln!("Warning: Failed to auto-export: {}", e);
            } else {
                // Clear dirty flags after successful export
                storage.clear_dirty_issues()?;
            }
        }
    }

    result
}

