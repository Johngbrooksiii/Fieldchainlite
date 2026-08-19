use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::Result;
use tracing::{info, warn, error};
use crate::config::Config;
use std::time::Duration;
use async_trait::async_trait;

#[async_trait]
pub trait Subsystem: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self) -> Result<()>;
    async fn reset(&self) -> Result<()>;
    fn max_restarts(&self) -> Option<u32> { None }
    fn backoff_base_ms(&self) -> Option<u64> { None }
}

/// Simple supervisor that restarts subsystems with exponential backoff.
/// Handles graceful shutdown via Ctrl+C.
pub struct Phoenix {
    subsystems: Vec<Arc<dyn Subsystem>>,
    config: Config,
    shutdown: Arc<AtomicBool>,
}

impl Phoenix {
    pub fn new(config: Config, shutdown: Arc<AtomicBool>) -> Self {
        Phoenix {
            subsystems: Vec::new(),
            config,
            shutdown,
        }
    }

    pub fn register(&mut self, subsystem: Arc<dyn Subsystem>) {
        self.subsystems.push(subsystem);
    }

    pub async fn start(&self) {
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            shutdown.store(true, Ordering::SeqCst);
            info!("Ctrl+C received, initiating shutdown.");
        });

        for sub in &self.subsystems {
            let sub = sub.clone();
            let config = self.config.clone();
            let shutdown = self.shutdown.clone();

            tokio::spawn(async move {
                let max = sub.max_restarts().unwrap_or(config.max_restarts);
                let base = sub.backoff_base_ms().unwrap_or(config.backoff_base_ms);
                let mut attempts = 0;
                let mut delay = base;

                loop {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }

                    if let Err(e) = sub.run().await {
                        error!("Subsystem {} failed: {}", sub.name(), e);
                        attempts += 1;
                        if attempts > max {
                            error!("Subsystem {} exceeded restart limit, halting.", sub.name());
                            break;
                        }
                        info!("Restarting {} (attempt {}/{})", sub.name(), attempts, max);
                        if let Err(r) = sub.reset().await {
                            error!("Reset failed for {}: {}", sub.name(), r);
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        delay *= 2;
                    } else {
                        break;
                    }
                }
            });
        }

        // Outer supervision loop: wait for shutdown signal.
        while !self.shutdown.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
        info!("Shutdown signal received, exiting supervision loop.");
    }
}

  
