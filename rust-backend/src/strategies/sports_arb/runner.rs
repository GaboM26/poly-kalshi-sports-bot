//! Sports-arb background tasks: live price handling, the periodic market
//! scan, the auto-trade queue and executor, and ended-market cleanup.

use std::sync::Arc;

use chrono::Utc;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use crate::config::SportsArbConfig;
use crate::core::SubscriptionInfo;
use crate::models::PriceUpdate;
use crate::services::{DiscordClient, PerformanceMetrics};
use crate::strategies::sports_arb::execution::{
    calculate_contracts_to_trade, executable_depth, executable_polymarket_book,
    find_profitable_contract_size, is_game_suspended, kalshi_buy_levels, polymarket_buy_levels, submit_paired_order,
};
use crate::strategies::sports_arb::{ArbitrageService, PairedOrderParams};

/// How often the executor re-checks an empty or busy queue. Internal loop
/// mechanics, not a trading knob.
const EXECUTOR_IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Time from `now` to the next scan: a `market_scan_interval_secs`
/// wall-clock boundary plus one of `market_scan_offsets_secs`.
fn until_next_scan(now: chrono::DateTime<chrono::Utc>, settings: &SportsArbConfig) -> std::time::Duration {
    let interval_ms = settings.market_scan_interval_secs * 1000;
    let now_ms = now.timestamp_millis().max(0) as u64;
    let into_interval = now_ms % interval_ms;
    let wait_ms = settings
        .market_scan_offsets_secs
        .iter()
        .map(|offset| offset * 1000)
        .map(|offset_ms| {
            if offset_ms > into_interval {
                offset_ms - into_interval
            } else {
                interval_ms + offset_ms - into_interval
            }
        })
        .min()
        .unwrap_or(interval_ms);
    std::time::Duration::from_millis(wait_ms)
}

