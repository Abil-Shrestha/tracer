mod cli;

use anyhow::Result;
use clap::Parser;
use tracer::{find_database_path, storage::sqlite::SqliteStorage, sync::SyncSession};

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

    // Serialize the entire local read/import/mutate/publish/acknowledge cycle.
    let mut sync = SyncSession::open(&db_path)?;
    let mut storage: Box<dyn tracer::Storage> = Box::new(SqliteStorage::new(&db_path)?);

    // Get actor name
    let actor = if let Some(actor) = cli.actor {
        actor
    } else {
        tracer::get_actor()
    };

    // Get prefix from config or default to "bd"
    let prefix = storage.get_config("prefix")?.unwrap_or_else(|| "bd".to_string());

    // Explicit export is a local backup, and import is also the recovery path.
    // Neither may be blocked by an invalid/conflicting managed JSONL file.
    let explicit_sync = matches!(&cli.command, cli::Commands::Import(_) | cli::Commands::Export(_));
    let no_publish = matches!(&cli.command, cli::Commands::Export(_))
        || matches!(&cli.command, cli::Commands::Import(args) if args.dry_run);
    if !explicit_sync {
        sync.import(storage.as_mut())?;
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
            cli::export::execute_import(args, &mut storage)
        }
        
        cli::Commands::Stats(args) => {
            cli::stats::execute(args, storage.as_ref(), cli.json)
        }
    };

    // Publication failures are command failures. SQLite remains recoverable/dirty.
    if result.is_ok() && !no_publish {
        sync.publish(storage.as_mut(), explicit_sync)?;
    }

    result
}
