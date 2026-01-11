//! Telegram notification service
//!
//! Sends notifications about auto-trade executions to Telegram.

use anyhow::Result;
use reqwest::Client;
use serde_json::json;
use tracing::{error, info};

use crate::config::TelegramConfig;

/// Telegram notification client
#[derive(Clone)]
pub struct TelegramClient {
    config: TelegramConfig,
    http: Client,
}

impl TelegramClient {
    /// Create a new Telegram client
    pub fn new(config: TelegramConfig) -> Self {
        Self {
            config,
            http: Client::new(),
        }
    }

    /// Check if Telegram notifications are enabled and configured
    pub fn is_enabled(&self) -> bool {
        self.config.enabled 
            && !self.config.bot_token.is_empty() 
            && !self.config.chat_id.is_empty()
    }

    /// Send auto-trade notification
    pub async fn send_auto_trade_notification(
        &self,
        event_name: &str,
        team_name: &str,
        profit_margin: f64,
        kalshi_success: bool,
        poly_success: bool,
        kalshi_error: Option<&str>,
        poly_error: Option<&str>,
        total_amount: f64,
        expected_profit: f64,
    ) {
        if !self.is_enabled() {
            return;
        }

        let message = self.format_auto_trade_message(
            event_name,
            team_name,
            profit_margin,
            kalshi_success,
            poly_success,
            kalshi_error,
            poly_error,
            total_amount,
            expected_profit,
        );

        if let Err(e) = self.send_message(&message).await {
            error!("Telegram 通知发送失败: {}", e);
        }
    }

    /// Format auto-trade message
    fn format_auto_trade_message(
        &self,
        event_name: &str,
        team_name: &str,
        profit_margin: f64,
        kalshi_success: bool,
        poly_success: bool,
        kalshi_error: Option<&str>,
        poly_error: Option<&str>,
        total_amount: f64,
        expected_profit: f64,
    ) -> String {
        let status_icon = if kalshi_success && poly_success {
            "✅"
        } else if kalshi_success || poly_success {
            "⚠️"
        } else {
            "❌"
        };

        let mut message = format!(
            "{} 自动下单通知\n\n",
            status_icon
        );
        
        message.push_str(&format!("事件: {}\n", event_name));
        message.push_str(&format!("队伍: {}\n", team_name));
        message.push_str(&format!("利润率: {:.2}%\n", profit_margin));
        message.push_str(&format!("投入金额: ${:.2}\n", total_amount));
        message.push_str(&format!("预期利润: ${:.2}\n\n", expected_profit));
        
        let kalshi_status = if kalshi_success {
            "✅ 成功".to_string()
        } else {
            format!("❌ 失败 - {}", kalshi_error.unwrap_or("未知错误"))
        };
        message.push_str(&format!("Kalshi: {}\n", kalshi_status));
        
        let poly_status = if poly_success {
            "✅ 成功".to_string()
        } else {
            format!("❌ 失败 - {}", poly_error.unwrap_or("未知错误"))
        };
        message.push_str(&format!("Polymarket: {}\n", poly_status));

        message
    }

    /// Send message to Telegram
    async fn send_message(&self, text: &str) -> Result<()> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.config.bot_token
        );

        let payload = json!({
            "chat_id": self.config.chat_id,
            "text": text,
            "parse_mode": "HTML",
        });

        let response = self.http
            .post(&url)
            .json(&payload)
            .send()
            .await?;

        if response.status().is_success() {
            info!("Telegram 通知发送成功");
            Ok(())
        } else {
            let error_text = response.text().await?;
            anyhow::bail!("Telegram API 错误: {}", error_text)
        }
    }
}