/// Start every sports-arb background task.
pub fn spawn(
    service: Arc<RwLock<ArbitrageService>>,
    discord: Arc<DiscordClient>,
    metrics: Arc<PerformanceMetrics>,
    mut price_rx: mpsc::Receiver<PriceUpdate>,
) {
    // Spawn price update handler
    let service_for_prices = service.clone();
    tokio::spawn(async move {
        while let Some(update) = price_rx.recv().await {
            let service = service_for_prices.read().await;
            service.ws_manager.on_price_update(update);
        }
    });

    // Spawn periodic market scanner (every market_scan_interval_secs)
    let service_for_scanner = service.clone();
    tokio::spawn(async move {
        let settings = service_for_scanner.read().await.settings.clone();
        info!(
            "🔍 Market scan task started, interval {} seconds ({} minutes)",
            settings.market_scan_interval_secs,
            settings.market_scan_interval_secs / 60
        );

        let mut scan_count = 0u64;

        // Wait for initial WebSocket connections to establish
        tokio::time::sleep(tokio::time::Duration::from_secs(settings.initial_scan_delay_secs)).await;

        loop {
            // Scans run on wall-clock boundaries (plus a few seconds for the
            // exchanges to list it), so a new 15-minute crypto window is
            // picked up as it opens instead of up to 5 minutes later.
            tokio::time::sleep(until_next_scan(chrono::Utc::now(), &settings)).await;
            scan_count += 1;

            info!("🔄 Starting periodic market scan #{}...", scan_count);

            // Scan for new markets
            // Fetch without the service lock: the network calls can take tens
            // of seconds, and holding the lock would stall every API handler.
            let (kalshi_client, polymarket_client) = {
                let service = service_for_scanner.read().await;
                (
                    service.kalshi_client.clone(),
                    service.polymarket_client.clone(),
                )
            };
            let scan_result = match ArbitrageService::fetch_market_snapshot(
                &kalshi_client,
                &polymarket_client,
                &settings,
            )
            .await
            {
                Ok(snapshot) => {
                    let mut service = service_for_scanner.write().await;
                    service.apply_market_snapshot(snapshot)
                }
                Err(e) => {
                    error!("❌ Market scan fetch failed: {:#}", e);
                    Ok((Vec::new(), SubscriptionInfo::empty()))
                }
            };

            match scan_result {
                Ok((new_markets, sub_info)) => {
                    if new_markets.is_empty() {
                        info!(
                            "✅ Periodic scan #{} complete; no new markets found",
                            scan_count
                        );
                    } else {
                        info!(
                            "🆕 Found {} new matching markets; starting hot subscription...",
                            new_markets.len()
                        );

                        // Update WebSocketManager with new markets
                        {
                            let service = service_for_scanner.read().await;
                            let added = service.ws_manager.add_matched_markets(
                                new_markets.clone(),
                                sub_info.market_lookup.clone(),
                            );
                            info!("📊 Added {} markets to the WebSocket manager", added);
                        }

                        // Hot subscribe to new markets
                        let kalshi_success = {
                            let service = service_for_scanner.read().await;
                            if !sub_info.kalshi_tickers.is_empty() {
                                match service
                                    .kalshi_client
                                    .subscribe_markets(sub_info.kalshi_tickers.clone())
                                    .await
                                {
                                    Ok(success) => success,
                                    Err(e) => {
                                        error!("❌ Kalshi hot-subscription failed: {:#}", e);
                                        false
                                    }
                                }
                            } else {
                                true
                            }
                        };

                        if kalshi_success {
                            let service = service_for_scanner.read().await;
                            let total_markets =
                                service.ws_manager.get_matched_markets_for_frontend().len();
                            info!("✅ Hot subscription succeeded; {} matched markets currently active", total_markets);

                            // Broadcast scan stats to frontend via WebSocket
                            let scan_stats = crate::models::ScanStats {
                                scan_count,
                                new_markets_found: new_markets.len(),
                                total_matched_markets: total_markets,
                                timestamp: Utc::now(),
                            };
                            service.ws_manager.broadcast_scan_stats(scan_stats);
                            info!("📡 Broadcast scan stats to the frontend");
                        } else {
                            warn!("⚠️ Part of the hot subscription failed");
                        }
                    }
                }
                Err(e) => {
                    error!("❌ Market scan failed: {:#}", e);
                }
            }
        }
    });

    // Spawn auto-trade queue checker (every auto_trade_queue_check_ms)
    let service_for_queue = service.clone();
    let metrics_for_queue = metrics.clone();
    tokio::spawn(async move {
        let check_ms = service_for_queue.read().await.settings.auto_trade_queue_check_ms;
        info!("📋 Auto-trade queue check task started, interval {}ms", check_ms);

        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(check_ms));

        loop {
            interval.tick().await;

            // Check and add opportunities to queue
            check_and_queue_auto_trade(&service_for_queue, &metrics_for_queue).await;
        }
    });

    // Spawn auto-trade executor (pauses auto_trade_pause_ms between trades)
    let service_for_executor = service.clone();
    let discord_for_executor = discord.clone();
    let metrics_for_executor = metrics.clone();
    tokio::spawn(async move {
        let pause_ms = service_for_executor.read().await.settings.auto_trade_pause_ms;
        info!("🚀 Auto-trade executor started, opportunity interval {}ms", pause_ms);

        loop {
            let service = service_for_executor.read().await;

            // Check if already executing
            if service
                .ws_manager
                .is_auto_trading
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                drop(service);
                tokio::time::sleep(EXECUTOR_IDLE_POLL).await;
                continue;
            }

            // Get next opportunity from queue
            let key = {
                let mut queue = service.ws_manager.auto_trade_queue.write();
                queue.pop_front()
            };

            if let Some(key) = key {
                // Mark as executing
                service
                    .ws_manager
                    .is_auto_trading
                    .store(true, std::sync::atomic::Ordering::Relaxed);

                info!("🎯 [Auto-trade executor] Starting processing: {}", key);

                // Execute the opportunity
                execute_single_auto_trade(
                    &service,
                    &discord_for_executor,
                    &metrics_for_executor,
                    &key,
                )
                .await;

                // Mark as done
                service
                    .ws_manager
                    .is_auto_trading
                    .store(false, std::sync::atomic::Ordering::Relaxed);

                drop(service);

                info!("⏱️  [Auto-trade executor] Waiting {}ms before processing the next opportunity", pause_ms);
                tokio::time::sleep(tokio::time::Duration::from_millis(pause_ms)).await;
            } else {
                // Queue empty, wait a bit
                drop(service);
                tokio::time::sleep(EXECUTOR_IDLE_POLL).await;
            }
        }
    });

    // Spawn ended market cleanup task (every ended_markets.cleanup_interval_secs)
    let service_for_cleanup = service.clone();
    tokio::spawn(async move {
        let cleanup_secs = service_for_cleanup.read().await.settings.ended_markets.cleanup_interval_secs;
        info!("🧹 Ended-game cleanup task started, interval {} seconds", cleanup_secs);

        let cleanup_every = tokio::time::Duration::from_secs(cleanup_secs);
        let mut interval = tokio::time::interval(cleanup_every);

        // Wait for initial data to be populated
        tokio::time::sleep(cleanup_every).await;

        loop {
            interval.tick().await;

            // Check and clean up ended markets
            cleanup_ended_markets(&service_for_cleanup).await;
        }
    });
}

