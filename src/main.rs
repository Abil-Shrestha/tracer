mod cli;

use anyhow::{Context, Result};
use clap::{error::ErrorKind, Parser};
use std::io::Write;
use std::process::ExitCode;
use tracer::{find_database_path, storage::sqlite::SqliteStorage, sync::SyncSession};

#[derive(Debug, Clone, Copy, thiserror::Error)]
enum Failure {
    #[error("database_error")]
    Database,
    #[error("sync_import_failed")]
    Import,
    #[error("command_failed")]
    Command,
    #[error("sync_publish_failed")]
    Publish,
    #[error("output_failed")]
    Output,
}

fn report_error(json: bool, code: &str, message: &str) {
    if json {
        eprintln!(
            "{}",
            serde_json::json!({"error": {"code": code, "message": message}})
        );
    } else {
        eprintln!("Error: {message}");
    }
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    // Parsing can fail before Clap constructs Cli. Do not treat positional text
    // following `--` as an output-mode flag.
    let json = args
        .iter()
        .skip(1)
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json");
    let cli = match cli::Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                error.exit();
            }
            if json {
                report_error(true, "invalid_arguments", &error.to_string());
            } else {
                let _ = error.print();
            }
            return ExitCode::from(2);
        }
    };
    let json = cli.json;
    // No command success is visible until the local publish/acknowledge cycle
    // succeeds. An error can still mean a mutation is durable in SQLite.
    let mut output = Vec::new();
    let result = run(cli, &mut output).and_then(|()| {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&output).context(Failure::Output)?;
        stdout.flush().context(Failure::Output)
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let code = error
                .downcast_ref::<Failure>()
                .copied()
                .unwrap_or(Failure::Command);
            report_error(json, &code.to_string(), &format!("{error:#}"));
            ExitCode::FAILURE
        }
    }
}

fn run(cli: cli::Cli, output: &mut dyn Write) -> Result<()> {
    if let cli::Commands::Init(args) = cli.command {
        return cli::init::execute(args, cli.json, output).context(Failure::Command);
    }
    if let cli::Commands::Learn(args) = cli.command {
        return cli::learn::execute(args).context(Failure::Command);
    }

    let db_path = match cli.db {
        Some(path) => path,
        None => find_database_path().context(Failure::Database)?,
    };
    let actor = cli.actor.unwrap_or_else(tracer::get_actor);

    // Resume context observes only the existing local cache, including dirty
    // work. It never imports, publishes, initializes a database, or claims work.
    if let cli::Commands::Context(args) = cli.command {
        let storage = SqliteStorage::open_read_only(&db_path).context(Failure::Database)?;
        return cli::context::execute(args, &storage, &actor, cli.json, output)
            .context(Failure::Command);
    }

    // Serialize the entire local read/import/mutate/publish/acknowledge cycle.
    let mut sync = SyncSession::open(&db_path).context(Failure::Database)?;
    let mut storage: Box<dyn tracer::Storage> =
        Box::new(SqliteStorage::new(&db_path).context(Failure::Database)?);
    let prefix = storage
        .get_config("prefix")
        .context(Failure::Database)?
        .unwrap_or_else(|| "bd".to_string());

    // Explicit export is a local backup, and import is also the recovery path.
    let explicit_sync = matches!(
        &cli.command,
        cli::Commands::Import(_) | cli::Commands::Export(_)
    );
    let no_publish = matches!(&cli.command, cli::Commands::Export(_))
        || matches!(&cli.command, cli::Commands::Import(args) if args.dry_run);
    if !explicit_sync {
        sync.import(storage.as_mut()).context(Failure::Import)?;
    }

    match cli.command {
        cli::Commands::Init(_) | cli::Commands::Learn(_) | cli::Commands::Context(_) => {
            unreachable!()
        }
        cli::Commands::Create(args) => {
            cli::create::execute(args, &mut storage, &actor, &prefix, cli.json, output)
        }
        cli::Commands::List(args) => cli::list::execute(args, storage.as_ref(), cli.json, output),
        cli::Commands::Show(args) => cli::show::execute(args, storage.as_ref(), cli.json, output),
        cli::Commands::Update(args) => {
            cli::update::execute_update(args, &mut storage, &actor, cli.json, output)
        }
        cli::Commands::Claim(args) => {
            cli::claim::execute_claim(args, &mut storage, &actor, cli.json, output)
        }
        cli::Commands::Release(args) => {
            cli::claim::execute_release(args, &mut storage, &actor, cli.json, output)
        }
        cli::Commands::Close(args) => {
            cli::update::execute_close(args, &mut storage, &actor, cli.json, output)
        }
        cli::Commands::Comment(args) => {
            cli::update::execute_comment(args, &mut storage, &actor, cli.json, output)
        }
        cli::Commands::Ready(args) => {
            cli::ready::execute_ready(args, storage.as_ref(), cli.json, output)
        }
        cli::Commands::Blocked(args) => {
            cli::ready::execute_blocked(args, storage.as_ref(), cli.json, output)
        }
        cli::Commands::Dep(dep_cmd) => match dep_cmd {
            cli::dep::DepCommands::Add(args) => {
                cli::dep::execute_add(args, &mut storage, &actor, cli.json, output)
            }
            cli::dep::DepCommands::Remove(args) => {
                cli::dep::execute_remove(args, &mut storage, &actor, cli.json, output)
            }
            cli::dep::DepCommands::Tree(args) => {
                cli::dep::execute_tree(args, storage.as_ref(), cli.json, output)
            }
            cli::dep::DepCommands::Cycles => {
                cli::dep::execute_cycles(storage.as_ref(), cli.json, output)
            }
        },
        cli::Commands::Export(args) => cli::export::execute_export(args, storage.as_ref(), output),
        cli::Commands::Import(args) => {
            cli::export::execute_import(args, &mut storage, cli.json, output)
        }
        cli::Commands::Stats(args) => cli::stats::execute(args, storage.as_ref(), cli.json, output),
    }
    .context(Failure::Command)?;

    if !no_publish {
        sync.publish(storage.as_mut(), explicit_sync)
            .context(Failure::Publish)?;
    }
    Ok(())
}
