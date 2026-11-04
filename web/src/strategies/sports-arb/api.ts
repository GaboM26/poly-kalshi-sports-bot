/**
 * Sports-arb backend endpoints.
 */
import { ArbitrageExecuteRequest, ArbitrageExecuteResponse } from '../../types';
import { apiUrl, getAuthHeaders } from '../../shared/api';

/** Route prefix of this strategy on the backend. */
const SPORTS_ARB_API = '/api';

/** URL of a sports-arb route, e.g. `sportsArbUrl('/tracking')`. */
export function sportsArbUrl(path: string): string {
  return apiUrl(`${SPORTS_ARB_API}${path}`);
}

/**
 * Automated trading status response type.
 */
export interface AutoTradeStatus {
  enabled: boolean;
  trade_count: number;
  max_trade_count: number;
  remaining: number;
  max_amount: number;
  min_duration_ms: number;
  /** Whether flexible order sizing is enabled. */
  flexible_mode: boolean;
  /** Maximum contracts per trade. */
  max_contracts: number;
  /** Minimum contracts required to trade. */
  min_contracts: number;
  /** Maximum adverse price allowed for an emergency close, in cents/contract. */
  neutralization_max_loss_cents: number;
  last_trade_time: string | null;
}

/**
 * Application settings response type.
 */
export interface AppSettings {
  /** Data refresh interval in seconds. */
  refresh_interval: number;
  /** Minimum profit margin for displayed opportunities. */
  min_profit_margin: number;
  /** Default USD amount used for arbitrage calculations. */
  default_bet_amount: number;
  /** Profit margin threshold for starting tracking. */
  tracking_threshold: number;
  updated_at: string | null;
}

/**
 * Executes an arbitrage trade by placing orders on both platforms.
 */
export async function executeArbitrage(
  request: ArbitrageExecuteRequest
): Promise<ArbitrageExecuteResponse> {
  const response = await fetch(sportsArbUrl('/arbitrage/execute'), {
    method: 'POST',
    headers: getAuthHeaders(),
    body: JSON.stringify(request),
  });
  const contentType = response.headers.get('content-type') || '';
  if (!contentType.includes('application/json')) {
    throw new Error(await response.text());
  }
  return response.json();
}

/**
 * Gets automated trading status.
 */
export async function getAutoTradeStatus(): Promise<AutoTradeStatus> {
  const response = await fetch(sportsArbUrl('/auto-trade/status'));
  return response.json();
}

/**
 * Enables automated trading.
 */
export async function enableAutoTrade(): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(sportsArbUrl('/auto-trade/enable'), {
    method: 'POST',
    headers: getAuthHeaders(),
  });
  return response.json();
}

/**
 * Disables automated trading.
 */
export async function disableAutoTrade(): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(sportsArbUrl('/auto-trade/disable'), {
    method: 'POST',
    headers: getAuthHeaders(),
  });
  return response.json();
}

/**
 * Resets the automated trade count.
 */
export async function resetAutoTradeCount(): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(sportsArbUrl('/auto-trade/reset'), {
    method: 'POST',
    headers: getAuthHeaders(),
  });
  return response.json();
}

/**
 * Updates automated trading settings.
 */
export async function updateAutoTradeSettings(
  settings: {
    max_amount?: number;
    min_duration_ms?: number;
    max_trade_count?: number;
    /** Whether flexible order sizing is enabled. */
    flexible_mode?: boolean;
    /** Maximum contracts per trade. */
    max_contracts?: number;
    /** Minimum contracts required. */
    min_contracts?: number;
    /** Maximum adverse price allowed for an emergency close, in cents/contract. */
    neutralization_max_loss_cents?: number;
  }
): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(sportsArbUrl('/auto-trade/settings'), {
    method: 'PUT',
    headers: getAuthHeaders(),
    body: JSON.stringify(settings),
  });
  return response.json();
}

/**
 * Gets application settings.
 */
export async function getAppSettings(): Promise<AppSettings> {
  const response = await fetch(sportsArbUrl('/settings'), {
    headers: getAuthHeaders(),
  });
  return response.json();
}

/**
 * Updates application settings.
 */
export async function updateAppSettings(
  settings: {
    refresh_interval?: number;
    min_profit_margin?: number;
    default_bet_amount?: number;
    tracking_threshold?: number;
  }
): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(sportsArbUrl('/settings'), {
    method: 'PUT',
    headers: getAuthHeaders(),
    body: JSON.stringify(settings),
  });
  return response.json();
}
