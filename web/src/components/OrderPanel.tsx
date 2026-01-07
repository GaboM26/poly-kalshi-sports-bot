import { useState, useEffect, useCallback } from 'react';
import { KalshiPosition, PolymarketPosition, UnifiedPosition } from '../types';
import { 
  getKalshiPositions, 
  createKalshiOrder,
  getPolymarketPositions,
  createPolymarketOrder
} from '../utils/api';

interface OrderPanelProps {
  apiBaseUrl: string;
}

export function OrderPanel({ apiBaseUrl }: OrderPanelProps) {
  const [positions, setPositions] = useState<UnifiedPosition[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);

  // 转换 Kalshi 持仓为统一格式
  const convertKalshiPosition = (pos: KalshiPosition): UnifiedPosition => ({
    id: pos.ticker,
    platform: 'kalshi',
    ticker: pos.ticker,
    title: pos.event_ticker || pos.ticker,
    size: pos.position,
    side: pos.position > 0 ? 'yes' : 'no',
    value: Math.abs(pos.market_exposure) / 100,
    pnl: pos.realized_pnl ? pos.realized_pnl / 100 : undefined,
  });

  // 转换 Polymarket 持仓为统一格式
  const convertPolyPosition = (pos: PolymarketPosition): UnifiedPosition => ({
    id: pos.conditionId || pos.asset || String(pos.id) || Math.random().toString(),
    platform: 'polymarket',
    ticker: pos.conditionId || pos.asset || '',
    title: pos.title || pos.asset || '未知市场',
    size: pos.size ? parseFloat(pos.size) : 0,
    avgPrice: pos.avgPrice ? parseFloat(pos.avgPrice) : undefined,
    curPrice: pos.curPrice ? parseFloat(pos.curPrice) : undefined,
    value: pos.value ? parseFloat(pos.value) : undefined,
    pnl: pos.pnl ? parseFloat(pos.pnl) : undefined,
    pnlPercent: pos.pnlPercent ? parseFloat(pos.pnlPercent) : undefined,
  });

  // 加载数据 - 分别获取，避免一个失败影响另一个
  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    
    let kalshiPositions: KalshiPosition[] = [];
    let polyPositions: PolymarketPosition[] = [];
    const errors: string[] = [];

    // 获取 Kalshi 持仓
    try {
      const kalshiRes = await getKalshiPositions(apiBaseUrl);
      if (kalshiRes.positions) {
        kalshiPositions = kalshiRes.positions;
      }
      if (kalshiRes.error) {
        errors.push(`Kalshi: ${kalshiRes.error}`);
      }
    } catch (e) {
      errors.push(`Kalshi: ${e instanceof Error ? e.message : '获取失败'}`);
    }

    // 获取 Polymarket 持仓
    try {
      const polyRes = await getPolymarketPositions(apiBaseUrl);
      if (polyRes.positions) {
        polyPositions = polyRes.positions;
      }
      if (polyRes.error) {
        errors.push(`Poly: ${polyRes.error}`);
      }
    } catch (e) {
      errors.push(`Poly: ${e instanceof Error ? e.message : '获取失败'}`);
    }

    const unified: UnifiedPosition[] = [];
    
    // 添加 Kalshi 持仓（过滤掉 position=0 的）
    for (const pos of kalshiPositions) {
      if (pos.position !== 0) {
        unified.push(convertKalshiPosition(pos));
      }
    }
    
    // 添加 Polymarket 持仓（过滤掉 size=0 的）
    for (const pos of polyPositions) {
      const size = pos.size ? parseFloat(pos.size) : 0;
      if (size !== 0) {
        unified.push(convertPolyPosition(pos));
      }
    }
    
    setPositions(unified);
    
    // 只有在没有任何持仓时才显示完整错误
    // 否则只在底部显示警告
    if (errors.length > 0 && unified.length === 0) {
      setError(errors.join('; '));
    } else if (errors.length > 0) {
      // 有持仓但也有错误，显示为警告
      setError(errors.join('; '));
    } else {
      setError(null);
    }
    
    setLoading(false);
  }, [apiBaseUrl]);

  // 初始加载和定时刷新
  useEffect(() => {
    loadData();
    const interval = setInterval(loadData, 15000); // 每15秒刷新
    return () => clearInterval(interval);
  }, [loadData]);

  // Kalshi 卖出持仓
  const handleSellKalshi = async (position: UnifiedPosition) => {
    if (position.size === 0) return;
    
    // Kalshi 卖出逻辑：
    // position > 0 表示持有 YES，要卖出 YES
    // position < 0 表示持有 NO，要卖出 NO
    const side = position.size > 0 ? 'yes' : 'no';
    const count = Math.abs(position.size);
    
    setActionLoading(position.id);
    try {
      const result = await createKalshiOrder(apiBaseUrl, {
        ticker: position.ticker,
        side: side,
        action: 'sell',
        count: count,
      });
      
      if (result.success) {
        loadData(); // 刷新数据
      } else {
        alert(`卖出失败: ${result.error}`);
      }
    } catch (e) {
      alert(`卖出失败: ${e instanceof Error ? e.message : '未知错误'}`);
    } finally {
      setActionLoading(null);
    }
  };

  // Polymarket 卖出持仓
  const handleSellPoly = async (position: UnifiedPosition) => {
    if (position.size === 0) return;
    
    setActionLoading(position.id);
    try {
      // Polymarket 卖出：amount 是 USDC 金额
      // 使用当前价值作为卖出金额
      const amount = position.value || Math.abs(position.size);
      
      const result = await createPolymarketOrder(apiBaseUrl, {
        token_id: position.ticker,
        side: 'sell',
        amount: amount,
      });
      
      if (result.success) {
        loadData(); // 刷新数据
      } else {
        alert(`卖出失败: ${result.error}`);
      }
    } catch (e) {
      alert(`卖出失败: ${e instanceof Error ? e.message : '未知错误'}`);
    } finally {
      setActionLoading(null);
    }
  };

  // 统一卖出处理
  const handleSell = (position: UnifiedPosition) => {
    if (position.platform === 'kalshi') {
      handleSellKalshi(position);
    } else {
      handleSellPoly(position);
    }
  };

  // 格式化数值
  const formatValue = (value?: number) => {
    if (value === undefined) return '-';
    return `$${value.toFixed(2)}`;
  };

  const formatPnl = (pnl?: number, percent?: number) => {
    if (pnl === undefined) return '-';
    const sign = pnl >= 0 ? '+' : '';
    const percentStr = percent !== undefined ? ` (${sign}${percent.toFixed(1)}%)` : '';
    return `${sign}$${pnl.toFixed(2)}${percentStr}`;
  };

  return (
    <div className="card h-full flex flex-col overflow-hidden">
      {/* 标题栏 */}
      <div className="flex items-center justify-between border-b border-[--border-color] px-3 py-2 flex-shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[--text-primary]">💼 持仓</span>
          <span className="text-[10px] text-[--text-muted]">({positions.length})</span>
        </div>
        <button
          className="px-2 py-1 text-[--text-muted] hover:text-[--text-secondary] text-xs"
          onClick={loadData}
          disabled={loading}
          title="刷新"
        >
          {loading ? '...' : '🔄'}
        </button>
      </div>

      {/* 内容区 */}
      <div className="flex-1 overflow-y-auto p-2">
        {loading && positions.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[--text-muted] text-xs">
            加载中...
          </div>
        ) : error && positions.length === 0 ? (
          <div className="flex items-center justify-center h-full text-red-400 text-xs">
            {error}
          </div>
        ) : positions.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[--text-muted] text-xs">
            暂无持仓
          </div>
        ) : (
          <div className="space-y-1.5">
            {positions.map((pos) => (
              <div
                key={`${pos.platform}-${pos.id}`}
                className="bg-[--bg-tertiary] rounded p-2"
              >
                {/* 第一行：平台标记 + 市场名 */}
                <div className="flex items-center justify-between mb-1">
                  <div className="flex items-center gap-2 flex-1 min-w-0">
                    <span className={`text-[9px] px-1.5 py-0.5 rounded font-medium ${
                      pos.platform === 'kalshi' 
                        ? 'bg-blue-500/20 text-blue-400' 
                        : 'bg-purple-500/20 text-purple-400'
                    }`}>
                      {pos.platform === 'kalshi' ? 'K' : 'P'}
                    </span>
                    <span className="text-xs font-medium text-[--text-primary] truncate" title={pos.ticker}>
                      {pos.title || pos.ticker}
                    </span>
                  </div>
                </div>
                
                {/* 第二行：持仓信息 */}
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-3">
                    {/* 数量 */}
                    <div className="flex items-center gap-1">
                      <span className={`text-xs font-mono ${pos.size > 0 ? 'text-green-400' : 'text-red-400'}`}>
                        {pos.size > 0 ? '+' : ''}{pos.size.toFixed(pos.platform === 'polymarket' ? 2 : 0)}
                      </span>
                      {pos.side && pos.platform === 'kalshi' && (
                        <span className={`text-[9px] px-1 rounded ${
                          pos.side === 'yes' ? 'bg-green-500/20 text-green-400' : 'bg-red-500/20 text-red-400'
                        }`}>
                          {pos.side.toUpperCase()}
                        </span>
                      )}
                    </div>
                    
                    {/* 价值 */}
                    <span className="text-[10px] text-[--text-muted]">
                      {formatValue(pos.value)}
                    </span>
                    
                    {/* 盈亏 */}
                    {pos.pnl !== undefined && (
                      <span className={`text-[10px] ${pos.pnl >= 0 ? 'text-green-400' : 'text-red-400'}`}>
                        {formatPnl(pos.pnl, pos.pnlPercent)}
                      </span>
                    )}
                  </div>
                  
                  {/* 卖出按钮 */}
                  <button
                    className="px-2 py-1 text-[10px] bg-red-500/20 text-red-400 rounded hover:bg-red-500/30 disabled:opacity-50"
                    onClick={() => handleSell(pos)}
                    disabled={actionLoading === pos.id || pos.size === 0}
                  >
                    {actionLoading === pos.id ? '...' : '卖出'}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* 底部错误提示（如果有部分错误） */}
      {error && positions.length > 0 && (
        <div className="border-t border-[--border-color] px-2 py-1 bg-yellow-500/10">
          <span className="text-[9px] text-yellow-400">⚠️ {error}</span>
        </div>
      )}
    </div>
  );
}