fn save_auto_trade_skip(
    service: &ArbitrageService,
    key: &str,
    record: &crate::models::ArbitrageTrackingRecord,
    opportunity: &crate::models::ArbitrageOpportunity,
    contracts: i32,
    duration_ms: i64,
    reason: &str,
) {
    info!(
        "⚠️ [Auto-trade] {}: {} - {}",
        reason, record.event_name, record.team_name
    );
    if service.ws_manager.should_record_skip(key, reason) {
        if let Err(error) = service
            .ws_manager
            .get_storage()
            .save_skipped_auto_trade_record(
                &record.event_name,
                &record.team_name,
                &record.kalshi_market_id,
                &record.polymarket_market_id,
                &opportunity.kalshi_side,
                &opportunity.polymarket_side,
                contracts,
                opportunity.kalshi_price,
                opportunity.polymarket_price,
                opportunity.profit_margin,
                duration_ms,
                reason,
            )
        {
            error!("Failed to save skipped auto-trade record: {:#}", error);
        }
    }
}

/// Check and add eligible opportunities to auto-trade queue
async fn check_and_queue_auto_trade(
    service: &RwLock<ArbitrageService>,
    _metrics: &Arc<PerformanceMetrics>,
) {
    let service = service.read().await;

    // Get auto-trade state
    let auto_state = service.ws_manager.get_auto_trade_state();

    // Early return if not enabled or already at limit
    if !auto_state.enabled {
        return;
    }

    if auto_state.trade_count >= auto_state.max_trade_count {
        return;
    }

    // Get active tracking records
    let tracking_records = service.ws_manager.get_active_tracking_for_auto_trade();

    // Add eligible opportunities to queue
    for (key, _record, duration_ms) in tracking_records {
        // Check eligibility
        let (eligible, _reason) = service
            .ws_manager
            .check_auto_trade_eligibility(&key, duration_ms);

        if !eligible {
            continue;
        }

        // Add to queue if not already queued
        let mut queue = service.ws_manager.auto_trade_queue.write();
        if !queue.contains(&key) {
            queue.push_back(key.clone());
            info!("📋 [Auto-trade queue] Added opportunity: {}", key);
        }
    }
}

