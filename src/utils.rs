use anyhow::{Context, Result};
use std::env;
use std::path::{Path, PathBuf};

/// Find the database path using the standard search order:
/// 1. $TRACE_DB environment variable
/// 2. .trace/*.db in current directory or ancestors
/// 3. ~/.trace/default.db (fallback)
pub fn find_database_path() -> Result<PathBuf> {
    // 1. Check environment variable
    if let Ok(db_path) = env::var("TRACE_DB") {
        return Ok(PathBuf::from(db_path));
    }

    // 2. Search for .trace/*.db in current directory and ancestors
    if let Some(found_db) = find_database_in_tree()? {
        return Ok(found_db);
    }

    // 3. Fall back to home directory default
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let default_db = home.join(".trace").join("default.db");
    
    // Create parent directory if it doesn't exist
    if let Some(parent) = default_db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    
    Ok(default_db)
}

/// Find the JSONL path for a given database path
pub fn find_jsonl_path(db_path: &Path) -> PathBuf {
    let parent = db_path.parent().unwrap_or_else(|| std::path::Path::new("."));

    // Backups and filtered exports must never become the managed sync file.
    parent.join("issues.jsonl")
}

/// Walk up the directory tree looking for .trace/*.db
fn find_database_in_tree() -> Result<Option<PathBuf>> {
    let mut current = env::current_dir()?;
    
    loop {
        let trace_dir = current.join(".trace");
        if trace_dir.is_dir() {
            // Look for *.db files in .trace/
            if let Ok(entries) = std::fs::read_dir(&trace_dir) {
                for entry in entries.flatten() {
                    if let Some(ext) = entry.path().extension() {
                        if ext == "db" {
                            return Ok(Some(entry.path()));
                        }
                    }
                }
            }
        }
        
        // Move up one directory
        if !current.pop() {
            break;
        }
    }
    
    Ok(None)
}

/// Get the actor name from environment or system
pub fn get_actor() -> String {
    env::var("TRACE_ACTOR")
        .or_else(|_| env::var("USER"))
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Parse dependency specification like "blocks:bd-1" or "bd-1" (defaults to blocks)
pub fn parse_dependency_spec(spec: &str) -> Result<(crate::types::DependencyType, String)> {
    if let Some((type_str, id)) = spec.split_once(':') {
        let dep_type = type_str.parse()?;
        Ok((dep_type, id.to_string()))
    } else {
        // Default to blocks type
        Ok((crate::types::DependencyType::Blocks, spec.to_string()))
    }
}

/// Format issue for display with colors
pub fn format_issue(issue: &crate::types::Issue, with_description: bool) -> String {
    use colored::Colorize;
    
    let mut output = String::new();
    
    // Header line: ID, title, [priority, type]
    output.push_str(&format!(
        "{} {} [{}, {}]\n",
        issue.id.bold().cyan(),
        issue.title,
        format!("P{}", issue.priority).yellow(),
        issue.issue_type.to_string().green()
    ));
    
    // Status and assignee
    let status_colored = match issue.status {
        crate::types::Status::Open => "open".green(),
        crate::types::Status::InProgress => "in_progress".blue(),
        crate::types::Status::Blocked => "blocked".red(),
        crate::types::Status::Closed => "closed".dimmed(),
    };
    output.push_str(&format!("  Status: {}\n", status_colored));
    
    if !issue.assignee.is_empty() {
        output.push_str(&format!("  Assignee: {}\n", issue.assignee));
    }
    
    if let Some(est) = issue.estimated_minutes {
        output.push_str(&format!("  Estimated: {} minutes\n", est));
    }
    
    // Description
    if with_description && !issue.description.is_empty() {
        output.push_str(&format!("\n  {}\n", issue.description));
    }
    
    // Timestamps
    output.push_str(&format!(
        "  Created: {} | Updated: {}\n",
        issue.created_at.format("%Y-%m-%d %H:%M"),
        issue.updated_at.format("%Y-%m-%d %H:%M")
    ));
    
    if let Some(closed_at) = issue.closed_at {
        output.push_str(&format!("  Closed: {}\n", closed_at.format("%Y-%m-%d %H:%M")));
    }
    
    output
}

/// Compute SHA256 hash of data
pub fn compute_hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}
