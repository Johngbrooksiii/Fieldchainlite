use crate::hash::ChainRecord;
use anyhow::Result;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

pub struct Chain {
    pool: SqlitePool,
    log_path: PathBuf,
}

impl Chain {
    pub async fn new(home: &PathBuf) -> Result<Self> {
        let db_path = home.join("chain.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!("sqlite:{}?mode=rwc", db_path.display()))
            .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                instance_id TEXT NOT NULL,
                day INTEGER NOT NULL,
                tick INTEGER NOT NULL,
                h0 BLOB NOT NULL,
                h1 BLOB NOT NULL,
                wavehash BLOB NOT NULL UNIQUE,
                parent_wavehash BLOB NOT NULL,
                media_hashes TEXT NOT NULL,
                xml_hash BLOB NOT NULL,
                timestamp INTEGER NOT NULL,
                conflict_detected INTEGER NOT NULL DEFAULT 0,
                previous_hash BLOB
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS schema_version (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                version INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query("INSERT OR IGNORE INTO schema_version (id, version) VALUES (1, 2)")
            .execute(&pool)
            .await?;

        let log_path = home.join("chain.log");
        Ok(Chain { pool, log_path })
    }

    /// Appends a record to the chain, writing to the log first for durability.
    pub async fn append(&self, record: &ChainRecord) -> Result<()> {
        // Write log line and fsync first
        let line = serde_json::to_string(record)? + "\n";
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;
        f.write_all(line.as_bytes())?;
        f.sync_all()?;

        // Then insert into SQLite
        let media_hashes_json = serde_json::to_string(&record.media_hashes)?;
        let conflict_flag = if record.conflict_detected { 1 } else { 0 };

        sqlx::query(
            "INSERT INTO records (
                instance_id, day, tick, h0, h1, wavehash, parent_wavehash,
                media_hashes, xml_hash, timestamp, conflict_detected, previous_hash
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&record.instance_id)
        .bind(record.day)
        .bind(record.tick)
        .bind(&record.h0)
        .bind(&record.h1)
        .bind(&record.wavehash)
        .bind(&record.parent_wavehash)
        .bind(&media_hashes_json)
        .bind(&record.xml_hash)
        .bind(record.timestamp)
        .bind(conflict_flag)
        .bind(&record.previous_hash)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_head_wavehash(&self) -> Result<Option<Vec<u8>>> {
        let row: Option<(Vec<u8>,)> =
            sqlx::query_as("SELECT wavehash FROM records ORDER BY id DESC LIMIT 1")
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|r| r.0))
    }

    pub async fn exists_instance_id(&self, instance_id: &str) -> Result<bool> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM records WHERE instance_id = ? LIMIT 1")
                .bind(instance_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    pub async fn get_record_by_instance_id(
        &self,
        instance_id: &str,
    ) -> Result<Option<ChainRecord>> {
        let row: Option<(
            String,
            i64,
            i64,
            Vec<u8>,
            Vec<u8>,
            Vec<u8>,
            Vec<u8>,
            String,
            Vec<u8>,
            i64,
            i32,
            Option<Vec<u8>>,
        )> = sqlx::query_as(
            "SELECT instance_id, day, tick, h0, h1, wavehash, parent_wavehash,
                    media_hashes, xml_hash, timestamp, conflict_detected, previous_hash
             FROM records WHERE instance_id = ? ORDER BY id DESC LIMIT 1",
        )
        .bind(instance_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let media_hashes: Vec<Vec<u8>> = serde_json::from_str(&row.7)?;
            Ok(Some(ChainRecord {
                instance_id: row.0,
                day: row.1,
                tick: row.2,
                h0: row.3,
                h1: row.4,
                wavehash: row.5,
                parent_wavehash: row.6,
                media_hashes,
                xml_hash: row.8,
                timestamp: row.9,
                conflict_detected: row.10 != 0,
                previous_hash: row.11,
            }))
        } else {
            Ok(None)
        }
    }

    /// Verifies chain continuity by checking parent_wavehash links.
    pub async fn verify(&self) -> Result<(Option<Vec<u8>>, u64, u64)> {
        use sqlx::Row;
        let rows = sqlx::query("SELECT wavehash, parent_wavehash FROM records ORDER BY id ASC")
            .fetch_all(&self.pool)
            .await?;

        let mut head = None;
        let mut count = 0u64;
        let mut errors = 0u64;
        let mut prev_wavehash: Option<Vec<u8>> = None;

        for row in rows {
            let wavehash: Vec<u8> = row.get(0);
            let parent: Vec<u8> = row.get(1);
            count += 1;
            if let Some(ref prev) = prev_wavehash {
                if parent != *prev {
                    errors += 1;
                    tracing::warn!("Chain break: parent does not match previous wavehash");
                }
            }
            prev_wavehash = Some(wavehash.clone());
            head = Some(wavehash);
        }

        Ok((head, count, errors))
    }
}

