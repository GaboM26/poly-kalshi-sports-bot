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
    "https://api.elections.kalshi.com/trade-api/v2".to_string()
}

/// Polymarket API configuration
///
/// For Magic Link users, only need to configure:
/// - private_key: Controller private key (from https://reveal.magic.link/polymarket)
/// - wallet_address: Smart Wallet address (from polymarket.com/settings)
///
/// API credentials are automatically derived.
#[derive(Debug, Clone, Deserialize)]
pub struct PolymarketConfig {
    /// Controller private key
    #[serde(default)]
    pub private_key: String,
    /// Smart Wallet address
    #[serde(default)]
    pub wallet_address: String,

    /// Gamma API base URL
    #[serde(default = "default_poly_base_url")]
    pub base_url: String,
    /// CLOB API URL
    #[serde(default = "default_clob_url")]
    pub clob_url: String,
    /// Signature type (1 = Magic Link user)
    #[serde(default = "default_signature_type")]
    pub signature_type: u8,

    /// API credentials (auto-derived, no manual configuration needed)
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_secret: String,
    #[serde(default)]
    pub api_passphrase: String,
}

fn default_poly_base_url() -> String {
    "https://gamma-api.polymarket.com".to_string()
}

fn default_clob_url() -> String {
    "https://clob.polymarket.com".to_string()
}

fn default_signature_type() -> u8 {
    1
}

/// Application settings
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsConfig {
    /// Refresh interval in seconds
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval: u64,
    /// Minimum profit margin percentage
    #[serde(default = "default_min_profit_margin")]
    pub min_profit_margin: f64,
    /// Default bet amount in USD
    #[serde(default = "default_bet_amount")]
    pub default_bet_amount: f64,
}

fn default_refresh_interval() -> u64 {
    5
}

fn default_min_profit_margin() -> f64 {
    1.0
}

fn default_bet_amount() -> f64 {
    100.0
}

impl Default for SettingsConfig {
    fn default() -> Self {
        Self {
            refresh_interval: default_refresh_interval(),
            min_profit_margin: default_min_profit_margin(),
            default_bet_amount: default_bet_amount(),
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

/// Main configuration struct
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub kalshi: KalshiConfig,
    pub polymarket: PolymarketConfig,
    #[serde(default)]
    pub settings: SettingsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
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

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if self.kalshi.api_key.is_empty() {
            anyhow::bail!("Kalshi API key is not configured");
        }
        if self.kalshi.api_secret.is_empty() {
            anyhow::bail!("Kalshi API secret is not configured");
        }
        Ok(())
    }
}
