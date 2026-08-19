use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use anyhow::Result;
use crate::utils::sync_dir;

const COUNTER_FILE: &str = "counter.bin";

/// A persistent, only-ever-incrementing counter.
#[derive(Debug, Clone, Copy)]
pub struct Counter {
    value: u64,
}

impl Counter {
    pub const TICKS_PER_DAY: u64 = 86_400_000;

    pub fn load_or_init(home: &PathBuf) -> Result<Self> {
        let path = home.join(COUNTER_FILE);
        if path.exists() {
            let mut f = File::open(&path)?;
            let mut buf = [0u8; 8];
            f.read_exact(&mut buf)?;
            let value = u64::from_le_bytes(buf);
            Ok(Counter { value })
        } else {
            Ok(Counter { value: 0 })
        }
    }

    /// Increments the counter and returns the new (day, tick) pair.
    pub fn next(&mut self, home: &PathBuf) -> Result<(i64, i64)> {
        self.value += 1;
        self.save(home)?;
        Ok(self.current())
    }

    /// Returns the current (day, tick) derived from the counter.
    pub fn current(&self) -> (i64, i64) {
        let day = (self.value / Self::TICKS_PER_DAY) as i64;
        let tick = (self.value % Self::TICKS_PER_DAY) as i64;
        (day, tick)
    }

    /// Persists the counter atomically.
    fn save(&self, home: &PathBuf) -> Result<()> {
        let path = home.join(COUNTER_FILE);
        let temp_path = home.join(format!("{}.tmp", COUNTER_FILE));

        {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temp_path)?;
            f.write_all(&self.value.to_le_bytes())?;
            f.sync_all()?;
        }

        std::fs::rename(&temp_path, &path)?;
        sync_dir(home)?;
        Ok(())
    }
}

pub const FIBONACCI: [u64; 8] = [8, 13, 21, 34, 55, 89, 144, 233];

pub fn fibonacci_delay(index: usize) -> tokio::time::Duration {
    tokio::time::Duration::from_millis(FIBONACCI[index % FIBONACCI.len()])
}

