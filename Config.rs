use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zip::CompressionMethod;
use anyhow::Result;
use rand::RngCore;
use crate::utils::sync_dir;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub drop_folder: PathBuf,
    pub home_dir: PathBuf,
    pub media_dir: PathBuf,
    pub packages_dir: PathBuf,
    pub tmp_dir: PathBuf,
    pub use_saf: bool,
    pub saf_uri: Option<String>,
    pub max_restarts: u32,
    pub backoff_base_ms: u64,
    pub compression_method: String,
    pub odk_reference_path: PathBuf,
}

impl Config {
    /// Loads configuration from `~/.fieldchain/config/paths.json`, creating defaults if missing.
    pub fn load() -> anyhow::Result<Self> {
        let home = dirs::home_dir().unwrap_or_else(|| {
            PathBuf::from("/data/data/com.termux/files/home")
        });
        let base = home.join(".fieldchain");
        let config_path = base.join("config/paths.json");

        if config_path.exists() {
            let data = std::fs::read_to_string(&config_path)?;
            let mut cfg: Config = serde_json::from_str(&data)?;
            if Self::is_test_mode() {
                cfg.drop_folder = home.join("FieldChainDrop");
                cfg.odk_reference_path = home.join("fake_odk_instances");
            }
            Ok(cfg)
        } else {
            let cfg = Config {
                drop_folder: home.join("FieldChainDrop"),
                home_dir: base.clone(),
                media_dir: base.join("media"),
                packages_dir: base.join("packages"),
                tmp_dir: base.join("tmp"),
                use_saf: false,
                saf_uri: None,
                max_restarts: 3,
                backoff_base_ms: 1000,
                compression_method: "Deflated".to_string(),
                odk_reference_path: home.join("fake_odk_instances"),
            };

            std::fs::create_dir_all(&base)?;
            std::fs::create_dir_all(&cfg.media_dir)?;
            std::fs::create_dir_all(&cfg.packages_dir)?;
            std::fs::create_dir_all(&cfg.tmp_dir)?;
            std::fs::create_dir_all(&base.join("config"))?;
            std::fs::create_dir_all(&cfg.drop_folder)?;
            std::fs::create_dir_all(&cfg.odk_reference_path)?;
            std::fs::write(&config_path, serde_json::to_string_pretty(&cfg)?)?;
            Ok(cfg)
        }
    }

    /// Returns the compression method to use for ZIP files.
    pub fn compression_method(&self) -> CompressionMethod {
        match self.compression_method.as_str() {
            "Stored" => CompressionMethod::Stored,
            _ => CompressionMethod::Deflated,
        }
    }

    /// Returns true if we are in test mode (environment variable or path-based).
    pub fn is_test_mode() -> bool {
        std::env::var("FIELDCHAIN_TEST_MODE").is_ok()
            || dirs::home_dir()
                .map(|h| h.starts_with("/home/workdir"))
                .unwrap_or(false)
    }

    /// Returns true if SAF simulation should be used (use_saf is true and no URI is provided).
    pub fn saf_simulation(&self) -> bool {
        self.use_saf && self.saf_uri.is_none()
    }

    /// Load the per‑installation salt, generating a fresh 32‑byte secure salt if missing.
    /// The salt is stored atomically in `home_dir/salt.bin`.
    pub fn get_or_create_salt(&self) -> Result<[u8; 32]> {
        let salt_path = self.home_dir.join("salt.bin");
        if salt_path.exists() {
            let data = std::fs::read(&salt_path)?;
            if data.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&data);
                return Ok(arr);
            } else {
                // Corrupted or wrong size – recreate
                tracing::warn!("Salt file corrupted (wrong size), regenerating");
            }
        }
        // Generate new salt
        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);

        // Atomic write
        let temp_path = self.home_dir.join("salt.bin.tmp");
        std::fs::write(&temp_path, &salt)?;
        let f = std::fs::File::open(&temp_path)?;
        f.sync_all()?;
        std::fs::rename(&temp_path, &salt_path)?;
        sync_dir(&self.home_dir)?;
        tracing::info!("Generated new installation salt");
        Ok(salt)
    }
}

