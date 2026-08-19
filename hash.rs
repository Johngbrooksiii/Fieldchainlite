use sha2::{Sha256, Digest};
use serde::{Serialize, Deserialize};
use std::path::Path;

pub const HASH_LEN: usize = 32;

/// H0 = SHA‑256(raw XML bytes || metadata || salt)
pub fn compute_h0(xml: &[u8], metadata: &[u8], salt: &[u8; 32]) -> anyhow::Result<Vec<u8>> {
    let mut hasher = Sha256::new();
    hasher.update(xml);
    hasher.update(metadata);
    hasher.update(salt);
    Ok(hasher.finalize().to_vec())
}

/// H1 = SHA‑256(H0 || parent_wavehash)
pub fn compute_h1(h0: &[u8], parent_wavehash: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(h0);
    hasher.update(parent_wavehash);
    hasher.finalize().to_vec()
}

/// wavehash = SHA‑256(day_le || tick_le || H1 || media_hashes...)
pub fn compute_wavehash(day: i64, tick: i64, h1: &[u8], media_hashes: &[Vec<u8>]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(&day.to_le_bytes());
    hasher.update(&tick.to_le_bytes());
    hasher.update(h1);
    for mh in media_hashes {
        hasher.update(mh);
    }
    hasher.finalize().to_vec()
}

/// SHA‑256 of a file's raw content.
pub fn hash_file(path: &Path) -> anyhow::Result<Vec<u8>> {
    let data = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hasher.finalize().to_vec())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChainRecord {
    pub instance_id: String,
    pub day: i64,
    pub tick: i64,
    pub h0: Vec<u8>,
    pub h1: Vec<u8>,
    pub wavehash: Vec<u8>,
    pub parent_wavehash: Vec<u8>,
    pub media_hashes: Vec<Vec<u8>>,
    pub xml_hash: Vec<u8>,
    pub timestamp: i64,
    pub conflict_detected: bool,
    pub previous_hash: Option<Vec<u8>>,
}

