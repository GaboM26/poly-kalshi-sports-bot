import { useEffect, useState } from 'react';
import { DataCoverage } from '../types';

interface HeaderProps {
  isConnected: boolean;
  stats: {
    kalshiCount: number;
    polymarketCount: number;
    matchedCount: number;
    opportunitiesCount: number;
  };
  totalProfit: number;
  lastUpdateTime: Date | null;
  updateCount: number;
  dataCoverage: DataCoverage;
}

export function Header({ isConnected, stats, totalProfit, lastUpdateTime, updateCount, dataCoverage }: HeaderProps) {
  const [isFlashing, setIsFlashing] = useState(false);
  
  // 当收到新数据时闪烁动画
  useEffect(() => {
    if (updateCount > 0) {
      setIsFlashing(true);
      const timer = setTimeout(() => setIsFlashing(false), 300);
      return () => clearTimeout(timer);
    }
  }, [updateCount]);

  const formatLastUpdate = () => {
    if (!lastUpdateTime) return '--';
    const seconds = Math.floor((Date.now() - lastUpdateTime.getTime()) / 1000);
    if (seconds < 5) return 'Just now';
    if (seconds < 60) return `${seconds}s ago`;
    return `${Math.floor(seconds / 60)}m ago`;
  };

  // 每秒刷新显示时间
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  // 计算覆盖率百分比
  const getCoveragePercent = () => {
    if (dataCoverage.total_markets === 0) return 0;
    return Math.round((dataCoverage.both_ready / dataCoverage.total_markets) * 100);
  };

  const coveragePercent = getCoveragePercent();

  // 格式化延迟显示
  const formatLatency = (latencyMs?: number) => {
    if (latencyMs === undefined || latencyMs === null) return '--';
    if (latencyMs < 1000) return `${latencyMs}ms`;
    return `${(latencyMs / 1000).toFixed(1)}s`;
  };

  // 获取延迟颜色
  const getLatencyColor = (latencyMs?: number) => {
    if (latencyMs === undefined || latencyMs === null) return 'text-gray-500';
    if (latencyMs < 100) return 'text-green-400';
    if (latencyMs < 500) return 'text-yellow-400';
    if (latencyMs < 2000) return 'text-orange-400';
    return 'text-red-400';
  };

  return (
    <header className="border-b border-[--border-color] bg-[--bg-secondary]">
      <div className="max-w-7xl mx-auto px-4 h-12 flex items-center justify-between">
        {/* Logo */}
        <div className="flex items-center gap-2">
          <span className="text-lg">📊</span>
          <span className="font-semibold text-[--text-primary]">Arbitrage Scanner</span>
        </div>

        {/* 统计信息 */}
        <div className="flex items-center gap-4 text-sm">
          {/* 数据覆盖率 */}
          <div className="flex items-center gap-2 px-3 py-1 rounded bg-[--bg-tertiary]" title="数据覆盖率：两个平台都有实时数据的市场对数量">
            <span className="text-xs text-[--text-muted]">📡 数据:</span>
            <span className={`font-mono text-xs ${dataCoverage.kalshi_connected ? 'text-blue-400' : 'text-gray-500'}`}>
              K:{dataCoverage.kalshi_coverage}
            </span>
            <span className={`font-mono text-xs ${dataCoverage.polymarket_connected ? 'text-purple-400' : 'text-gray-500'}`}>
              P:{dataCoverage.polymarket_coverage}
            </span>
            <span className={`font-mono text-xs font-semibold ${coveragePercent >= 80 ? 'text-green-400' : coveragePercent >= 50 ? 'text-yellow-400' : 'text-red-400'}`}>
              ✓:{dataCoverage.full_coverage}
            </span>
          </div>

          {/* 平台延迟 */}
          <div className="flex items-center gap-2 px-3 py-1 rounded bg-[--bg-tertiary]" title="平台数据延迟：从交易所接收到数据到现在的时间">
            <span className="text-xs text-[--text-muted]">⚡ 延迟:</span>
            <span className={`font-mono text-xs ${getLatencyColor(dataCoverage.kalshi_latency_ms)}`}>
              K:{formatLatency(dataCoverage.kalshi_latency_ms)}
            </span>
            <span className={`font-mono text-xs ${getLatencyColor(dataCoverage.polymarket_latency_ms)}`}>
              P:{formatLatency(dataCoverage.polymarket_latency_ms)}
            </span>
          </div>

          <div className="h-4 w-px bg-[--border-color]" />
          
          <StatItem 
            label="Opps" 
            value={stats.opportunitiesCount} 
            color="text-green-400" 
            highlight={isFlashing}
          />
          <StatItem label="Profit" value={`$${totalProfit.toFixed(0)}`} color="text-emerald-400" />
          
          {/* 最后更新时间 */}
          <div className="flex items-center gap-1.5 pl-3 border-l border-[--border-color]">
            <span className={`text-xs transition-colors duration-300 ${isFlashing ? 'text-cyan-400' : 'text-[--text-muted]'}`}>
              ⏱ {formatLastUpdate()}
            </span>
          </div>
          
          {/* 连接状态 */}
          <div className="flex items-center gap-1.5 pl-3 border-l border-[--border-color]">
            <span className={`status-dot ${isConnected ? 'status-connected animate-pulse-dot' : 'status-disconnected'}`} />
            <span className={`text-xs ${isConnected ? 'text-green-400' : 'text-red-400'}`}>
              {isConnected ? 'Live' : 'Offline'}
            </span>
          </div>
        </div>
      </div>
    </header>
  );
}

function StatItem({ label, value, color, highlight }: { label: string; value: string | number; color: string; highlight?: boolean }) {
  return (
    <div className="flex items-center gap-1.5">
      <span className="text-[--text-muted] text-xs">{label}</span>
      <span className={`${color} font-semibold tabular-nums transition-all duration-300 ${highlight ? 'scale-110 brightness-125' : ''}`}>
        {value}
      </span>
    </div>
  );
}
