use crate::config::Config;
use crate::hash::*;
use crate::chain::Chain;
use crate::timing::{Counter, FIBONACCI};
use crate::utils::sync_dir;
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use walkdir::WalkDir;
use anyhow::Result;
use tracing::{info, error, warn};
use std::time::{Duration, SystemTime};
use async_trait::async_trait;

/// Validates that the XML is well-formed and contains an instance_id/instanceID element.
/// Returns the extracted instance_id if found and non-empty, else an error.
fn validate_and_extract_instance_id(xml: &[u8]) -> Result<String, String> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    if xml.is_empty() {
        return Err("empty file".into());
    }

    let mut reader = Reader::from_reader(xml);
    reader.trim_text(true);
    let mut buf = Vec::new();
    let mut inside_instance_id = false;
    let mut instance_id = None;
    let mut depth = 0i32;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                let name = e.name().local_name();
                let lower = name.as_ref().to_ascii_lowercase();
                if lower == b"instanceid" || lower == b"instance_id" {
                    inside_instance_id = true;
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.name().local_name();
                let lower = name.as_ref().to_ascii_lowercase();
                if lower == b"instanceid" || lower == b"instance_id" {
                    inside_instance_id = true;
                }
            }
            Ok(Event::Text(e)) => {
                if inside_instance_id && instance_id.is_none() {
                    if let Ok(text) = e.unescape() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            instance_id = Some(trimmed.to_string());
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                depth -= 1;
                if depth < 0 {
                    return Err("mismatched tags".into());
                }
                let name = e.name().local_name();
                let lower = name.as_ref().to_ascii_lowercase();
                if lower == b"instanceid" || lower == b"instance_id" {
                    inside_instance_id = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    if depth != 0 {
        return Err("unclosed tags".into());
    }
    instance_id.ok_or_else(|| "missing or empty <instance_id> or <instanceID> element".into())
}

/// Three‑snapshot stability check.
async fn is_stable(dir: &Path) -> Result<bool> {
    fn snapshot(dir: &Path) -> Result<Vec<(PathBuf, SystemTime, u64)>> {
        let mut entries = Vec::new();
        for entry in WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path().to_path_buf();
            let metadata = std::fs::metadata(&path)?;
            let mtime = metadata.modified()?;
            let len = metadata.len();
            entries.push((path, mtime, len));
        }
        // Sort for deterministic comparison
        entries.sort_by_key(|(p, _, _)| p.clone());
        Ok(entries)
    }

    fn snapshots_equal(s1: &[(PathBuf, SystemTime, u64)], s2: &[(PathBuf, SystemTime, u64)]) -> bool {
        if s1.len() != s2.len() {
            return false;
        }
        s1.iter().zip(s2.iter()).all(|(a, b)| a == b)
    }

    let snap1 = snapshot(dir)?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snap2 = snapshot(dir)?;
    if !snapshots_equal(&snap1, &snap2) {
        return Ok(false);
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let snap3 = snapshot(dir)?;
    if !snapshots_equal(&snap2, &snap3) {
        return Ok(false);
    }
    Ok(true)
}

pub struct InstanceWatcher {
    config: Config,
    chain: Arc<Chain>,
    counter: Arc<Mutex<Counter>>,
    shutdown: Arc<AtomicBool>,
    salt: [u8; 32],
}

impl InstanceWatcher {
    pub fn new(config: Config, chain: Arc<Chain>, counter: Arc<Mutex<Counter>>, shutdown: Arc<AtomicBool>) -> Result<Self> {
        let salt = config.get_or_create_salt()?;
        Ok(InstanceWatcher {
            config,
            chain,
            counter,
            shutdown,
            salt,
        })
    }

    /// Copies an instance directory into the cache using a temporary location for atomicity.
    fn cache_instance(&self, instance_id: &str, instance_dir: &Path) -> Result<PathBuf> {
        let cache_root = self.config.media_dir.join("instance_cache");
        std::fs::create_dir_all(&cache_root)?;

        let cache_dir = cache_root.join(instance_id);
        let tmp_dir = self.config.tmp_dir.join(instance_id);

        // Clear any stale temporary directory from a previous crash
        if tmp_dir.exists() {
            std::fs::remove_dir_all(&tmp_dir)?;
        }
        std::fs::create_dir_all(&tmp_dir)?;

        for entry in WalkDir::new(instance_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let src = entry.path();
            let rel = src.strip_prefix(instance_dir).unwrap();
            let dst = tmp_dir.join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(src, &dst)?;
        }

        if cache_dir.exists() {
            std::fs::remove_dir_all(&cache_dir)?;
        }
        std::fs::rename(&tmp_dir, &cache_dir)?;
        sync_dir(&cache_root)?;

        Ok(cache_dir)
    }

    fn mark_processed(&self, instance_id: &str) -> Result<()> {
        let cache_root = self.config.media_dir.join("instance_cache");
        let cache_dir = cache_root.join(instance_id);
        let sentinel = cache_dir.join(".processed");
        let temp_sentinel = cache_dir.join(".processed.tmp");

        let timestamp = chrono::Utc::now().to_rfc3339();
        std::fs::write(&temp_sentinel, timestamp)?;

        let f = std::fs::File::open(&temp_sentinel)?;
        f.sync_all()?;

        std::fs::rename(&temp_sentinel, &sentinel)?;
        sync_dir(&cache_dir)?;

        Ok(())
    }

    async fn process_instance(&self, instance_dir: &Path) -> Result<()> {
        let dir_name = instance_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        info!("📁 Processing instance: {:?}", instance_dir);

        if !is_stable(instance_dir).await? {
            warn!("Instance {} not stable yet, skipping for now.", dir_name);
            return Ok(());
        }

        let xml_path = instance_dir.join("submission.xml");
        if !xml_path.exists() {
            warn!("No submission.xml in {:?}", instance_dir);
            return Ok(());
        }

        let xml_bytes = std::fs::read(&xml_path)?;
        let instance_id = match validate_and_extract_instance_id(&xml_bytes) {
            Ok(id) => id,
            Err(e) => {
                warn!("Rejecting instance {:?}: invalid XML ({})", instance_dir, e);
                return Ok(());
            }
        };

        info!("Using instance_id: {}", instance_id);

        let xml_hash = hash_file(&xml_path)?;
        if xml_hash.len() != HASH_LEN {
            return Err(anyhow::anyhow!("xml_hash has wrong length"));
        }

        // Collect media files deterministically.
        let mut media_files = Vec::new();
        for entry in WalkDir::new(instance_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path();
            let name = path.file_name().unwrap().to_str().unwrap();
            if name == "submission.xml" || name == "metadata.json" {
                continue;
            }
            media_files.push(path.to_path_buf());
        }
        media_files.sort_by_key(|p| {
            let rel = p.strip_prefix(instance_dir).unwrap();
            rel.to_str().unwrap_or("").to_string()
        });

        let mut media_hashes = Vec::new();
        for path in &media_files {
            let h = hash_file(path)?;
            if h.len() != HASH_LEN {
                return Err(anyhow::anyhow!("media hash has wrong length"));
            }
            media_hashes.push(h);
        }

        let existing = self.chain.get_record_by_instance_id(&instance_id).await?;
        let mut conflict = false;
        let mut previous_hash = None;

        if let Some(existing_rec) = existing {
            let same = existing_rec.xml_hash == xml_hash
                && existing_rec.media_hashes == media_hashes;
            if !same {
                conflict = true;
                previous_hash = Some(existing_rec.wavehash);
                warn!("Conflict detected for instance_id {}: content differs", instance_id);
            } else {
                info!("Instance {} already processed with identical content, skipping", instance_id);
                return Ok(());
            }
        }

        let parent = self.chain.get_head_wavehash().await?.unwrap_or_default();
        let metadata = format!("{}:{}", instance_id, chrono::Utc::now().timestamp());
        let h0 = compute_h0(&xml_bytes, metadata.as_bytes(), &self.salt)?;
        if h0.len() != HASH_LEN {
            return Err(anyhow::anyhow!("h0 has wrong length"));
        }
        let h1 = compute_h1(&h0, &parent);
        if h1.len() != HASH_LEN {
            return Err(anyhow::anyhow!("h1 has wrong length"));
        }

        let mut counter = self.counter.lock().await;
        let (day, tick) = counter.next(&self.config.home_dir)?;
        drop(counter);

        let wavehash = compute_wavehash(day, tick, &h1, &media_hashes);
        if wavehash.len() != HASH_LEN {
            return Err(anyhow::anyhow!("wavehash has wrong length"));
        }

        let record = ChainRecord {
            instance_id: instance_id.clone(),
            day,
            tick,
            h0,
            h1,
            wavehash: wavehash.clone(),
            parent_wavehash: parent.clone(),
            media_hashes: media_hashes.clone(),
            xml_hash,
            timestamp: chrono::Utc::now().timestamp_millis(),
            conflict_detected: conflict,
            previous_hash,
        };

        self.chain.append(&record).await?;

        let cache_dir = self.cache_instance(&instance_id, instance_dir)?;
        crate::package::generate_package(&self.config, &instance_id, &record, Some(&cache_dir))?;
        self.mark_processed(&instance_id)?;

        info!("✅ Successfully processed instance {}", instance_id);
        Ok(())
    }

    async fn scan_directory(&self, dir: &Path) -> Result<()> {
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() && path.join("submission.xml").exists() {
                if let Err(e) = self.process_instance(&path).await {
                    error!("Error processing {}: {}", path.display(), e);
                    // Continue with other instances
                }
            }
        }
        Ok(())
    }

    async fn startup_scan(&self) -> Result<()> {
        info!("🔍 Starting startup scan");
        self.scan_directory(&self.config.drop_folder).await?;

        if Config::is_test_mode() || self.config.saf_simulation() {
            info!("Scanning simulated ODK instances root: {:?}", self.config.odk_reference_path);
            self.scan_directory(&self.config.odk_reference_path).await?;
        }

        if self.config.use_saf {
            info!("SAF enabled, but only simulation is supported; using ODK reference path for scanning.");
        }

        Ok(())
    }

    pub async fn watch(&self) -> Result<()> {
        self.startup_scan().await?;

        let drop = &self.config.drop_folder;
        if !drop.exists() {
            std::fs::create_dir_all(drop)?;
        }

        let (tx, mut rx) = tokio::sync::mpsc::channel(100);
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    if event.kind.is_create() || event.kind.is_modify() {
                        for path in event.paths {
                            let _ = tx.try_send(path);
                        }
                    }
                }
            },
            NotifyConfig::default(),
        )?;

        watcher.watch(drop, RecursiveMode::NonRecursive)?;
        info!("👁️ Watching drop folder: {:?}", drop);

        let should_watch_odk = Config::is_test_mode() || self.config.saf_simulation();
        if should_watch_odk {
            let odk_path = &self.config.odk_reference_path;
            if !odk_path.exists() {
                std::fs::create_dir_all(odk_path)?;
            }
            watcher.watch(odk_path, RecursiveMode::NonRecursive)?;
            info!("👁️ Watching simulated ODK root: {:?}", odk_path);
        }

        let mut fib_index = 0;
        let shutdown_flag = self.shutdown.clone();

        loop {
            tokio::select! {
                Some(path) = rx.recv() => {
                    if path.is_dir() && path.join("submission.xml").exists() {
                        let delay = tokio::time::Duration::from_millis(FIBONACCI[fib_index % FIBONACCI.len()]);
                        fib_index += 1;
                        tokio::time::sleep(delay).await;
                        if let Err(e) = self.process_instance(&path).await {
                            error!("Failed to process {}: {}", path.display(), e);
                        }
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(200)) => {
                    if shutdown_flag.load(Ordering::SeqCst) {
                        info!("Shutdown flag detected, stopping watcher loop.");
                        break;
                    }
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl super::phoenix::Subsystem for InstanceWatcher {
    fn name(&self) -> &'static str { "InstanceWatcher" }

    async fn run(&self) -> Result<()> {
        self.watch().await
    }

    async fn reset(&self) -> Result<()> {
        Ok(())
    }

    fn max_restarts(&self) -> Option<u32> { Some(5) }
}

