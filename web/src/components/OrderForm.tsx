import { useState } from 'react';
import { MatchedMarketData } from '../types';
import { createKalshiOrder, createPolymarketOrder, executeArbitrage } from '../utils/api';

interface OrderFormProps {
  market: MatchedMarketData;
  apiBaseUrl: string;
  onOrderPlaced?: () => void;
}

type Platform = 'kalshi' | 'polymarket';
type Side = 'yes' | 'no';
type Action = 'buy' | 'sell';

export function OrderForm({ market, apiBaseUrl, onOrderPlaced }: OrderFormProps) {
  const [count, setCount] = useState(1);
  const [amount, setAmount] = useState(10); // Polymarket USDC 金额
  const [loading, setLoading] = useState<string | null>(null);
  const [result, setResult] = useState<{ success: boolean; message: string; elapsed_ms?: number } | null>(null);

  // Kalshi 下单
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
          message: `Kalshi ${action} ${side.toUpperCase()} 成功! 成交 ${response.order?.fill_count || 0} 个`,
          elapsed_ms: response.elapsed_ms,
        });
        onOrderPlaced?.();
      } else {
        setResult({
          success: false,
          message: response.error || '下单失败',
        });
      }
    } catch (e) {
      setResult({
        success: false,
        message: e instanceof Error ? e.message : '下单失败',
      });
    } finally {
      setLoading(null);
    }
  };

  // Polymarket 下单
  const handlePolyOrder = async (side: 'buy' | 'sell', teamIndex: 0 | 1) => {
    // 根据市场数据获取对应的 token_id
    // market 中有 polymarket_market_id 作为 conditionId
    // 需要从匹配数据中获取具体的 token_id
    // 暂时使用 market_id 作为 token（实际应该是 token_id_a 或 token_id_b）
    
    const loadingKey = `poly_${side}_${teamIndex}`;
    setLoading(loadingKey);
    setResult(null);

    try {
      // 注意：这里需要实际的 token_id
      // 由于前端匹配数据可能没有 token_id，暂时提示
      // 在真实场景中，应该从后端获取完整的市场数据包含 token_id
      const tokenId = market.polymarket_market_id; // 这应该是具体的 token_id
      
      if (!tokenId) {
        setResult({
          success: false,
          message: 'Token ID 未找到',
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
          message: `Polymarket ${side.toUpperCase()} 成功! 订单ID: ${response.order_id?.slice(0, 8) || 'N/A'}`,
          elapsed_ms: response.elapsed_ms,
        });
        onOrderPlaced?.();
      } else {
        setResult({
          success: false,
          message: response.error || '下单失败',
        });
      }
    } catch (e) {
      setResult({
        success: false,
        message: e instanceof Error ? e.message : '下单失败',
      });
    } finally {
      setLoading(null);
    }
  };

  // 套利一键下单
  const handleArbitrageOrder = async () => {
    if (!market.has_opportunity || !market.arbitrage_type) {
      alert('当前无套利机会');
      return;
    }

    setLoading('arbitrage');
    setResult(null);

    try {
      // 解析套利策略
      const isKalshiYes = market.arbitrage_type.includes('KalshiYes');
      const kalshiSide: Side = isKalshiYes ? 'yes' : 'no';
      const polySide: 'buy' | 'sell' = isKalshiYes ? 'sell' : 'buy';

      // 计算下注金额（基于合约数量和价格）
      const kalshiPrice = isKalshiYes ? market.kalshi_yes_price : market.kalshi_no_price;
      const kalshiBet = count * kalshiPrice; // 美元
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
          message: `套利成功! Kalshi: ${response.kalshi?.success ? '✓' : '✗'}, Poly: ${response.polymarket?.success ? '✓' : '✗'}`,
          elapsed_ms: (response.kalshi?.elapsed_ms || 0) + (response.polymarket?.elapsed_ms || 0),
        });
        onOrderPlaced?.();
      } else {
        const errors = [];
        if (!response.kalshi?.success) errors.push(`Kalshi: ${response.kalshi?.error || '失败'}`);
        if (!response.polymarket?.success) errors.push(`Poly: ${response.polymarket?.error || '失败'}`);
        setResult({
          success: false,
          message: errors.join('; ') || response.error || '套利失败',
        });
      }
    } catch (e) {
      setResult({
        success: false,
        message: e instanceof Error ? e.message : '下单失败',
      });
    } finally {
      setLoading(null);
    }
  };

  return (
    <div className="space-y-3">
      {/* 数量设置区 */}
      <div className="grid grid-cols-2 gap-2">
        {/* Kalshi 合约数量 */}
        <div className="bg-[--bg-tertiary] rounded p-2">
          <div className="text-[10px] text-[--text-muted] mb-1">Kalshi 合约数</div>
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

        {/* Polymarket USDC 金额 */}
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

      {/* Kalshi 下单 */}
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
            {loading === 'kalshi_yes_buy' ? '...' : `买YES ${(market.kalshi_yes_price * 100).toFixed(0)}¢`}
          </button>
          <button
            className="py-1.5 text-[10px] rounded bg-red-500/20 text-red-400 hover:bg-red-500/30 disabled:opacity-50"
            onClick={() => handleKalshiOrder('no', 'buy')}
            disabled={loading !== null}
          >
            {loading === 'kalshi_no_buy' ? '...' : `买NO ${(market.kalshi_no_price * 100).toFixed(0)}¢`}
          </button>
        </div>
        <div className="grid grid-cols-2 gap-1.5">
          <button
            className="py-1 text-[9px] rounded bg-green-500/10 text-green-400/70 hover:bg-green-500/20 disabled:opacity-50"
            onClick={() => handleKalshiOrder('yes', 'sell')}
            disabled={loading !== null}
          >
            {loading === 'kalshi_yes_sell' ? '...' : '卖YES'}
          </button>
          <button
            className="py-1 text-[9px] rounded bg-red-500/10 text-red-400/70 hover:bg-red-500/20 disabled:opacity-50"
            onClick={() => handleKalshiOrder('no', 'sell')}
            disabled={loading !== null}
          >
            {loading === 'kalshi_no_sell' ? '...' : '卖NO'}
          </button>
        </div>
      </div>

      {/* Polymarket 下单 */}
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
            {loading === 'poly_buy_0' ? '...' : `买入 ${(market.poly_yes_price * 100).toFixed(0)}¢`}
          </button>
          <button
            className="py-1.5 text-[10px] rounded bg-red-500/20 text-red-400 hover:bg-red-500/30 disabled:opacity-50"
            onClick={() => handlePolyOrder('buy', 1)}
            disabled={loading !== null}
          >
            {loading === 'poly_buy_1' ? '...' : `买入 ${(market.poly_no_price * 100).toFixed(0)}¢`}
          </button>
        </div>
        <div className="grid grid-cols-2 gap-1.5">
          <button
            className="py-1 text-[9px] rounded bg-green-500/10 text-green-400/70 hover:bg-green-500/20 disabled:opacity-50"
            onClick={() => handlePolyOrder('sell', 0)}
            disabled={loading !== null}
          >
            {loading === 'poly_sell_0' ? '...' : '卖出'}
          </button>
          <button
            className="py-1 text-[9px] rounded bg-red-500/10 text-red-400/70 hover:bg-red-500/20 disabled:opacity-50"
            onClick={() => handlePolyOrder('sell', 1)}
            disabled={loading !== null}
          >
            {loading === 'poly_sell_1' ? '...' : '卖出'}
          </button>
        </div>
      </div>

      {/* 套利一键下单 */}
      {market.has_opportunity && (
        <button
          className="w-full py-2 text-xs font-medium rounded bg-gradient-to-r from-green-500 to-emerald-500 text-white hover:from-green-600 hover:to-emerald-600 disabled:opacity-50"
          onClick={handleArbitrageOrder}
          disabled={loading !== null}
        >
          {loading === 'arbitrage' ? '执行中...' : `🚀 一键套利 (${market.profit_margin.toFixed(2)}%)`}
        </button>
      )}

      {/* 结果提示 */}
      {result && (
        <div className={`p-2 rounded text-xs ${
          result.success 
            ? 'bg-green-500/20 text-green-400' 
            : 'bg-red-500/20 text-red-400'
        }`}>
          <div>{result.message}</div>
          {result.elapsed_ms && (
            <div className="text-[10px] opacity-70 mt-0.5">
              耗时: {result.elapsed_ms.toFixed(0)}ms
            </div>
          )}
        </div>
      )}
    </div>
  );
}