/// Execute auto-trade for a single opportunity (extracted from original for loop)
async fn execute_single_auto_trade(
    service: &ArbitrageService,
    discord: &Arc<DiscordClient>,
    _metrics: &Arc<PerformanceMetrics>,
    key: &str,
) {
    let (record, duration_ms) = match service.ws_manager.get_tracking_record(key) {
        Some((record, duration_ms)) => (record, duration_ms),
        None => {
            info!("⚠️ [Auto-trade] Opportunity no longer exists: {}", key);
            return;
        }
    };

    let auto_state = service.ws_manager.get_auto_trade_state();
    let (eligible, reason) = service
        .ws_manager
        .check_auto_trade_eligibility(key, duration_ms);
    if !eligible {
        info!(
            "⚠️ [Auto-trade] Opportunity no longer meets conditions: {} - {}",
            key, reason
        );
        return;
    }

    let opportunity = match service.ws_manager.get_opportunity_by_key(key) {
        Some(opportunity) => opportunity,
        None => {
            info!("⚠️ [Auto-trade] Opportunity data does not exist: {}", key);
            return;
        }
    };

    let Some(matched_market) = service
        .get_matched_markets()
        .iter()
        .find(|market| market.market_key() == key)
    else {
        save_auto_trade_skip(
            service,
            key,
            &record,
            &opportunity,
            auto_state.min_contracts,
            duration_ms,
            "Matched market disappeared before execution",
        );
        return;
    };
    if is_game_suspended(matched_market.polymarket_market.period.as_deref()) {
        save_auto_trade_skip(
            service,
            key,
            &record,
            &opportunity,
            auto_state.min_contracts,
            duration_ms,
            "Polymarket reports the game as suspended",
        );
        return;
    }
    let polymarket_competitor = match opportunity.polymarket_side.as_str() {
        "yes" => record.team_name.as_str(),
        "no" => match matched_market
            .polymarket_market
            .get_opponent(&record.team_name)
        {
            Some(opponent) => opponent,
            None => {
                save_auto_trade_skip(
                    service,
                    key,
                    &record,
                    &opportunity,
                    auto_state.min_contracts,
                    duration_ms,
                    "Polymarket US opponent is unavailable for the NO hedge leg",
                );
                return;
            }
        },
        _ => {
            save_auto_trade_skip(
                service,
                key,
                &record,
                &opportunity,
                auto_state.min_contracts,
                duration_ms,
                "Polymarket auto-trade side must be yes or no",
            );
            return;
        }
    };
    let Some(poly_execution) = matched_market
        .polymarket_market
        .us_execution_for_competitor(polymarket_competitor)
    else {
        save_auto_trade_skip(
            service,
            key,
            &record,
            &opportunity,
            auto_state.min_contracts,
            duration_ms,
            "Polymarket US position side is unavailable for the matched competitor",
        );
        return;
    };

    let Some(kalshi_book) = service
        .kalshi_client
        .get_fresh_orderbook(&record.kalshi_market_id, service.settings.executable_book_max_age())
    else {
        save_auto_trade_skip(
            service,
            key,
            &record,
            &opportunity,
            auto_state.min_contracts,
            duration_ms,
            "Fresh Kalshi websocket executable depth is unavailable",
        );
        return;
    };
    let (poly_book, poly_book_source) =
        match executable_polymarket_book(service, &poly_execution.market_slug).await {
        Ok(book) => book,
        Err(reason) => {
            save_auto_trade_skip(
                service,
                key,
                &record,
                &opportunity,
                auto_state.min_contracts,
                duration_ms,
                &format!("Fresh Polymarket US executable book is unavailable: {reason}"),
            );
            return;
        }
    };

    let kalshi_levels = kalshi_buy_levels(&kalshi_book, &opportunity.kalshi_side);

    // The WebSocket book can silently go stale for one ticker while the
    // connection stays alive (live games move fast): it then showed
    // liquidity Kalshi no longer had and every Kalshi IOC filled nothing.
    // Veto when Kalshi's own REST book cannot cover the price we would send.
    // REST is never used for sizing, only to refuse.
    let rest_book = match service
        .kalshi_client
        .get_rest_orderbook(&record.kalshi_market_id)
        .await
    {
        Ok(book) => book,
        Err(reason) => {
            save_auto_trade_skip(
                service,
                key,
                &record,
                &opportunity,
                auto_state.min_contracts,
                duration_ms,
                &format!("Kalshi REST book cross-check unavailable: {reason:#}"),
            );
            return;
        }
    };
    let rest_levels = kalshi_buy_levels(&rest_book, &opportunity.kalshi_side);
    let rest_best = rest_levels.first().map(|(price, _)| *price);
    let ws_best = kalshi_levels.first().map(|(price, _)| *price);
    if rest_best.is_none_or(|rest| ws_best.is_none_or(|ws| rest > ws)) {
        tracing::warn!(
            "Kalshi WebSocket book disagrees with REST for {}: ws_best_ask_cents={:?} rest_best_ask_cents={:?} ws_asks={:?} rest_asks={:?}; skipping",
            record.kalshi_market_id,
            ws_best,
            rest_best,
            &kalshi_levels[..kalshi_levels.len().min(4)],
            &rest_levels[..rest_levels.len().min(4)],
        );
        save_auto_trade_skip(
            service,
            key,
            &record,
            &opportunity,
            auto_state.min_contracts,
            duration_ms,
            "Kalshi WebSocket book disagrees with Kalshi's REST book (possible stale book)",
        );
        return;
    }
    let poly_levels = polymarket_buy_levels(&poly_book, poly_execution.position_side);
    let max_depth = executable_depth(&kalshi_levels).min(executable_depth(&poly_levels));
    let Some(size_cap) = calculate_contracts_to_trade(
        max_depth,
        max_depth,
        auto_state.flexible_mode,
        auto_state.min_contracts,
        auto_state.max_contracts,
    ) else {
        save_auto_trade_skip(
            service,
            key,
            &record,
            &opportunity,
            auto_state.min_contracts,
            duration_ms,
            "Fresh executable depth is below the configured contract minimum",
        );
        return;
    };

    let Some(sized) =
        find_profitable_contract_size(
            size_cap,
            auto_state.min_contracts,
            auto_state.max_amount,
            service.ws_manager.calculator.min_profit_margin(),
            poly_book.minimum_trade_qty,
            &kalshi_levels,
            &poly_levels,
            service.settings.limit_cushion_cents,
        )
    else {
        let reason = if poly_book.minimum_trade_qty.is_none() {
            "Polymarket US market minimum trade quantity is unavailable"
        } else {
            "No fresh executable size clears the minimum profit margin after fees, worst-case prices, Polymarket's minimum trade quantity, and amount limit"
        };
        tracing::info!(
            "Auto-trade sizing skip detail: {} kalshi_side={} poly_side={} size_cap={} poly_min_qty={:?} max_amount={} min_margin={} kalshi_levels(cents,qty)={:?} poly_levels(price,qty)={:?} poly_source={:?}",
            record.event_name,
            opportunity.kalshi_side,
            opportunity.polymarket_side,
            size_cap,
            poly_book.minimum_trade_qty,
            auto_state.max_amount,
            service.ws_manager.calculator.min_profit_margin(),
            &kalshi_levels[..kalshi_levels.len().min(4)],
            &poly_levels[..poly_levels.len().min(4)],
            poly_book_source,
        );
        save_auto_trade_skip(service, key, &record, &opportunity, size_cap, duration_ms, reason);
        return;
    };

    // Detection-only sport: everything above only read fresh books, so this
    // is a dry run of the real decision. Record what would have traded and
    // never send an order.
    if !service.settings.auto_trade_enabled_for(&record.kalshi_market_id) {
        let reason = format!(
            "DRY RUN (detection-only sport): would trade {} contracts, Kalshi {} @ {}c, Polymarket {} @ {:.3}, net margin {:.2}% after fees",
            sized.contracts,
            opportunity.kalshi_side,
            sized.kalshi_price_cents,
            opportunity.polymarket_side,
            sized.poly_price,
            sized.profit_margin,
        );
        info!("🧪 [Auto-trade] {}: {}", record.event_name, reason);
        save_auto_trade_skip(service, key, &record, &opportunity, sized.contracts, duration_ms, &reason);
        return;
    }

    let outcome = submit_paired_order(
        service,
        discord,
        PairedOrderParams {
            event_name: record.event_name.clone(),
            team_name: record.team_name.clone(),
            kalshi_market_id: record.kalshi_market_id.clone(),
            kalshi_side: opportunity.kalshi_side.clone(),
            polymarket_market_slug: poly_execution.market_slug.clone(),
            polymarket_position_side: poly_execution.position_side,
            contracts: sized.contracts,
            kalshi_price_cents: sized.kalshi_price_cents,
            poly_price: sized.poly_price,
            kalshi_fee: sized.kalshi_fee,
            poly_fee: sized.poly_fee,
            profit_margin: sized.profit_margin,
            duration_ms,
            poly_book_fetched_at_ms: poly_book.fetched_at_ms,
            poly_book_source,
            neutralization_max_loss_cents: auto_state.neutralization_max_loss_cents,
            dry_run: false,
        },
    )
    .await;

    // submit_paired_order already halts auto-trading (and, for a fill
    // mismatch it can't fully neutralize, sends the Discord alert) on any
    // of these outcomes, so there is nothing further to react to here.
    match outcome.status.as_str() {
        "persistence_failed_before_submission"
        | "persistence_failed_after_poly_ack"
        | "persistence_failed_after_kalshi_ack" => {}
        _ if outcome.unsafe_fill_quantity || outcome.residual_contracts > 0 => {}
        "executed" => {
            service.ws_manager.mark_as_auto_traded(key);
            if let Err(reason) = service.ws_manager.increment_trade_count() {
                error!("Failed to increment auto-trade count: {:#}", reason);
            }
        }
        _ => {
            service.ws_manager.mark_as_auto_traded(key);
        }
    }
}

