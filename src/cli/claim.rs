use anyhow::Result;
use clap::Args;
use tracer::storage::Storage;

#[derive(Args)]
pub struct ClaimArgs {
    /// Ready issue to claim for the current actor
    pub id: String,
}

#[derive(Args)]
pub struct ReleaseArgs {
    /// Owned issue to release
    pub id: String,

    /// Manually recover ownership from another actor (no automatic expiry)
    #[arg(long)]
    pub force: bool,
}

pub fn execute_claim(
    args: ClaimArgs,
    storage: &mut Box<dyn Storage>,
    actor: &str,
    json: bool,
) -> Result<()> {
    storage.claim_issue(&args.id, actor)?;

    if json {
        let issue = storage
            .get_issue(&args.id)?
            .expect("Claimed issue should exist");
        println!("{}", serde_json::to_string_pretty(&issue)?);
    } else {
        use colored::Colorize;
        println!("✓ Claimed issue {} for {}", args.id.bold().cyan(), actor);
    }
    Ok(())
}

pub fn execute_release(
    args: ReleaseArgs,
    storage: &mut Box<dyn Storage>,
    actor: &str,
    json: bool,
) -> Result<()> {
    storage.release_issue(&args.id, actor, args.force)?;

    if json {
        let issue = storage
            .get_issue(&args.id)?
            .expect("Released issue should exist");
        println!("{}", serde_json::to_string_pretty(&issue)?);
    } else {
        use colored::Colorize;
        println!("✓ Released issue {}", args.id.bold().cyan());
    }
    Ok(())
}
