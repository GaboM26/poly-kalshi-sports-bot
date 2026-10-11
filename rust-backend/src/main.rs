//! Polytaoli - Prediction Market Arbitrage Scanner
//!
//! A high-performance arbitrage scanner for Kalshi and Polymarket prediction markets.

use anyhow::Result;
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

mod api;
mod clients;
mod config;
mod core;
mod models;
mod services;
mod utils;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    // Create the log directory.
    std::fs::create_dir_all("logs")?;

    // Initialize the debug log path (using the current working directory).
    utils::init_debug_log_path(None);

    // File log appender - rotate daily.
    let file_appender = tracing_appender::rolling::daily("logs", "polytaoli.log");
    let (non_blocking_file, _guard) = tracing_appender::non_blocking(file_appender);

    // Console log layer - show only info and higher. tower_http=info (not
    // warn) so every completed HTTP request logs a line, not just failures.
    let console_layer = fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_filter(EnvFilter::new("polytaoli=info,tower_http=info"));

    // File log layer - record info and higher.
    let file_layer = fmt::layer()
        .with_writer(non_blocking_file)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true)
        .with_filter(EnvFilter::new("polytaoli=info,tower_http=info"));

    // Combine log layers.
    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    info!("🚀 Starting Polytaoli - Prediction Market Arbitrage Scanner");
    info!("📝 Log file: logs/polytaoli.log");

    // Load configuration
    let config = Config::from_file("config.toml")?;
    info!("✅ Configuration file loaded");

    spawn_hang_watchdog();

    // Initialize and run the application
    let app = api::create_app(config).await?;

    // Start the server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    info!("🌐 Server listening at: http://0.0.0.0:8000");

    axum::serve(listener, app).await?;

    Ok(())
}

/// The backend once froze with every thread parked and no log output for
/// ~50 minutes. A std thread (independent of the async runtime) reports the
/// two ways that can happen: a `parking_lot` lock cycle (with each thread's
/// backtrace) and an async runtime that stops making progress.
fn spawn_hang_watchdog() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn now_secs() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
    }

    let heartbeat = Arc::new(AtomicU64::new(now_secs()));
    let beat = heartbeat.clone();
    tokio::spawn(async move {
        loop {
            beat.store(now_secs(), Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(10));
        for (i, threads) in parking_lot::deadlock::check_deadlock().iter().enumerate() {
            error!("DEADLOCK #{}: {} threads in a lock cycle", i, threads.len());
            for t in threads {
                error!("deadlocked thread {:?}:\n{:?}", t.thread_id(), t.backtrace());
            }
        }
        let stalled = now_secs().saturating_sub(heartbeat.load(Ordering::Relaxed));
        if stalled >= 30 {
            error!("Async runtime made no progress for {}s", stalled);
        }
    });
}
