/**
 * Backend access shared by every strategy page: URL building, auth headers,
 * and the shared endpoints (manual orders, positions).
 *
 * Requests are same-origin: in development Vite proxies /api and /ws to the
 * backend (see vite.config.ts), in production the backend serves this UI.
 * Set VITE_API_URL only to reach a backend on another origin.
 */
import {
  OrderRequest,
  OrderResponse,
  PolymarketOrderRequest,
  PolymarketOrderResponse,
  UnifiedPositionsResponse,
} from '../types';

const API_ORIGIN = (import.meta.env.VITE_API_URL ?? '').replace(/\/$/, '');

/** Absolute or same-origin URL for a backend path such as `/api/health`. */
export function apiUrl(path: string): string {
  return `${API_ORIGIN}${path}`;
}

/** WebSocket URL of the backend's live feed. */
export function wsUrl(): string {
  const origin = API_ORIGIN || window.location.origin;
  return `${origin.replace(/^http/, 'ws')}/ws`;
}

/**
 * Gets authentication headers.
 */
export function getAuthHeaders(): HeadersInit {
  const token = localStorage.getItem('auth_token');
  const headers: HeadersInit = {
    'Content-Type': 'application/json',
  };

  if (token) {
    headers['Authorization'] = `Bearer ${token}`;
  }

  return headers;
}

// ==================== Manual trading ====================

/**
 * Creates a Kalshi market order.
 */
export async function createKalshiOrder(
  request: OrderRequest
): Promise<OrderResponse> {
  const response = await fetch(apiUrl('/api/order/kalshi'), {
    method: 'POST',
    headers: getAuthHeaders(),
    body: JSON.stringify(request),
  });
  return response.json();
}

/**
 * Creates a Polymarket market order.
 */
export async function createPolymarketOrder(
  request: PolymarketOrderRequest
): Promise<PolymarketOrderResponse> {
  const response = await fetch(apiUrl('/api/order/polymarket'), {
    method: 'POST',
    headers: getAuthHeaders(),
    body: JSON.stringify(request),
  });
  return response.json();
}

// ==================== Positions ====================

/**
 * Gets positions grouped into hedged pairs and standalone legs.
 */
export async function getUnifiedPositions(): Promise<UnifiedPositionsResponse> {
  const response = await fetch(apiUrl('/api/positions/unified'));
  if (!response.ok) {
    throw new Error(`Positions request failed (${response.status})`);
  }
  return response.json();
}
