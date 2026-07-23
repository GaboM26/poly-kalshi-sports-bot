export interface Market {
  platform: 'Kalshi' | 'Polymarket';
  event_id: string;
  event_name: string;
  yes_price: number;
  no_price: number;
  volume: number;
  team_name?: string;
  category: string;
  end_time?: string;
}

export interface ArbitrageOpportunity {
  kalshi_market: Market;
  polymarket_market: Market;
  arbitrage_type: ArbitrageType;
  profit_margin: number;
  optimal_bet: [number, number];
  expected_profit: number;
  match_confidence: number;
}

export type ArbitrageType = 
  | 'KalshiYesPolymarketNo'
  | 'KalshiNoPolymarketYes';

export interface ScanResponse {
  kalshi_markets: Market[];
  polymarket_markets: Market[];
  opportunities: ArbitrageOpportunity[];
  matched_count: number;
}

export interface WsMessage {
  type: 'opportunity' | 'opportunities' | 'opportunities_list' | 'matched_markets_list' | 'scan_started' | 'scan_completed' | 'log' | 'ping' | 'pong' | 'connected' | 'stats' | 'metrics';
  data?: ArbitrageOpportunity | ArbitrageOpportunity[] | MatchedMarketData[] | MetricsReport;
  kalshi_count?: number;
  polymarket_count?: number;
  matched_count?: number;
  opportunities_count?: number;
  count?: number;
  level?: string;
  message?: string;
  timestamp?: string;
}

// Performance monitoring types
export interface OperationStats {
  name: string;
  count: number;
  avg_ms: number;
  max_ms: number;
  total_ms: number;
}

export interface ApiLatency {
  kalshi_ms?: number;
  polymarket_ms?: number;
}

export interface MetricsReport {
  operations: OperationStats[];
  api_latency: ApiLatency;
}

// Matched market data, including real-time prices
export interface MatchedMarketData {
  event_name: string;
  team_name: string;
  game_date?: string;               // Game date in YYYY-MM-DD format
  kalshi_market_id: string;
  polymarket_market_id: string;
  poly_token_id?: string;           // Polymarket token_id for Yes orderbook
  poly_opponent_token_id?: string;  // Polymarket opponent token_id for No orderbook
  kalshi_yes_price: number;
  kalshi_no_price: number;
  poly_yes_price: number;
  poly_no_price: number;
  kalshi_ready: boolean;
  poly_ready: boolean;
  both_ready: boolean;
  confidence: number;
  end_time?: string;
  // Arbitrage fields
  has_opportunity: boolean;
  profit_margin: number;
  expected_profit: number;  // Net profit after fees
  gross_profit?: number;    // Gross profit before fees
  kalshi_contracts?: number;  // Number of Kalshi contracts
  kalshi_fee?: number;      // Kalshi trading fee
  arbitrage_type?: string;
}

export interface LogEntry {
  time: string;
  level: 'info' | 'success' | 'warning' | 'error';
  message: string;
}

// Arbitrage tracking record
export interface ProfitHistoryEntry {
  time: string;
  profit_margin: number;
  kalshi_price: number;
  polymarket_price: number;
}

export interface TrackingRecord {
  event_name: string;
  team_name: string;
  kalshi_market_id: string;
  polymarket_market_id: string;
  start_time: string;
  end_time: string | null;
  duration_seconds: number | null;
  duration_ms: number | null;  // Duration in milliseconds
  max_profit_margin: number;
  max_profit_time: string | null;
  profit_history: ProfitHistoryEntry[];
  // Depth information
  poly_ask_depth: number;  // Polymarket ask depth (USD)
  kalshi_ask_depth: number;  // Kalshi ask depth (contracts)
}

export interface ActiveTracking {
  event_name: string;
  team_name: string;
  start_time: string;
  duration_seconds: number;
  max_profit_margin: number;
}

export interface TrackingStats {
  active_count: number;
  completed_count: number;
  active: ActiveTracking[];
  recent_completed: TrackingRecord[];
}

// Data coverage
export interface DataCoverage {
  total_markets: number;
  kalshi_ready: number;
  polymarket_ready: number;
  both_ready: number;
  kalshi_coverage: string;
  polymarket_coverage: string;
  full_coverage: string;
  kalshi_connected: boolean;
  polymarket_connected: boolean;
  kalshi_latency_ms?: number;
  polymarket_latency_ms?: number;
}

// Account balances
export interface PlatformBalance {
  available: boolean;
  balance?: number;
  portfolio_value?: number;
  error?: string;
  pnl?: string;
  trades?: number;
  positions?: number;
  updated_ts?: number;
}

