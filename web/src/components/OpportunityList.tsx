import { useEffect, useRef, useState } from 'react';
import { MatchedMarketData } from '../types';

interface OpportunityListProps {
  matchedMarkets: MatchedMarketData[];
  onSelectMarket?: (market: MatchedMarketData) => void;
}

export function OpportunityList({ matchedMarkets, onSelectMarket }: OpportunityListProps) {
  // 追踪价格变化用于高亮动画
  const [flashingCells, setFlashingCells] = useState<Set<string>>(new Set());
  const prevPricesRef = useRef<Map<string, { k_yes: number; k_no: number; p_yes: number; p_no: number }>>(new Map());

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
      <div className="card p-8 text-center">
        <div className="text-4xl mb-3">⏳</div>
        <div className="text-[--text-secondary]">Scanning markets...</div>
        <div className="text-[--text-muted] text-xs mt-1">Real-time opportunities will appear here</div>
      </div>
    );
  }

  // 统计有套利机会的数量
  const oppCount = matchedMarkets.filter(m => m.has_opportunity).length;

  return (
    <div className="card overflow-hidden">
      <div className="overflow-x-auto">
        <table>
          <thead>
            <tr>
              <th>Event</th>
              <th className="text-center">Team</th>
              <th className="text-center">Kalshi</th>
              <th className="text-center">Polymarket</th>
              <th className="text-center">Strategy</th>
              <th className="text-right">Profit</th>
            </tr>
          </thead>
          <tbody>
            {matchedMarkets.map((market) => {
              const key = `${market.event_name}_${market.team_name}`;
              const kalshiFlashing = flashingCells.has(`${key}_kalshi`);
              const polyFlashing = flashingCells.has(`${key}_poly`);
              
              return (
                <tr
                  key={key}
                  onClick={() => onSelectMarket?.(market)}
                  className={`cursor-pointer ${market.has_opportunity ? 'bg-[rgba(16,185,129,0.05)]' : ''}`}
                >
                  {/* Event */}
                  <td>
                    <span className="text-[--text-primary] font-medium">
                      {market.event_name}
                    </span>
                  </td>

                  {/* Team */}
                  <td className="text-center">
                    <span className="px-2 py-0.5 rounded bg-[--bg-tertiary] text-[--accent-yellow] text-xs font-medium">
                      {market.team_name || '-'}
                    </span>
                  </td>

                  {/* Kalshi Prices */}
                  <td className="text-center">
                    <span className={`price-tag price-kalshi tabular-nums ${kalshiFlashing ? 'flash-update' : ''} ${!market.kalshi_ready ? 'opacity-50' : ''}`}>
                      <span className="text-green-400">{formatCents(market.kalshi_yes_price)}</span>
                      <span className="text-[--text-muted] mx-1">/</span>
                      <span className="text-red-400">{formatCents(market.kalshi_no_price)}</span>
                    </span>
                  </td>

                  {/* Polymarket Prices */}
                  <td className="text-center">
                    <span className={`price-tag price-poly tabular-nums ${polyFlashing ? 'flash-update' : ''} ${!market.poly_ready ? 'opacity-50' : ''}`}>
                      <span className="text-green-400">{formatCents(market.poly_yes_price)}</span>
                      <span className="text-[--text-muted] mx-1">/</span>
                      <span className="text-red-400">{formatCents(market.poly_no_price)}</span>
                    </span>
                  </td>

                  {/* Strategy */}
                  <td className="text-center">
                    {market.has_opportunity ? (
                      <span className="text-[--text-secondary] text-xs">
                        {getStrategyShort(market.arbitrage_type || '')}
                      </span>
                    ) : (
                      <span className="text-[--text-muted] text-xs">-</span>
                    )}
                  </td>

                  {/* Profit */}
                  <td className="text-right">
                    {market.has_opportunity ? (
                      <div className={getProfitClass(market.profit_margin)}>
                        <span className="text-base font-bold tabular-nums">{market.profit_margin.toFixed(2)}%</span>
                        <span className="text-xs ml-1 opacity-70">${market.expected_profit.toFixed(0)}</span>
                      </div>
                    ) : (
                      <span className="text-[--text-muted] text-xs">-</span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      
      {/* 底部统计 */}
      <div className="px-4 py-2 bg-[--bg-tertiary] border-t border-[--border-color] flex justify-between items-center text-xs text-[--text-muted]">
        <span>Total: {matchedMarkets.length} matched markets</span>
        <span className={oppCount > 0 ? 'text-[--accent-green]' : ''}>
          {oppCount > 0 ? `🔥 ${oppCount} opportunities` : 'No opportunities'}
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
  return type;
}
