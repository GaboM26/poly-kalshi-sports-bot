//! Shared platform: everything built once at startup and handed to every
//! strategy. Exchange clients are cheap to clone (they share their HTTP
//! client, signing key and caches), so every strategy talks to the same
//! connections and the same order book caches.

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::clients::{KalshiClient, PolymarketClient};
use crate::config::Config;
use crate::services::{ArbitrageStorage, DiscordClient, PerformanceMetrics};

pub struct Platform {
    pub config: Config,
    pub kalshi: KalshiClient,
    pub polymarket: PolymarketClient,
    pub storage: Arc<ArbitrageStorage>,
    pub metrics: Arc<PerformanceMetrics>,
    pub discord: Arc<DiscordClient>,
}

impl Platform {
    pub fn new(config: Config) -> Result<Self> {
        let storage = Arc::new(ArbitrageStorage::new(&config.storage.db_path)?);
        let metrics = Arc::new(PerformanceMetrics::new());

        // Polymarket US credentials are validated here (PolymarketClient::new
        // fails fast if they're missing) - there's no separate order service
        // to health-check anymore.
        let kalshi = KalshiClient::new(config.kalshi.clone())?;
        let polymarket = PolymarketClient::new(config.polymarket.clone())?;

        let discord = Arc::new(DiscordClient::new(config.discord.clone()));
        if discord.is_enabled() {
            info!("✅ Discord notifications enabled");
        } else {
            info!("ℹ️ Discord notifications disabled");
        }

        Ok(Self {
            config,
            kalshi,
            polymarket,
            storage,
            metrics,
            discord,
        })
    }
}
