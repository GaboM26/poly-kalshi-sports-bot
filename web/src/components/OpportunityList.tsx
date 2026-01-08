import { useEffect, useRef, useState } from 'react';
import { MatchedMarketData, ArbitrageExecuteRequest } from '../types';
import { executeArbitrage } from '../utils/api';

interface OpportunityListProps {
  matchedMarkets: MatchedMarketData[];
  onSelectMarket?: (market: MatchedMarketData) => void;
  apiBaseUrl?: string;
}

type SortOption = 'profit' | 'event' | 'team';

export function OpportunityList({ matchedMarkets, onSelectMarket, apiBaseUrl = '' }: OpportunityListProps) {
  // 追踪价格变化用于高亮动画
  const [flashingCells, setFlashingCells] = useState<Set<string>>(new Set());
  const prevPricesRef = useRef<Map<string, { k_yes: number; k_no: number; p_yes: number; p_no: number }>>(new Map());
  
  // 排序选项
  const [sortBy, setSortBy] = useState<SortOption>('profit');
  
  // 执行状态
  const [executingKey, setExecutingKey] = useState<string | null>(null);
  const [lastResult, setLastResult] = useState<{ key: string; success: boolean; message: string } | null>(null);
  
  // 执行套利
  const handleExecute = async (market: MatchedMarketData, e: React.MouseEvent) => {
    e.stopPropagation();
    if (!market.has_opportunity || !apiBaseUrl) return;
    
    const key = `${market.event_name}_${market.team_name}`;
    setExecutingKey(key);
    setLastResult(null);
    
    try {
      // 解析策略类型
      // K↑ P↓ = Kalshi Yes + Polymarket No
      // K↓ P↑ = Kalshi No + Polymarket Yes
      const isKalshiYes = market.arbitrage_type?.includes('KalshiYes');
      
      // 计算下注金额 (假设总投资 $10 用于测试)
      const totalBet = 10;
      const impliedSum = (isKalshiYes ? market.kalshi_yes_price : market.kalshi_no_price) +
                         (isKalshiYes ? market.poly_no_price : market.poly_yes_price);
      const guaranteedReturn = totalBet / impliedSum;
      
      const kalshiPrice = isKalshiYes ? market.kalshi_yes_price : market.kalshi_no_price;
      const polyPrice = isKalshiYes ? market.poly_no_price : market.poly_yes_price;
      
      const kalshiBet = guaranteedReturn * kalshiPrice;
      const polyBet = guaranteedReturn * polyPrice;
      
      // 构建请求 - 需要获取正确的 token_id
      // 注意：polymarket_market_id 是 condition_id，需要找到对应的 token_id
      // 这里简化处理，使用 market_id 作为 token_id（实际可能需要从后端获取）
      const request: ArbitrageExecuteRequest = {
        kalshi_ticker: market.kalshi_market_id,
        kalshi_side: isKalshiYes ? 'yes' : 'no',
        kalshi_bet: kalshiBet,
        kalshi_price: kalshiPrice,
        poly_token_id: market.polymarket_market_id, // TODO: 需要正确的 token_id
        poly_side: 'buy',
        poly_amount: polyBet
      };
      
      const result = await executeArbitrage(apiBaseUrl, request);
      
      if (result.success) {
        setLastResult({ key, success: true, message: '套利执行成功！' });
      } else {
        const errors = [];
        if (!result.kalshi.success) errors.push(`K: ${result.kalshi.error}`);
        if (!result.polymarket.success) errors.push(`P: ${result.polymarket.error}`);
        setLastResult({ key, success: false, message: errors.join('; ') });
      }
    } catch (err) {
      setLastResult({ key, success: false, message: err instanceof Error ? err.message : '执行失败' });
    } finally {
      setExecutingKey(null);
    }
  };

  useEffect(() => {
    const newFlashing = new Set<string>();
    
    matchedMarkets.forEach((m) => {
      const key = `${m.event_name}_${m.team_name}`;
      const prev = prevPricesRef.current.get(key);
      
      if (prev) {
        if (prev.k_yes !== m.kalshi_yes_price || prev.k_no !== m.kalshi_no_price) {
          newFlashing.add(`${key}_kalshi`);
        }
        if (prev.p_yes !== m.poly_yes_price || prev.p_no !== m.poly_no_price) {
          newFlashing.add(`${key}_poly`);
        }
      }
      
      // 更新缓存
      prevPricesRef.current.set(key, {
        k_yes: m.kalshi_yes_price,
        k_no: m.kalshi_no_price,
        p_yes: m.poly_yes_price,
        p_no: m.poly_no_price
      });
    });
    
    if (newFlashing.size > 0) {
      setFlashingCells(newFlashing);
      // 清除闪烁状态
      const timer = setTimeout(() => setFlashingCells(new Set()), 500);
      return () => clearTimeout(timer);
    }
  }, [matchedMarkets]);

  if (matchedMarkets.length === 0) {
    return (
      <div className="p-4 text-center h-full flex flex-col items-center justify-center">
        <div className="text-2xl mb-2">⏳</div>
        <div className="text-[--text-secondary] text-sm">Scanning markets...</div>
        <div className="text-[--text-muted] text-[10px] mt-1">Real-time opportunities will appear here</div>
      </div>
    );
  }

  // 统计有套利机会的数量
  const oppCount = matchedMarkets.filter(m => m.has_opportunity).length;

  // 排序市场
  const sortedMarkets = [...matchedMarkets].sort((a, b) => {
    switch (sortBy) {
      case 'profit':
        // 按利润率降序（有套利机会的排在前面）
        if (a.has_opportunity && !b.has_opportunity) return -1;
        if (!a.has_opportunity && b.has_opportunity) return 1;
        if (a.has_opportunity && b.has_opportunity) {
          return b.profit_margin - a.profit_margin;
        }
        return a.event_name.localeCompare(b.event_name);
      
      case 'event':
        // 按事件名称字母序
        return a.event_name.localeCompare(b.event_name);
      
      case 'team':
        // 按队伍名称字母序
        return (a.team_name || '').localeCompare(b.team_name || '');
      
      default:
        return 0;
    }
  });

  return (
    <div className="overflow-hidden flex flex-col h-full text-xs">
      {/* 排序选择器 */}
      <div className="px-2 py-1 bg-[--bg-tertiary] border-b border-[--border-color] flex items-center justify-between flex-shrink-0">
        <span className="text-[10px] text-[--text-muted]">排序:</span>
        <div className="flex gap-1">
          <button
            onClick={() => setSortBy('profit')}
            className={`px-2 py-0.5 rounded text-[10px] font-medium transition-colors ${
              sortBy === 'profit'
                ? 'bg-[--accent-green] text-white'
                : 'bg-[--bg-secondary] text-[--text-secondary] hover:bg-[--bg-primary]'
            }`}
          >
            💰 收益
          </button>
          <button
            onClick={() => setSortBy('event')}
            className={`px-2 py-0.5 rounded text-[10px] font-medium transition-colors ${
              sortBy === 'event'
                ? 'bg-[--accent-purple] text-white'
                : 'bg-[--bg-secondary] text-[--text-secondary] hover:bg-[--bg-primary]'
            }`}
          >
            📅 事件
          </button>
          <button
            onClick={() => setSortBy('team')}
            className={`px-2 py-0.5 rounded text-[10px] font-medium transition-colors ${
              sortBy === 'team'
                ? 'bg-[--accent-yellow] text-white'
                : 'bg-[--bg-secondary] text-[--text-secondary] hover:bg-[--bg-primary]'
            }`}
          >
            🏀 队伍
          </button>
        </div>
      </div>

      {/* 表格容器 - 可滚动 */}
      <div className="flex-1 overflow-y-auto overflow-x-hidden">
        <table className="w-full">
          <thead className="sticky top-0 bg-[--bg-secondary] z-10 text-[10px]">
            <tr>
              <th className="py-1 px-2 font-medium text-[--text-secondary]">Event</th>
              <th className="py-1 px-2 text-center font-medium text-[--text-secondary]">Team</th>
              <th className="py-1 px-2 text-center font-medium text-[--text-secondary]">Kalshi</th>
              <th className="py-1 px-2 text-center font-medium text-[--text-secondary]">Polymarket</th>
              <th className="py-1 px-2 text-center font-medium text-[--text-secondary]">Str.</th>
              <th className="py-1 px-2 text-right font-medium text-[--text-secondary]">Profit</th>
              <th className="py-1 px-2 text-center w-12 font-medium text-[--text-secondary]">Act</th>
            </tr>
          </thead>
          <tbody>
            {sortedMarkets.map((market) => {
              const key = `${market.event_name}_${market.team_name}`;
              const kalshiFlashing = flashingCells.has(`${key}_kalshi`);
              const polyFlashing = flashingCells.has(`${key}_poly`);
              
              return (
                <tr
                  key={key}
                  onClick={() => onSelectMarket?.(market)}
                  className={`cursor-pointer border-b border-[--border-color] hover:bg-[--bg-secondary] transition-colors ${market.has_opportunity ? 'bg-[rgba(16,185,129,0.05)]' : ''}`}
                >
                  {/* Event */}
                  <td className="py-1 px-2">
                    <span className="text-[--text-primary] font-medium truncate max-w-[100px] block" title={market.event_name}>
                      {market.event_name}
                    </span>
                  </td>

                  {/* Team */}
                  <td className="text-center py-1 px-2">
                    <span className="px-1.5 py-0.5 rounded bg-[--bg-tertiary] text-[--accent-yellow] text-[10px] font-medium truncate max-w-[60px] inline-block" title={market.team_name}>
                      {market.team_name || '-'}
                    </span>
                  </td>

                  {/* Kalshi Prices */}
                  <td className="text-center py-1 px-2">
                    <span className={`price-tag price-kalshi tabular-nums text-[10px] ${kalshiFlashing ? 'flash-update' : ''} ${!market.kalshi_ready ? 'opacity-50' : ''}`}>
                      <span className="text-green-400">{formatCents(market.kalshi_yes_price)}</span>
                      <span className="text-[--text-muted] mx-0.5">/</span>
                      <span className="text-red-400">{formatCents(market.kalshi_no_price)}</span>
                    </span>
                  </td>

                  {/* Polymarket Prices */}
                  <td className="text-center py-1 px-2">
                    <span className={`price-tag price-poly tabular-nums text-[10px] ${polyFlashing ? 'flash-update' : ''} ${!market.poly_ready ? 'opacity-50' : ''}`}>
                      <span className="text-green-400">{formatCents(market.poly_yes_price)}</span>
                      <span className="text-[--text-muted] mx-0.5">/</span>
                      <span className="text-red-400">{formatCents(market.poly_no_price)}</span>
                    </span>
                  </td>

                  {/* Strategy */}
                  <td className="text-center py-1 px-2">
                    {market.has_opportunity ? (
                      <span className="text-[--text-secondary] text-[10px]">
                        {getStrategyShort(market.arbitrage_type || '')}
                      </span>
                    ) : (
                      <span className="text-[--text-muted] text-[10px]">-</span>
                    )}
                  </td>

                  {/* Profit (净利润) */}
                  <td className="text-right py-1 px-2">
                    {market.has_opportunity ? (
                      <div className={getProfitClass(market.profit_margin)}>
                        <span className="text-sm font-bold tabular-nums">{market.profit_margin.toFixed(2)}%</span>
                        <div className="text-[9px] opacity-70 leading-none" title="净利润">${market.expected_profit.toFixed(2)}</div>
                      </div>
                    ) : (
                      <span className="text-[--text-muted] text-[10px]">-</span>
                    )}
                  </td>
                  
                  {/* Action */}
                  <td className="text-center py-1 px-2">
                    {market.has_opportunity ? (
                      <div className="flex flex-col items-center gap-0.5">
                        <button
                          onClick={(e) => handleExecute(market, e)}
                          disabled={executingKey === key || !apiBaseUrl}
                          className={`px-1.5 py-0.5 text-[9px] font-medium rounded transition-colors ${
                            executingKey === key
                              ? 'bg-gray-500/30 text-gray-400 cursor-wait'
                              : 'bg-[--accent-green]/20 text-[--accent-green] hover:bg-[--accent-green]/30'
                          }`}
                        >
                          {executingKey === key ? '...' : '执行'}
                        </button>
                        {lastResult?.key === key && (
                          <span className={`text-[9px] leading-none ${lastResult.success ? 'text-green-400' : 'text-red-400'}`}>
                            {lastResult.success ? '✓' : '✗'}
                          </span>
                        )}
                      </div>
                    ) : (
                      <span className="text-[--text-muted] text-[10px]">-</span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      
      {/* 底部统计 */}
      <div className="px-2 py-1 bg-[--bg-tertiary] border-t border-[--border-color] flex justify-between items-center text-[10px] text-[--text-muted] flex-shrink-0">
        <span>Total: {matchedMarkets.length}</span>
        <span className={oppCount > 0 ? 'text-[--accent-green]' : ''}>
          {oppCount > 0 ? `🔥 ${oppCount} Opps` : 'No Opps'}
        </span>
      </div>
    </div>
  );
}

function formatCents(price: number): string {
  return (price * 100).toFixed(0) + '¢';
}

function getProfitClass(margin: number): string {
  if (margin >= 5) return 'profit-high';
  if (margin >= 2) return 'profit-medium';
  return 'profit-low';
}

function getStrategyShort(type: string): string {
  if (type.includes('KalshiYes') && type.includes('PolymarketNo')) return 'K↑ P↓';
  if (type.includes('KalshiNo') && type.includes('PolymarketYes')) return 'K↓ P↑';
  // Try to shorten more if needed
  return type.replace('Kalshi', 'K').replace('Polymarket', 'P').replace('Yes', '↑').replace('No', '↓');
}
