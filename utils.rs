use std::path::Path;
use anyhow::Result;

/// Synchronise the directory to ensure file creations/renames are durable.
pub fn sync_dir(dir: &Path) -> Result<()> {
    let f = std::fs::File::open(dir)?;
    f.sync_all()?;
    Ok(())
}

