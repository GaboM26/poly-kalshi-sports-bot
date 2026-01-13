//! 测试并发下单 - 验证 Polymarket 是否因余额锁定导致第二个订单失败
//!
//! 运行方式:
//! cd rust-backend
//! cargo run --bin test_concurrent_orders
//!
//! 测试目的：
//! 同时对同一个 token 下两个 $1 的市价买单，观察是否一个成功一个失败

use anyhow::Result;
use std::time::Instant;
use tracing::{info, error};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

// 导入项目模块
use polytaoli::config::Config;
use polytaoli::clients::PolymarketClient;

/// 用于测试的 token ID (可以从你的日志中获取一个活跃的 token)
/// 使用活跃市场: "Will 2025 be the hottest year on record?" - Yes 侧
const TEST_TOKEN_ID: &str = "2853768819561879023657600399360829876689515906714535926781067187993853038980";

/// 每个订单的 USDC 金额（Polymarket 最小订单金额通常是 $5）
const USDC_PER_ORDER: f64 = 5.0;

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    let console_layer = fmt::layer()
        .with_target(false)
        .with_filter(EnvFilter::new("info"));
    
    tracing_subscriber::registry()
        .with(console_layer)
        .init();

    info!("🧪 Polymarket 并发下单测试");
    info!("════════════════════════════════════════════════════════════");
    info!("测试目的: 验证同一 token 快速下两单是否会导致余额锁定问题");
    info!("Token ID: {}...", &TEST_TOKEN_ID[..20]);
    info!("每单金额: ${:.2}", USDC_PER_ORDER);
    info!("════════════════════════════════════════════════════════════");

    // 加载配置
    let config = Config::from_file("config.toml")?;
    info!("✅ 配置文件加载完成");

    // 初始化 Polymarket 客户端
    let mut poly_client = PolymarketClient::new(config.polymarket);
    poly_client.init_clob().await?;
    info!("✅ Polymarket CLOB 客户端初始化完成");

    // 获取当前余额
    match poly_client.get_balance().await {
        Ok(balance) => {
            info!("💰 当前 USDC 余额: ${:.4}", balance);
            if balance < USDC_PER_ORDER * 2.0 {
                error!("❌ 余额不足！需要至少 ${:.2}", USDC_PER_ORDER * 2.0);
                return Ok(());
            }
        }
        Err(e) => {
            error!("❌ 获取余额失败: {}", e);
            return Ok(());
        }
    }

    info!("");
    info!("🚀 开始并发下单测试...");
    info!("   同时发送两个 ${:.2} 的市价买单到同一个 token", USDC_PER_ORDER);
    info!("");

    let start = Instant::now();

    // 创建两个并发的下单 future
    let client1 = poly_client.clone();
    let client2 = poly_client.clone();

    let order1 = async {
        let order_start = Instant::now();
        let result = client1.market_buy(TEST_TOKEN_ID, USDC_PER_ORDER).await;
        let duration = order_start.elapsed().as_millis();
        (result, duration, "订单1")
    };

    let order2 = async {
        let order_start = Instant::now();
        let result = client2.market_buy(TEST_TOKEN_ID, USDC_PER_ORDER).await;
        let duration = order_start.elapsed().as_millis();
        (result, duration, "订单2")
    };

    // 并发执行两个订单
    let ((result1, duration1, name1), (result2, duration2, name2)) = 
        tokio::join!(order1, order2);

    let total_duration = start.elapsed().as_millis();

    info!("");
    info!("════════════════════════════════════════════════════════════");
    info!("📊 测试结果:");
    info!("════════════════════════════════════════════════════════════");
    
    // 打印订单1结果
    match &result1 {
        Ok(response) => {
            let status = response.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
            let success = response.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
            info!("✅ {} 成功 ({}ms)", name1, duration1);
            info!("   status={}, success={}", status, success);
            info!("   响应: {}", response);
        }
        Err(e) => {
            error!("❌ {} 失败 ({}ms)", name1, duration1);
            error!("   错误: {}", e);
        }
    }

    info!("");

    // 打印订单2结果
    match &result2 {
        Ok(response) => {
            let status = response.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
            let success = response.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
            info!("✅ {} 成功 ({}ms)", name2, duration2);
            info!("   status={}, success={}", status, success);
            info!("   响应: {}", response);
        }
        Err(e) => {
            error!("❌ {} 失败 ({}ms)", name2, duration2);
            error!("   错误: {}", e);
        }
    }

    info!("");
    info!("⏱️  总耗时: {}ms", total_duration);
    info!("");

    // 分析结果
    let order1_success = result1.is_ok();
    let order2_success = result2.is_ok();

    info!("════════════════════════════════════════════════════════════");
    info!("📝 结论:");
    info!("════════════════════════════════════════════════════════════");

    match (order1_success, order2_success) {
        (true, true) => {
            info!("✅ 两个订单都成功了！");
            info!("   结论: Polymarket 允许对同一 token 并发下单");
            info!("   之前的失败可能是其他原因（余额真的不足？价格变化？）");
        }
        (true, false) | (false, true) => {
            info!("⚠️  一个成功，一个失败！");
            info!("   结论: 验证了余额锁定假设");
            info!("   第一个订单成功后锁定了 USDC，导致第二个订单失败");
        }
        (false, false) => {
            info!("❌ 两个订单都失败了！");
            info!("   可能原因: 余额不足、token 无效、API 问题等");
        }
    }

    // 获取最新余额
    info!("");
    match poly_client.get_balance().await {
        Ok(balance) => {
            info!("💰 测试后 USDC 余额: ${:.4}", balance);
        }
        Err(e) => {
            info!("⚠️  获取测试后余额失败: {}", e);
        }
    }

    Ok(())
}
