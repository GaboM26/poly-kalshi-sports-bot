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

// 性能监控相关类型
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

// 匹配市场数据（包含实时价格）
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
  // 套利相关
  has_opportunity: boolean;
  profit_margin: number;
  expected_profit: number;  // 净利润（扣除费用后）
  gross_profit?: number;    // 毛利润（扣除费用前）
  kalshi_contracts?: number;  // Kalshi 合约数量
  kalshi_fee?: number;      // Kalshi 交易费用
  arbitrage_type?: string;
}

export interface LogEntry {
  time: string;
  level: 'info' | 'success' | 'warning' | 'error';
  message: string;
}

// 套利追踪记录
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
  duration_ms: number | null;  // 毫秒级持续时间
  max_profit_margin: number;
  max_profit_time: string | null;
  profit_history: ProfitHistoryEntry[];
  // 深度信息
  poly_ask_depth: number;  // Polymarket ask 深度 (USD)
  kalshi_ask_depth: number;  // Kalshi ask 深度 (contracts)
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

// 数据覆盖率
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

// 账户余额
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

// ==================== 交易相关类型 ====================

// Kalshi 订单
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

// Kalshi 持仓
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

// Kalshi 下单请求
export interface KalshiOrderRequest {
  ticker: string;
  side: 'yes' | 'no';
  action: 'buy' | 'sell';
  count: number;
}

// 兼容旧代码
export type OrderRequest = KalshiOrderRequest;

// Polymarket 下单请求
export interface PolymarketOrderRequest {
  token_id: string;
  side: 'buy' | 'sell';
  amount: number;  // USDC 金额
}

// 下单响应
export interface OrderResponse {
  success: boolean;
  order?: KalshiOrder;
  elapsed_ms?: number;
  error?: string;
}

// Polymarket 下单响应
export interface PolymarketOrderResponse {
  success: boolean;
  order_id?: string;
  status?: string;
  taking_amount?: string;
  making_amount?: string;
  elapsed_ms?: number;
  error?: string;
}

// Polymarket 订单
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

// Polymarket 持仓
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
  // 可能有其他字段，根据 API 响应动态处理
  [key: string]: unknown;
}

// 统一持仓类型（用于合并展示）
export interface UnifiedPosition {
  id: string;
  platform: 'kalshi' | 'polymarket';
  ticker: string;        // Kalshi ticker 或 Poly conditionId
  title?: string;        // 市场标题
  size: number;          // 持仓数量
  side?: 'yes' | 'no' | 'buy' | 'sell';
  avgPrice?: number;     // 平均价格
  curPrice?: number;     // 当前价格
  value?: number;        // 持仓价值
  pnl?: number;          // 盈亏
  pnlPercent?: number;   // 盈亏百分比
}

// 订单列表响应
export interface OrdersResponse {
  orders: KalshiOrder[];
  error?: string;
}

// Polymarket 订单列表响应
export interface PolymarketOrdersResponse {
  orders: PolymarketOrder[];
  error?: string;
}

// 持仓列表响应
export interface PositionsResponse {
  positions: KalshiPosition[];
  error?: string;
}

// 套利执行请求
export interface ArbitrageExecuteRequest {
  kalshi_ticker: string;
  kalshi_side: 'yes' | 'no';
  kalshi_bet: number;
  kalshi_price: number;
  poly_token_id: string;
  poly_side: 'buy' | 'sell';
  poly_amount: number;
}

// 套利执行响应
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

// 订单簿深度
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
