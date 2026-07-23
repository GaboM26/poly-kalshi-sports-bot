import { useState } from 'react';
import { MatchedMarketData } from '../types';
import { createKalshiOrder, createPolymarketOrder, executeArbitrage } from '../utils/api';

interface OrderFormProps {
  market: MatchedMarketData;
  apiBaseUrl: string;
  onOrderPlaced?: () => void;
}

type Side = 'yes' | 'no';
type Action = 'buy' | 'sell';

export function OrderForm({ market, apiBaseUrl, onOrderPlaced }: OrderFormProps) {
  const [count, setCount] = useState(1);
  const [amount, setAmount] = useState(10); // Polymarket USDC amount
  const [loading, setLoading] = useState<string | null>(null);
  const [result, setResult] = useState<{ success: boolean; message: string; elapsed_ms?: number } | null>(null);

  // Place a Kalshi order.
  const handleKalshiOrder = async (side: Side, action: Action) => {
    const loadingKey = `kalshi_${side}_${action}`;
    setLoading(loadingKey);
    setResult(null);

    try {
      const response = await createKalshiOrder(apiBaseUrl, {
        ticker: market.kalshi_market_id,
        side,
        action,
        count,
      });

      if (response.success) {
        setResult({
          success: true,
          message: `Kalshi ${action} ${side.toUpperCase()} succeeded! Filled ${response.order?.fill_count || 0}`,
          elapsed_ms: response.elapsed_ms,
        });
        onOrderPlaced?.();
      } else {
        setResult({
          success: false,
          message: response.error || 'Order failed',
        });
      }
    } catch (e) {
      setResult({
        success: false,
        message: e instanceof Error ? e.message : 'Order failed',
      });
    } finally {
      setLoading(null);
    }
  };

  // Place a Polymarket order.
  const handlePolyOrder = async (side: 'buy' | 'sell', teamIndex: 0 | 1) => {
    // Get the matching token ID from market data.
    // market has polymarket_market_id as the condition ID.
    // The specific token ID must be obtained from matched data.
    // Temporarily use market_id as the token (it should be token_id_a or token_id_b).
    
    const loadingKey = `poly_${side}_${teamIndex}`;
    setLoading(loadingKey);
    setResult(null);

    try {
      // A concrete token ID is required here.
      // The matched frontend data may not contain one.
      // In production, fetch complete market data including the token ID from the backend.
      const tokenId = market.polymarket_market_id; // This should be the concrete token ID.
      
      if (!tokenId) {
        setResult({
          success: false,
          message: 'Token ID not found',
        });
        return;
      }

      const response = await createPolymarketOrder(apiBaseUrl, {
        token_id: tokenId,
        side: side,
        amount: amount,
      });

      if (response.success) {
        setResult({
          success: true,
          message: `Polymarket ${side.toUpperCase()} succeeded! Order ID: ${response.order_id?.slice(0, 8) || 'N/A'}`,
          elapsed_ms: response.elapsed_ms,
        });
        onOrderPlaced?.();
      } else {
        setResult({
          success: false,
          message: response.error || 'Order failed',
        });
      }
    } catch (e) {
      setResult({
        success: false,
        message: e instanceof Error ? e.message : 'Order failed',
      });
    } finally {
      setLoading(null);
    }
  };

  // Place both arbitrage orders.
  const handleArbitrageOrder = async () => {
    if (!market.has_opportunity || !market.arbitrage_type) {
      alert('No current arbitrage opportunity');
      return;
    }

    setLoading('arbitrage');
    setResult(null);

    try {
      // Determine the arbitrage strategy.
      const isKalshiYes = market.arbitrage_type.includes('KalshiYes');
      const kalshiSide: Side = isKalshiYes ? 'yes' : 'no';
      const polySide: 'buy' | 'sell' = isKalshiYes ? 'sell' : 'buy';

      // Calculate bet amounts from the contract count and prices.
      const kalshiPrice = isKalshiYes ? market.kalshi_yes_price : market.kalshi_no_price;
      const kalshiBet = count * kalshiPrice; // USD
      const polyAmount = amount; // USDC

      const response = await executeArbitrage(apiBaseUrl, {
        kalshi_ticker: market.kalshi_market_id,
        kalshi_side: kalshiSide,
        kalshi_bet: kalshiBet,
        kalshi_price: kalshiPrice,
        poly_token_id: market.polymarket_market_id,
        poly_side: polySide,
        poly_amount: polyAmount,
      });

      if (response.success) {
        setResult({
          success: true,
          message: `Arbitrage succeeded! Kalshi: ${response.kalshi?.success ? '✓' : '✗'}, Poly: ${response.polymarket?.success ? '✓' : '✗'}`,
          elapsed_ms: (response.kalshi?.elapsed_ms || 0) + (response.polymarket?.elapsed_ms || 0),
        });
        onOrderPlaced?.();
      } else {
        const errors = [];
        if (!response.kalshi?.success) errors.push(`Kalshi: ${response.kalshi?.error || 'Failed'}`);
        if (!response.polymarket?.success) errors.push(`Poly: ${response.polymarket?.error || 'Failed'}`);
        setResult({
          success: false,
          message: errors.join('; ') || response.error || 'Arbitrage failed',
        });
      }
    } catch (e) {
      setResult({
        success: false,
        message: e instanceof Error ? e.message : 'Order failed',
      });
    } finally {
      setLoading(null);
    }
  };

  return (
    <div className="space-y-3">
      {/* Size settings */}
      <div className="grid grid-cols-2 gap-2">
        {/* Kalshi contract count */}
        <div className="bg-[--bg-tertiary] rounded p-2">
          <div className="text-[10px] text-[--text-muted] mb-1">Kalshi Contracts</div>
          <div className="flex items-center gap-1">
            <button
              className="w-6 h-6 rounded bg-[--bg-secondary] text-[--text-secondary] hover:bg-[--bg-primary] disabled:opacity-50 text-xs"
              onClick={() => setCount(Math.max(1, count - 1))}
              disabled={count <= 1}
            >
              -
            </button>
            <input
              type="number"
              min={1}
              value={count}
              onChange={(e) => setCount(Math.max(1, parseInt(e.target.value) || 1))}
              className="flex-1 h-6 px-1 text-center text-xs bg-[--bg-secondary] border border-[--border-color] rounded text-[--text-primary]"
            />
            <button
              className="w-6 h-6 rounded bg-[--bg-secondary] text-[--text-secondary] hover:bg-[--bg-primary] text-xs"
              onClick={() => setCount(count + 1)}
            >
              +
            </button>
          </div>
        </div>

        {/* Polymarket USDC amount */}
        <div className="bg-[--bg-tertiary] rounded p-2">
          <div className="text-[10px] text-[--text-muted] mb-1">Poly USDC</div>
          <div className="flex items-center gap-1">
            <button
              className="w-6 h-6 rounded bg-[--bg-secondary] text-[--text-secondary] hover:bg-[--bg-primary] disabled:opacity-50 text-xs"
              onClick={() => setAmount(Math.max(1, amount - 5))}
              disabled={amount <= 1}
            >
              -
            </button>
            <input
              type="number"
              min={1}
              value={amount}
              onChange={(e) => setAmount(Math.max(1, parseFloat(e.target.value) || 1))}
              className="flex-1 h-6 px-1 text-center text-xs bg-[--bg-secondary] border border-[--border-color] rounded text-[--text-primary]"
            />
            <button
              className="w-6 h-6 rounded bg-[--bg-secondary] text-[--text-secondary] hover:bg-[--bg-primary] text-xs"
              onClick={() => setAmount(amount + 5)}
            >
              +
            </button>
          </div>
        </div>
      </div>

      {/* Kalshi order */}
      <div className="bg-[--bg-tertiary] rounded p-2">
        <div className="flex items-center justify-between mb-2">
          <span className="text-[10px] text-blue-400 font-medium">Kalshi</span>
          <span className="text-[10px] text-[--text-muted] truncate max-w-[120px]">{market.kalshi_market_id}</span>
        </div>
        <div className="grid grid-cols-2 gap-1.5 mb-1.5">
          <button
            className="py-1.5 text-[10px] rounded bg-green-500/20 text-green-400 hover:bg-green-500/30 disabled:opacity-50"
            onClick={() => handleKalshiOrder('yes', 'buy')}
            disabled={loading !== null}
          >
            {loading === 'kalshi_yes_buy' ? '...' : `Buy YES ${(market.kalshi_yes_price * 100).toFixed(0)}¢`}
          </button>
          <button
            className="py-1.5 text-[10px] rounded bg-red-500/20 text-red-400 hover:bg-red-500/30 disabled:opacity-50"
            onClick={() => handleKalshiOrder('no', 'buy')}
            disabled={loading !== null}
          >
            {loading === 'kalshi_no_buy' ? '...' : `Buy NO ${(market.kalshi_no_price * 100).toFixed(0)}¢`}
          </button>
        </div>
        <div className="grid grid-cols-2 gap-1.5">
          <button
            className="py-1 text-[9px] rounded bg-green-500/10 text-green-400/70 hover:bg-green-500/20 disabled:opacity-50"
            onClick={() => handleKalshiOrder('yes', 'sell')}
            disabled={loading !== null}
          >
            {loading === 'kalshi_yes_sell' ? '...' : 'Sell YES'}
          </button>
          <button
            className="py-1 text-[9px] rounded bg-red-500/10 text-red-400/70 hover:bg-red-500/20 disabled:opacity-50"
            onClick={() => handleKalshiOrder('no', 'sell')}
            disabled={loading !== null}
          >
            {loading === 'kalshi_no_sell' ? '...' : 'Sell NO'}
          </button>
        </div>
      </div>

      {/* Polymarket order */}
      <div className="bg-[--bg-tertiary] rounded p-2">
        <div className="flex items-center justify-between mb-2">
          <span className="text-[10px] text-purple-400 font-medium">Polymarket</span>
          <span className="text-[10px] text-[--text-muted] truncate max-w-[120px]">{market.polymarket_market_id.slice(0, 16)}...</span>
        </div>
        <div className="grid grid-cols-2 gap-1.5 mb-1.5">
          <button
            className="py-1.5 text-[10px] rounded bg-green-500/20 text-green-400 hover:bg-green-500/30 disabled:opacity-50"
            onClick={() => handlePolyOrder('buy', 0)}
            disabled={loading !== null}
          >
            {loading === 'poly_buy_0' ? '...' : `Buy ${(market.poly_yes_price * 100).toFixed(0)}¢`}
          </button>
          <button
            className="py-1.5 text-[10px] rounded bg-red-500/20 text-red-400 hover:bg-red-500/30 disabled:opacity-50"
            onClick={() => handlePolyOrder('buy', 1)}
            disabled={loading !== null}
          >
            {loading === 'poly_buy_1' ? '...' : `Buy ${(market.poly_no_price * 100).toFixed(0)}¢`}
          </button>
        </div>
        <div className="grid grid-cols-2 gap-1.5">
          <button
            className="py-1 text-[9px] rounded bg-green-500/10 text-green-400/70 hover:bg-green-500/20 disabled:opacity-50"
            onClick={() => handlePolyOrder('sell', 0)}
            disabled={loading !== null}
          >
            {loading === 'poly_sell_0' ? '...' : 'Sell'}
          </button>
          <button
            className="py-1 text-[9px] rounded bg-red-500/10 text-red-400/70 hover:bg-red-500/20 disabled:opacity-50"
            onClick={() => handlePolyOrder('sell', 1)}
            disabled={loading !== null}
          >
            {loading === 'poly_sell_1' ? '...' : 'Sell'}
          </button>
        </div>
      </div>

      {/* One-click arbitrage order */}
      {market.has_opportunity && (
        <button
          className="w-full py-2 text-xs font-medium rounded bg-gradient-to-r from-green-500 to-emerald-500 text-white hover:from-green-600 hover:to-emerald-600 disabled:opacity-50"
          onClick={handleArbitrageOrder}
          disabled={loading !== null}
        >
          {loading === 'arbitrage' ? 'Executing...' : `🚀 Execute Arbitrage (${market.profit_margin.toFixed(2)}%)`}
        </button>
      )}

      {/* Result message */}
      {result && (
        <div className={`p-2 rounded text-xs ${
          result.success 
            ? 'bg-green-500/20 text-green-400' 
            : 'bg-red-500/20 text-red-400'
        }`}>
          <div>{result.message}</div>
          {result.elapsed_ms && (
            <div className="text-[10px] opacity-70 mt-0.5">
              Elapsed: {result.elapsed_ms.toFixed(0)}ms
            </div>
          )}
        </div>
      )}
    </div>
  );
}
