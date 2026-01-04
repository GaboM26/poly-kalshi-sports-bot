import { ArbitrageOpportunity } from '../types';

interface OpportunityListProps {
  opportunities: ArbitrageOpportunity[];
  onSelectOpportunity?: (opportunity: ArbitrageOpportunity) => void;
}

export function OpportunityList({ opportunities, onSelectOpportunity }: OpportunityListProps) {
  if (opportunities.length === 0) {
    return (
      <div className="card p-8 text-center">
        <div className="text-4xl mb-3">⏳</div>
        <div className="text-[--text-secondary]">Scanning markets...</div>
        <div className="text-[--text-muted] text-xs mt-1">Real-time opportunities will appear here</div>
      </div>
    );
  }

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
            {opportunities.map((opp, index) => (
              <tr
                key={`${opp.kalshi_market.event_id}-${index}`}
                onClick={() => onSelectOpportunity?.(opp)}
                className="cursor-pointer"
              >
                {/* Event */}
                <td>
                  <span className="text-[--text-primary] font-medium">
                    {opp.kalshi_market.event_name}
                  </span>
                </td>

                {/* Team */}
                <td className="text-center">
                  <span className="px-2 py-0.5 rounded bg-[--bg-tertiary] text-[--accent-yellow] text-xs font-medium">
                    {opp.kalshi_market.team_name || '-'}
                  </span>
                </td>

                {/* Kalshi Prices */}
                <td className="text-center">
                  <span className="price-tag price-kalshi tabular-nums">
                    <span className="text-green-400">{formatCents(opp.kalshi_market.yes_price)}</span>
                    <span className="text-[--text-muted] mx-1">/</span>
                    <span className="text-red-400">{formatCents(opp.kalshi_market.no_price)}</span>
                  </span>
                </td>

                {/* Polymarket Prices */}
                <td className="text-center">
                  <span className="price-tag price-poly tabular-nums">
                    <span className="text-green-400">{formatCents(opp.polymarket_market.yes_price)}</span>
                    <span className="text-[--text-muted] mx-1">/</span>
                    <span className="text-red-400">{formatCents(opp.polymarket_market.no_price)}</span>
                  </span>
                </td>

                {/* Strategy */}
                <td className="text-center">
                  <span className="text-[--text-secondary] text-xs">
                    {getStrategyShort(opp.arbitrage_type)}
                  </span>
                </td>

                {/* Profit */}
                <td className="text-right">
                  <div className={getProfitClass(opp.profit_margin)}>
                    <span className="text-base font-bold tabular-nums">{opp.profit_margin.toFixed(2)}%</span>
                    <span className="text-xs ml-1 opacity-70">${opp.expected_profit.toFixed(0)}</span>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
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
