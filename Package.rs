use crate::config::Config;
use crate::hash::{hash_file, ChainRecord, HASH_LEN};
use crate::utils::sync_dir;
use anyhow::{anyhow, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use walkdir::WalkDir;
use zip::write::{FileOptions, ZipWriter};

pub fn generate_package(
    config: &Config,
    instance_id: &str,
    record: &ChainRecord,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let cache = cache_dir.ok_or_else(|| anyhow!("cache_dir required for package generation"))?;

    // Verify XML hash
    let xml_path = cache.join("submission.xml");
    if !xml_path.exists() {
        return Err(anyhow!("submission.xml missing in cache"));
    }
    let new_xml_hash = hash_file(&xml_path)?;
    if new_xml_hash.len() != HASH_LEN {
        return Err(anyhow!("xml_hash has wrong length"));
    }
    if new_xml_hash != record.xml_hash {
        return Err(anyhow!("xml_hash mismatch"));
    }

    // Collect media files (excluding submission.xml and metadata.json)
    let mut media_files = Vec::new();
    for entry in WalkDir::new(cache)
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
        let rel = p.strip_prefix(cache).unwrap_or(p);
        rel.to_str().unwrap_or("").to_string()
    });

    // Verify media hashes
    let mut new_media_hashes = Vec::new();
    for path in &media_files {
        let h = hash_file(path)?;
        if h.len() != HASH_LEN {
            return Err(anyhow!("media hash has wrong length"));
        }
        new_media_hashes.push(h);
    }
    if new_media_hashes != record.media_hashes {
        return Err(anyhow!("media_hashes mismatch"));
    }

    let packages_dir = &config.packages_dir;
    std::fs::create_dir_all(packages_dir)?;

    let zip_path = packages_dir.join(format!("{}.zip", instance_id));
    let json_path = packages_dir.join(format!("{}.json", instance_id));
    let temp_zip = packages_dir.join(format!("{}.tmp.zip", instance_id));
    let temp_json = packages_dir.join(format!("{}.tmp.json", instance_id));

    // Create ZIP
    {
        let zip_file = File::create(&temp_zip)?;
        let mut zip = ZipWriter::new(zip_file);
        let compression = config.compression_method();
        let options = FileOptions::default().compression_method(compression);

        zip.start_file("submission.xml", options)?;
        let mut f = File::open(xml_path)?;
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer)?;
        zip.write_all(&buffer)?;

        for path in media_files {
            let rel = path.strip_prefix(cache).unwrap_or(&path);
            let name = rel.to_str().unwrap_or("unknown");
            if name == "submission.xml" || name == "metadata.json" {
                continue;
            }
            zip.start_file(name, options)?;
            let mut f = File::open(path)?;
            let mut buffer = Vec::new();
            f.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
        }

        zip.finish()?;
        // Sync the temp zip file
        let f = File::open(&temp_zip)?;
        f.sync_all()?;
    }

    // Write JSON metadata
    {
        let json_data = serde_json::json!({
            "instance_id": instance_id,
            "day": record.day,
            "tick": record.tick,
            "h0": hex::encode(&record.h0),
            "h1": hex::encode(&record.h1),
            "wavehash": hex::encode(&record.wavehash),
            "parent_wavehash": hex::encode(&record.parent_wavehash),
            "media_hashes": record.media_hashes.iter().map(|h| hex::encode(h)).collect::<Vec<_>>(),
            "xml_hash": hex::encode(&record.xml_hash),
            "timestamp": record.timestamp,
            "conflict_detected": record.conflict_detected,
        });
        let mut f = File::create(&temp_json)?;
        f.write_all(serde_json::to_string_pretty(&json_data)?.as_bytes())?;
        f.sync_all()?;
    }

    // Atomically rename
    std::fs::rename(&temp_zip, &zip_path)?;
    std::fs::rename(&temp_json, &json_path)?;
    sync_dir(packages_dir)?;

    Ok(())
}

