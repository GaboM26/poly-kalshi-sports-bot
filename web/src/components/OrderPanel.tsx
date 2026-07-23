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

  // Convert a Kalshi position to the unified format.
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

  // Convert a Polymarket position to the unified format.
  const convertPolyPosition = (pos: PolymarketPosition): UnifiedPosition => ({
    id: pos.conditionId || pos.asset || String(pos.id) || Math.random().toString(),
    platform: 'polymarket',
    ticker: pos.conditionId || pos.asset || '',
    title: pos.title || pos.asset || 'Unknown Market',
    size: pos.size ? parseFloat(pos.size) : 0,
    avgPrice: pos.avgPrice ? parseFloat(pos.avgPrice) : undefined,
    curPrice: pos.curPrice ? parseFloat(pos.curPrice) : undefined,
    value: pos.value ? parseFloat(pos.value) : undefined,
    pnl: pos.pnl ? parseFloat(pos.pnl) : undefined,
    pnlPercent: pos.pnlPercent ? parseFloat(pos.pnlPercent) : undefined,
  });

  // Load each source separately so one failure does not affect the other.
  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    
    let kalshiPositions: KalshiPosition[] = [];
    let polyPositions: PolymarketPosition[] = [];
    const errors: string[] = [];

    // Fetch Kalshi positions.
    try {
      const kalshiRes = await getKalshiPositions(apiBaseUrl);
      if (kalshiRes.positions) {
        kalshiPositions = kalshiRes.positions;
      }
      if (kalshiRes.error) {
        errors.push(`Kalshi: ${kalshiRes.error}`);
      }
    } catch (e) {
      errors.push(`Kalshi: ${e instanceof Error ? e.message : 'Fetch failed'}`);
    }

    // Fetch Polymarket positions.
    try {
      const polyRes = await getPolymarketPositions(apiBaseUrl);
      if (polyRes.positions) {
        polyPositions = polyRes.positions;
      }
      if (polyRes.error) {
        errors.push(`Poly: ${polyRes.error}`);
      }
    } catch (e) {
      errors.push(`Poly: ${e instanceof Error ? e.message : 'Fetch failed'}`);
    }

    const unified: UnifiedPosition[] = [];
    
    // Add nonzero Kalshi positions.
    for (const pos of kalshiPositions) {
      if (pos.position !== 0) {
        unified.push(convertKalshiPosition(pos));
      }
    }
    
    // Add nonzero Polymarket positions.
    for (const pos of polyPositions) {
      const size = pos.size ? parseFloat(pos.size) : 0;
      if (size !== 0) {
        unified.push(convertPolyPosition(pos));
      }
    }
    
    setPositions(unified);
    
    // Show the full error only when there are no positions.
    // Otherwise, display a warning at the bottom.
    if (errors.length > 0 && unified.length === 0) {
      setError(errors.join('; '));
    } else if (errors.length > 0) {
      // Display an error as a warning when positions are still available.
      setError(errors.join('; '));
    } else {
      setError(null);
    }
    
    setLoading(false);
  }, [apiBaseUrl]);

  // Initial load and scheduled refresh.
  useEffect(() => {
    loadData();
    const interval = setInterval(loadData, 15000); // Refresh every 15 seconds
    return () => clearInterval(interval);
  }, [loadData]);

  // Sell a Kalshi position.
  const handleSellKalshi = async (position: UnifiedPosition) => {
    if (position.size === 0) return;
    
    // Kalshi selling logic:
    // position > 0 means holding YES, so sell YES.
    // position < 0 means holding NO, so sell NO.
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
        loadData(); // Refresh data
      } else {
        alert(`Sell failed: ${result.error}`);
      }
    } catch (e) {
      alert(`Sell failed: ${e instanceof Error ? e.message : 'Unknown error'}`);
    } finally {
      setActionLoading(null);
    }
  };

  // Sell a Polymarket position.
  const handleSellPoly = async (position: UnifiedPosition) => {
    if (position.size === 0) return;
    
    setActionLoading(position.id);
    try {
      // For Polymarket sells, amount is in USDC.
      // Use current value as the sell amount.
      const amount = position.value || Math.abs(position.size);
      
      const result = await createPolymarketOrder(apiBaseUrl, {
        token_id: position.ticker,
        side: 'sell',
        amount: amount,
      });
      
      if (result.success) {
        loadData(); // Refresh data
      } else {
        alert(`Sell failed: ${result.error}`);
      }
    } catch (e) {
      alert(`Sell failed: ${e instanceof Error ? e.message : 'Unknown error'}`);
    } finally {
      setActionLoading(null);
    }
  };

  // Unified sell handler.
  const handleSell = (position: UnifiedPosition) => {
    if (position.platform === 'kalshi') {
      handleSellKalshi(position);
    } else {
      handleSellPoly(position);
    }
  };

  // Format values.
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
      {/* Header */}
      <div className="flex items-center justify-between border-b border-[--border-color] px-3 py-2 flex-shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium text-[--text-primary]">💼 Positions</span>
          <span className="text-[10px] text-[--text-muted]">({positions.length})</span>
        </div>
        <button
          className="px-2 py-1 text-[--text-muted] hover:text-[--text-secondary] text-xs"
          onClick={loadData}
          disabled={loading}
          title="Refresh"
        >
          {loading ? '...' : '🔄'}
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-2">
        {loading && positions.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[--text-muted] text-xs">
            Loading...
          </div>
        ) : error && positions.length === 0 ? (
          <div className="flex items-center justify-center h-full text-red-400 text-xs">
            {error}
          </div>
        ) : positions.length === 0 ? (
          <div className="flex items-center justify-center h-full text-[--text-muted] text-xs">
            No positions
          </div>
        ) : (
          <div className="space-y-1.5">
            {positions.map((pos) => (
              <div
                key={`${pos.platform}-${pos.id}`}
                className="bg-[--bg-tertiary] rounded p-2"
              >
                {/* First row: platform badge and market name */}
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
                
                {/* Second row: position details */}
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-3">
                    {/* Quantity */}
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
                    
                    {/* Value */}
                    <span className="text-[10px] text-[--text-muted]">
                      {formatValue(pos.value)}
                    </span>
                    
                    {/* Profit and loss */}
                    {pos.pnl !== undefined && (
                      <span className={`text-[10px] ${pos.pnl >= 0 ? 'text-green-400' : 'text-red-400'}`}>
                        {formatPnl(pos.pnl, pos.pnlPercent)}
                      </span>
                    )}
                  </div>
                  
                  {/* Sell button */}
                  <button
                    className="px-2 py-1 text-[10px] bg-red-500/20 text-red-400 rounded hover:bg-red-500/30 disabled:opacity-50"
                    onClick={() => handleSell(pos)}
                    disabled={actionLoading === pos.id || pos.size === 0}
                  >
                    {actionLoading === pos.id ? '...' : 'Sell'}
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Bottom error notice for partial failures */}
      {error && positions.length > 0 && (
        <div className="border-t border-[--border-color] px-2 py-1 bg-yellow-500/10">
          <span className="text-[9px] text-yellow-400">⚠️ {error}</span>
        </div>
      )}
    </div>
  );
}