export interface AccountBalance {
  kalshi: PlatformBalance;
  polymarket: PlatformBalance;
}

// ==================== Trading Types ====================

// Kalshi order
export interface KalshiOrder {
  order_id: string;
  user_id?: string;
  client_order_id?: string;
  ticker: string;
  side: 'yes' | 'no';
  action: 'buy' | 'sell';
  type: 'limit' | 'market';
  status: 'resting' | 'canceled' | 'executed' | 'pending';
  yes_price?: number;
  no_price?: number;
  fill_count: number;
  remaining_count: number;
  initial_count: number;
  taker_fees?: number;
  maker_fees?: number;
  taker_fill_cost?: number;
  maker_fill_cost?: number;
  created_time: string;
  last_update_time?: string;
}

// Kalshi position
export interface KalshiPosition {
  ticker: string;
  event_ticker?: string;
  market_exposure: number;
  position: number;
  resting_orders_count: number;
  fees_paid?: number;
  total_traded?: number;
  realized_pnl?: number;
}

// Kalshi order request
export interface KalshiOrderRequest {
  ticker: string;
  side: 'yes' | 'no';
  action: 'buy' | 'sell';
  count: number;
}

// Backward compatibility
export type OrderRequest = KalshiOrderRequest;

// Polymarket order request
export interface PolymarketOrderRequest {
  token_id: string;
  side: 'buy' | 'sell';
  amount: number;  // USDC amount
}

// Order response
export interface OrderResponse {
  success: boolean;
  order?: KalshiOrder;
  elapsed_ms?: number;
  error?: string;
}

// Polymarket order response
export interface PolymarketOrderResponse {
  success: boolean;
  order_id?: string;
  status?: string;
  taking_amount?: string;
  making_amount?: string;
  elapsed_ms?: number;
  error?: string;
}

// Polymarket order
export interface PolymarketOrder {
  id: string;
  status: string;
  owner: string;
  maker_address: string;
  market: string;
  asset_id: string;
  side: string;
  original_size: string;
  size_matched: string;
  price: string;
  outcome: string;
  created_at: number;
  order_type: string;
}

// Polymarket position
export interface PolymarketPosition {
  id?: string;
  asset?: string;
  conditionId?: string;
  outcomeIndex?: number;
  size?: string;
  avgPrice?: string;
  curPrice?: string;
  value?: string;
  pnl?: string;
  pnlPercent?: string;
  title?: string;
  // Additional fields may be handled dynamically from the API response.
  [key: string]: unknown;
}

// Unified position type for combined display
export interface UnifiedPosition {
  id: string;
  platform: 'kalshi' | 'polymarket';
  ticker: string;        // Kalshi ticker or Polymarket condition ID
  title?: string;        // Market title
  size: number;          // Position size
  side?: 'yes' | 'no' | 'buy' | 'sell';
  avgPrice?: number;     // Average price
  curPrice?: number;     // Current price
  value?: number;        // Position value
  pnl?: number;          // Profit and loss
  pnlPercent?: number;   // Profit and loss percentage
}

// Orders list response
export interface OrdersResponse {
  orders: KalshiOrder[];
  error?: string;
}

// Polymarket orders list response
export interface PolymarketOrdersResponse {
  orders: PolymarketOrder[];
  error?: string;
}

// Positions list response
export interface PositionsResponse {
  positions: KalshiPosition[];
  error?: string;
}

// Arbitrage execution request
export interface ArbitrageExecuteRequest {
  kalshi_ticker: string;
  kalshi_side: 'yes' | 'no';
  kalshi_bet: number;
  kalshi_price: number;
  poly_token_id: string;
  poly_side: 'buy' | 'sell';
  poly_amount: number;
}

// Arbitrage execution response
export interface ArbitrageExecuteResponse {
  success: boolean;
  error?: string;
  kalshi: {
    success: boolean;
    order?: Record<string, unknown>;
    elapsed_ms?: number;
    count?: number;
    error?: string;
  };
  polymarket: {
    success: boolean;
    order_id?: string;
    status?: string;
    elapsed_ms?: number;
    amount?: number;
    error?: string;
  };
}

// Order book depth
export interface SideDepth {
  price?: number;
  size?: number;
}

export interface PlatformDepthDual {
  yes: SideDepth;
  no: SideDepth;
}

export interface OrderbookDepthResponse {
  kalshi?: PlatformDepthDual;
  polymarket?: PlatformDepthDual;
}