/// Clean up ended markets by unsubscribing from WebSocket feeds
///
/// This function:
/// 1. Checks all markets for extreme prices (Kalshi 99/2, Poly 100/0)
/// 2. If extreme prices persist for 20+ minutes, marks market as ended
/// 3. Unsubscribes from WebSocket feeds for ended markets
/// 4. Removes ended markets from internal caches
async fn cleanup_ended_markets(service: &RwLock<ArbitrageService>) {
    let service = service.read().await;

    // Get detection counts for logging
    let detecting_count = service.ws_manager.get_ending_detection_count();
    let ended_count = service.ws_manager.get_confirmed_ended_count();

    if detecting_count > 0 {
        info!(
            "🔍 [Cleanup] Monitoring {} potentially ended markets, confirmed {}",
            detecting_count, ended_count
        );
    }

    // Remove ended markets and get Kalshi subscriptions to unsubscribe.
    let kalshi_to_unsub = service.ws_manager.remove_ended_markets();

    if kalshi_to_unsub.is_empty() {
        return;
    }

    // Unsubscribe from Kalshi markets
    if !kalshi_to_unsub.is_empty() {
        match service
            .kalshi_client
            .unsubscribe_markets(kalshi_to_unsub.clone())
            .await
        {
            Ok(success) => {
                if success {
                    info!(
                        "✅ [Cleanup] Kalshi unsubscribe succeeded: {} markets",
                        kalshi_to_unsub.len()
                    );
                } else {
                    warn!("⚠️ [Cleanup] Kalshi unsubscribe partially failed");
                }
            }
            Err(e) => {
                error!("❌ [Cleanup] Kalshi unsubscribe failed: {:#}", e);
            }
        }
    }

    // Log summary
    let remaining_markets = service.ws_manager.get_matched_markets_for_frontend().len();
    info!(
        "✅ [Cleanup] Complete; {} active markets remain",
        remaining_markets
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rfc3339: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(rfc3339)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn until_next_scan_default(now: chrono::DateTime<chrono::Utc>) -> std::time::Duration {
        until_next_scan(now, &SportsArbConfig::default())
    }

    #[test]
    fn scans_wait_for_the_next_boundary_plus_offset() {
        assert_eq!(until_next_scan_default(at("2026-10-26T16:44:00Z")).as_secs(), 100);
        assert_eq!(until_next_scan_default(at("2026-10-26T16:45:39Z")).as_secs(), 1);
        // Exactly on a scan time: wait for the next one, never zero.
        assert_eq!(until_next_scan_default(at("2026-10-26T16:45:40Z")).as_secs(), 60);
        assert_eq!(until_next_scan_default(at("2026-10-26T16:46:30Z")).as_secs(), 10);
        assert_eq!(until_next_scan_default(at("2026-10-26T16:46:40Z")).as_secs(), 240);
    }
}
