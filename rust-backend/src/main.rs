//! Polytaoli - Prediction Market Arbitrage Scanner
//!
//! A high-performance arbitrage scanner for Kalshi and Polymarket prediction markets.

use anyhow::Result;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod clients;
mod clob;
mod config;
mod core;
mod models;
mod services;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "polytaoli=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("🚀 启动 Polytaoli - 预测市场套利扫描器");

    // Load configuration
    let config = Config::from_file("config.toml")?;
    info!("✅ 配置文件加载完成");

    // Initialize and run the application
    let app = api::create_app(config).await?;

    // Start the server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    info!("🌐 服务器监听地址: http://0.0.0.0:8000");

    axum::serve(listener, app).await?;

    Ok(())
}
