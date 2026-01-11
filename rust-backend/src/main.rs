//! Polytaoli - Prediction Market Arbitrage Scanner
//!
//! A high-performance arbitrage scanner for Kalshi and Polymarket prediction markets.

use anyhow::Result;
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

mod api;
mod clients;
mod clob;
mod config;
mod core;
mod models;
mod services;
mod utils;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    // 创建日志目录
    std::fs::create_dir_all("logs")?;
    
    // 初始化 debug 日志路径 (使用当前工作目录)
    utils::init_debug_log_path(None);

    // 文件日志 appender - 每天轮转
    let file_appender = tracing_appender::rolling::daily("logs", "polytaoli.log");
    let (non_blocking_file, _guard) = tracing_appender::non_blocking(file_appender);

    // 控制台日志层 - 只显示 info 和更高级别
    let console_layer = fmt::layer()
        .with_target(false)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_filter(EnvFilter::new("polytaoli=info,tower_http=warn"));

    // 文件日志层 - 记录 info 及以上级别
    let file_layer = fmt::layer()
        .with_writer(non_blocking_file)
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_line_number(true)
        .with_filter(EnvFilter::new("polytaoli=info,tower_http=warn"));

    // 组合日志层
    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    info!("🚀 启动 Polytaoli - 预测市场套利扫描器");
    info!("📝 日志文件: logs/polytaoli.log");

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
