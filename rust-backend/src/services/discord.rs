//! Discord webhook notifications.
//!
//! Posts trade results and newly tracked opportunities to a Discord channel
//! through an incoming webhook. A webhook is write-only: unlike a bot token it
//! cannot read the channel or act as a user.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use parking_lot::Mutex;
use reqwest::Client;
use serde_json::json;
use tracing::{error, info};

use crate::config::DiscordConfig;
use crate::models::ArbitrageOpportunity;
use crate::services::AutoTradeExecutionRecord;

/// One alert per market per window, so a market hovering around the
/// threshold cannot flood the channel or trip Discord's webhook rate limit.
const OPPORTUNITY_COOLDOWN: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct DiscordClient {
    config: DiscordConfig,
    http: Client,
    last_opportunity_alert: Arc<Mutex<HashMap<String, Instant>>>,
}

fn is_discord_webhook(url: &str) -> bool {
    ["https://discord.com/api/webhooks/", "https://discordapp.com/api/webhooks/"]
        .iter()
        .any(|prefix| url.starts_with(prefix))
}

impl DiscordClient {
    pub fn new(config: DiscordConfig) -> Self {
        Self {
            config,
            http: Client::new(),
            last_opportunity_alert: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Enabled, and the URL really is a Discord webhook (so a typo or a
    /// pasted foreign URL can never receive trade data).
    pub fn is_enabled(&self) -> bool {
        self.config.enabled && is_discord_webhook(&self.config.webhook_url)
    }

    /// Alert on a newly tracked opportunity at or above the configured margin.
    pub fn notify_opportunity(&self, opp: &ArbitrageOpportunity) {
        if !self.is_enabled() || opp.profit_margin < self.config.opportunity_min_margin {
            return;
        }
        {
            let mut last = self.last_opportunity_alert.lock();
            let key = opp.market_key();
            if last.get(&key).is_some_and(|t| t.elapsed() < OPPORTUNITY_COOLDOWN) {
                return;
            }
            last.insert(key, Instant::now());
        }
        self.spawn_send(format_opportunity(opp));
    }

    /// Report the outcome of a paired execution. `halted` is true when
    /// auto-trading was disabled because of residual exposure.
    pub fn notify_trade(&self, record: &AutoTradeExecutionRecord, halted: bool) {
        if self.is_enabled() {
            self.spawn_send(format_trade(record, halted));
        }
    }

    fn spawn_send(&self, text: String) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.send(&text).await {
                error!("Discord notification failed: {:#}", e);
            }
        });
    }

    async fn send(&self, text: &str) -> Result<()> {
        // `allowed_mentions.parse = []` keeps market names from pinging anyone.
        let payload = json!({
            "content": text,
            "allowed_mentions": { "parse": [] },
        });
        let response = self
            .http
            .post(&self.config.webhook_url)
            .json(&payload)
            .send()
            .await?;
        if response.status().is_success() {
            info!("Discord notification sent");
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!("Discord webhook returned {status}: {body}")
        }
    }
}

fn format_opportunity(opp: &ArbitrageOpportunity) -> String {
    format!(
        "📈 **Opportunity** {:.2}%\n{} — {}\nKalshi {} @ {:.2} | Polymarket {} @ {:.2}\nDepth (contracts at quoted price or better): Kalshi {} | Polymarket {:.0}",
        opp.profit_margin,
        opp.event_name,
        opp.team_name,
        opp.kalshi_side,
        opp.kalshi_price,
        opp.polymarket_side,
        opp.polymarket_price,
        opp.kalshi_ask_depth,
        opp.poly_ask_size,
    )
}

fn format_trade(r: &AutoTradeExecutionRecord, halted: bool) -> String {
    let icon = match r.status.as_str() {
        "executed" => "✅",
        "neutralized" => "⚠️",
        "exposed" => "🚨",
        _ => "❌",
    };
    let n = f64::from(r.contracts);
    let cost = n * (r.kalshi_price + r.polymarket_price) + r.kalshi_fee + r.polymarket_fee;
    let mut text = format!(
        "{icon} **Trade {}**\n{} — {}\nMargin {:.2}% | {} contracts | invested ${:.2} | locked-in ${:.2}\nKalshi {} filled {}/{} | Polymarket {} filled {}/{}",
        r.status,
        r.event_name,
        r.team_name,
        r.profit_margin,
        r.contracts,
        cost,
        n - cost,
        r.kalshi_side,
        r.kalshi_filled_contracts,
        r.contracts,
        r.polymarket_side,
        r.polymarket_filled_contracts,
        r.contracts,
    );
    for (leg, err) in [("Kalshi", &r.kalshi_error), ("Polymarket", &r.polymarket_error)] {
        if let Some(err) = err {
            text.push_str(&format!("\n{leg} error: {err}"));
        }
    }
    if r.residual_contracts > 0 {
        text.push_str(&format!(
            "\nResidual: {} contracts on {}",
            r.residual_contracts,
            r.residual_leg.as_deref().unwrap_or("unknown leg")
        ));
    }
    if halted {
        text.push_str("\n🛑 Auto-trading disabled.");
    }
    text.chars().take(1900).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(enabled: bool, url: &str) -> DiscordClient {
        DiscordClient::new(DiscordConfig {
            enabled,
            webhook_url: url.into(),
            opportunity_min_margin: 1.0,
        })
    }

    #[test]
    fn only_a_discord_webhook_url_enables_the_client() {
        let good = "https://discord.com/api/webhooks/1/abc";
        assert!(client(true, good).is_enabled());
        assert!(!client(false, good).is_enabled());
        assert!(!client(true, "").is_enabled());
        assert!(!client(true, "https://evil.example/api/webhooks/1/abc").is_enabled());
        assert!(!client(true, "http://discord.com/api/webhooks/1/abc").is_enabled());
    }
}
