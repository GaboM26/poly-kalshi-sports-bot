import { 
  Market, 
  ScanResponse, 
  OrderRequest, 
  OrderResponse, 
  OrdersResponse, 
  PositionsResponse,
  PolymarketOrderRequest,
  PolymarketOrderResponse,
  PolymarketOrdersResponse,
  PolymarketPosition,
  ArbitrageExecuteRequest,
  ArbitrageExecuteResponse
} from '../types';

const API_BASE = '/api';

/**
 * 获取认证头
 */
function getAuthHeaders(): HeadersInit {
  const token = localStorage.getItem('auth_token');
  const headers: HeadersInit = {
    'Content-Type': 'application/json',
  };
  
  if (token) {
    headers['Authorization'] = `Bearer ${token}`;
  }
  
  return headers;
}

export async function fetchKalshiMarkets(): Promise<Market[]> {
  const response = await fetch(`${API_BASE}/markets/kalshi`);
  if (!response.ok) {
    throw new Error('获取 Kalshi 市场失败');
  }
  return response.json();
}

export async function fetchPolymarketMarkets(): Promise<Market[]> {
  const response = await fetch(`${API_BASE}/markets/polymarket`);
  if (!response.ok) {
    throw new Error('获取 Polymarket 市场失败');
  }
  return response.json();
}

export async function fetchOpportunities(): Promise<ScanResponse> {
  const response = await fetch(`${API_BASE}/opportunities`);
  if (!response.ok) {
    throw new Error('获取套利机会失败');
  }
  return response.json();
}

export async function triggerScan(): Promise<ScanResponse> {
  const response = await fetch(`${API_BASE}/scan`, {
    method: 'POST',
  });
  if (!response.ok) {
    throw new Error('触发扫描失败');
  }
  return response.json();
}

export async function checkHealth(): Promise<{ status: string; version: string }> {
  const response = await fetch(`${API_BASE}/health`);
  if (!response.ok) {
    throw new Error('健康检查失败');
  }
  return response.json();
}

// ==================== 交易相关 API ====================

/**
 * 创建 Kalshi 市价订单
 */
export async function createKalshiOrder(
  baseUrl: string,
  request: OrderRequest
): Promise<OrderResponse> {
  const response = await fetch(`${baseUrl}/api/order/kalshi`, {
    method: 'POST',
    headers: getAuthHeaders(),
    body: JSON.stringify(request),
  });
  return response.json();
}

/**
 * 获取 Kalshi 订单列表
 */
export async function getKalshiOrders(
  baseUrl: string,
  status?: string
): Promise<OrdersResponse> {
  const url = status 
    ? `${baseUrl}/api/orders/kalshi?status=${status}`
    : `${baseUrl}/api/orders/kalshi`;
  const response = await fetch(url);
  return response.json();
}

/**
 * 获取 Kalshi 持仓列表
 */
export async function getKalshiPositions(
  baseUrl: string
): Promise<PositionsResponse> {
  const response = await fetch(`${baseUrl}/api/positions/kalshi`);
  return response.json();
}

/**
 * 取消 Kalshi 订单
 */
export async function cancelKalshiOrder(
  baseUrl: string,
  orderId: string
): Promise<{ success: boolean; error?: string }> {
  const response = await fetch(`${baseUrl}/api/orders/kalshi/${orderId}`, {
    method: 'DELETE',
  });
  return response.json();
}

// ==================== Polymarket 交易 API ====================

/**
 * 创建 Polymarket 市价订单
 */
export async function createPolymarketOrder(
  baseUrl: string,
  request: PolymarketOrderRequest
): Promise<PolymarketOrderResponse> {
  const response = await fetch(`${baseUrl}/api/order/polymarket`, {
    method: 'POST',
    headers: getAuthHeaders(),
    body: JSON.stringify(request),
  });
  return response.json();
}

/**
 * 获取 Polymarket 订单列表
 */
export async function getPolymarketOrders(
  baseUrl: string
): Promise<PolymarketOrdersResponse> {
  const response = await fetch(`${baseUrl}/api/orders/polymarket`);
  return response.json();
}

/**
 * 取消 Polymarket 订单
 */
export async function cancelPolymarketOrder(
  baseUrl: string,
  orderId: string
): Promise<{ success: boolean; error?: string }> {
  const response = await fetch(`${baseUrl}/api/orders/polymarket/${orderId}`, {
    method: 'DELETE',
  });
  return response.json();
}

/**
 * 获取 Polymarket 持仓列表
 */
export async function getPolymarketPositions(
  baseUrl: string
): Promise<{ positions: PolymarketPosition[]; error?: string }> {
  const response = await fetch(`${baseUrl}/api/positions/polymarket`);
  return response.json();
}

// ==================== 套利执行 API ====================

/**
 * 执行套利交易（同时在两个平台下单）
 */
export async function executeArbitrage(
  baseUrl: string,
  request: ArbitrageExecuteRequest
): Promise<ArbitrageExecuteResponse> {
  const response = await fetch(`${baseUrl}/api/arbitrage/execute`, {
    method: 'POST',
    headers: getAuthHeaders(),
    body: JSON.stringify(request),
  });
  return response.json();
}

// ==================== 自动下单 API ====================

/**
 * 自动下单状态响应类型
 */
export interface AutoTradeStatus {
  enabled: boolean;
  trade_count: number;
  max_trade_count: number;
  remaining: number;
  max_amount: number;
  min_duration_ms: number;
  last_trade_time: string | null;
}

/**
 * 获取自动下单状态
 */
export async function getAutoTradeStatus(
  baseUrl: string
): Promise<AutoTradeStatus> {
  const response = await fetch(`${baseUrl}/api/auto-trade/status`);
  return response.json();
}

/**
 * 开启自动下单
 */
export async function enableAutoTrade(
  baseUrl: string
): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(`${baseUrl}/api/auto-trade/enable`, {
    method: 'POST',
    headers: getAuthHeaders(),
  });
  return response.json();
}

/**
 * 关闭自动下单
 */
export async function disableAutoTrade(
  baseUrl: string
): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(`${baseUrl}/api/auto-trade/disable`, {
    method: 'POST',
    headers: getAuthHeaders(),
  });
  return response.json();
}

/**
 * 重置自动下单次数
 */
export async function resetAutoTradeCount(
  baseUrl: string
): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(`${baseUrl}/api/auto-trade/reset`, {
    method: 'POST',
    headers: getAuthHeaders(),
  });
  return response.json();
}

/**
 * 更新自动下单设置
 */
export async function updateAutoTradeSettings(
  baseUrl: string,
  settings: {
    max_amount?: number;
    min_duration_ms?: number;
    max_trade_count?: number;
  }
): Promise<{ success: boolean; message?: string; error?: string }> {
  const response = await fetch(`${baseUrl}/api/auto-trade/settings`, {
    method: 'PUT',
    headers: getAuthHeaders(),
    body: JSON.stringify(settings),
  });
  return response.json();
}
