mod config;
mod timing;
mod hash;
mod chain;
mod watcher;
mod package;
mod phoenix;
mod saf;
mod utils;

use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::fmt;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "verify" {
        return run_verify().await;
    }

    fmt::init();
    info!("🚀 FieldChain v0.4.6 starting");

    let config = config::Config::load()?;
    info!("Configuration loaded");

    if config.use_saf {
        saf::ensure_saf_access(&config).await?;
    }

    let chain = Arc::new(chain::Chain::new(&config.home_dir).await?);
    let counter = Arc::new(Mutex::new(timing::Counter::load_or_init(&config.home_dir)?));

    let shutdown = Arc::new(AtomicBool::new(false));

    let mut phoenix = phoenix::Phoenix::new(config.clone(), shutdown.clone());

    let watcher = Arc::new(watcher::InstanceWatcher::new(
        config.clone(),
        chain.clone(),
        counter.clone(),
        shutdown.clone(),
    )?);

    phoenix.register(watcher);

    info!("All subsystems registered, entering supervision loop");
    phoenix.start().await;

    Ok(())
}

async fn run_verify() -> Result<()> {
    let config = config::Config::load()?;
    let chain = chain::Chain::new(&config.home_dir).await?;
    let (head, count, errors) = chain.verify().await?;

    println!("Verification results:");
    println!(" Head wavehash: {}", head.map_or("None".to_string(), |h| hex::encode(h)));
    println!(" Total records: {}", count);
    println!(" Chain breaks: {}", errors);

    if errors == 0 {
        println!("✅ Chain is continuous.");
    } else {
        println!("⚠️ Chain has breaks!");
    }

    Ok(())
}

