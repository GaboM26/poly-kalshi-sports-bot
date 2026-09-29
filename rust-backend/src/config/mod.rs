//! Configuration management module
//!
//! Handles loading and parsing of TOML configuration files.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Kalshi API configuration
#[derive(Debug, Clone, Deserialize)]
pub struct KalshiConfig {
    pub api_key: String,
    pub api_secret: String,
    #[serde(default = "default_kalshi_base_url")]
    pub base_url: String,
}

fn default_kalshi_base_url() -> String {
    "https://external-api.kalshi.com/trade-api/v2".to_string()
}

/// Polymarket US API configuration.
///
/// `base_url` is the public gateway (unauthenticated: events, sports, market
/// book/detail). `api_base_url` is the authenticated API (orders, account,
/// portfolio), signed with Ed25519 per request in `clients/polymarket.rs`.
/// Credentials are preferably supplied via `POLYMARKET_KEY_ID` /
/// `POLYMARKET_SECRET_KEY` env vars, falling back to `key_id`/`secret_key`
/// below.
#[derive(Debug, Clone, Deserialize)]
pub struct PolymarketConfig {
    /// Public gateway base URL
    #[serde(default = "default_poly_base_url")]
    pub base_url: String,

    /// Authenticated API base URL
    #[serde(default = "default_poly_api_base_url")]
    pub api_base_url: String,

    /// Polymarket US API key ID (UUID). Prefer the POLYMARKET_KEY_ID env var.
    #[serde(default)]
    pub key_id: String,

    /// Base64-encoded Ed25519 secret key. Prefer the POLYMARKET_SECRET_KEY env var.
    #[serde(default)]
    pub secret_key: String,
}

fn default_poly_base_url() -> String {
    "https://gateway.polymarket.us".to_string()
}

fn default_poly_api_base_url() -> String {
    "https://api.polymarket.us".to_string()
}

/// Application settings
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsConfig {
    /// Refresh interval in seconds (REST quote polling clamps to at least 3 seconds)
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: u64,
    /// Minimum profit margin percentage
    #[serde(default = "default_min_profit_margin")]
    pub min_profit_margin: f64,
    /// Default bet amount in USD
    #[serde(default = "default_bet_amount")]
    pub default_bet_amount: f64,
    /// Tracking threshold for high-profit opportunities (percentage)
    #[serde(default = "default_tracking_threshold")]
    pub tracking_threshold: f64,
}

fn default_refresh_interval() -> u64 {
    3
}

fn default_min_profit_margin() -> f64 {
    1.0
}

fn default_bet_amount() -> f64 {
    10.0 // Testing phase: reduced from 100.0 to 10.0
}

fn default_tracking_threshold() -> f64 {
    1.0 // Start tracking when profit >= 1%
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            refresh_interval: default_refresh_interval(),
            min_profit_margin: default_min_profit_margin(),
            default_bet_amount: default_bet_amount(),
            tracking_threshold: default_tracking_threshold(),
        }
    }
}

/// Authentication configuration
#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_username")]
    pub username: String,
    #[serde(default = "default_password")]
    pub password: String,
    #[serde(default = "default_secret_key")]
    pub secret_key: String,
    #[serde(default = "default_token_expire_hours")]
    pub token_expire_hours: u64,
}

fn default_username() -> String {
    "admin".to_string()
}

fn default_password() -> String {
    "admin123".to_string()
}

fn default_secret_key() -> String {
    "your-secret-key-change-this-in-production".to_string()
}

fn default_token_expire_hours() -> u64 {
    24
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            username: default_username(),
            password: default_password(),
            secret_key: default_secret_key(),
            token_expire_hours: default_token_expire_hours(),
        }
    }
}

/// Telegram notification configuration
#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    /// Whether Telegram notifications are enabled
    #[serde(default = "default_telegram_enabled")]
    pub enabled: bool,
    /// Telegram bot token
    #[serde(default)]
    pub bot_token: String,
    /// Telegram chat ID (group or channel)
    #[serde(default)]
    pub chat_id: String,
}

fn default_telegram_enabled() -> bool {
    false
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: default_telegram_enabled(),
            bot_token: String::new(),
            chat_id: String::new(),
        }
    }
}

/// Main configuration struct
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub kalshi: KalshiConfig,
    pub polymarket: PolymarketConfig,
    #[serde(default)]
    pub settings: SettingsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub telegram: TelegramConfig,
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();

        // Try multiple paths
        let config_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            // Try current directory first
            let current = std::env::current_dir()?.join(path);
            if current.exists() {
                current
            } else {
                // Try parent directory (for running from rust-backend/)
                let parent = std::env::current_dir()?.parent().map(|p| p.join(path));
                if let Some(p) = parent {
                    if p.exists() {
                        p
                    } else {
                        // Fall back to python-backend directory
                        std::env::current_dir()?
                            .parent()
                            .map(|p| p.join("python-backend").join(path))
                            .unwrap_or(current)
                    }
                } else {
                    current
                }
            }
        };

        let contents = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config file: {:?}", config_path))?;

        let config: Config =
            toml::from_str(&contents).with_context(|| "Failed to parse config file")?;

        Ok(config)
    }
}
