use crate::config::Config;
use crate::utils::sync_dir;
use anyhow::Result;
use tracing::{info, warn};
use std::path::{Path, PathBuf};

/// Ensures SAF simulation is set up if needed.
pub async fn ensure_saf_access(config: &Config) -> Result<()> {
    if !config.use_saf {
        return Ok(());
    }

    if let Some(uri) = &config.saf_uri {
        info!("Using SAF URI: {}", uri);
        return Ok(());
    }

    // Simulation: create and remove a probe file to verify write access.
    let root = &config.odk_reference_path;
    std::fs::create_dir_all(root)?;
    let probe = root.join(".saf_probe");
    std::fs::write(&probe, "ok")?;
    std::fs::remove_file(&probe)?;
    sync_dir(root)?;
    info!("SAF simulation active: using ODK reference path: {:?}", root);

    Ok(())
}

/// Returns all subdirectories under the ODK root that contain `submission.xml`.
pub fn list_odk_instances(config: &Config) -> Result<Vec<PathBuf>> {
    let root = &config.odk_reference_path;
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut instances = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("submission.xml").exists() {
            instances.push(path);
        }
    }
    Ok(instances)
}

pub fn is_under_odk_root(config: &Config, path: &Path) -> bool {
    path.starts_with(&config.odk_reference_path)
}

pub fn odk_root(config: &Config) -> &Path {
    &config.odk_reference_path
}

