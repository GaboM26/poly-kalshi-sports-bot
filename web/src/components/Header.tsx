import { useEffect, useState } from 'react';
import { DataCoverage, AccountBalance, MetricsReport } from '../types';

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
  metrics: MetricsReport | null;
  apiBaseUrl: string;
  onLogout?: () => void;
  username?: string | null;
}

export function Header({ isConnected, stats, totalProfit, lastUpdateTime, updateCount, dataCoverage, metrics, apiBaseUrl, onLogout, username }: HeaderProps) {
  const [isFlashing, setIsFlashing] = useState(false);
  const [accountBalance, setAccountBalance] = useState<AccountBalance | null>(null);
  
  // 当收到新数据时闪烁动画
  useEffect(() => {
    if (updateCount > 0) {
      setIsFlashing(true);
      const timer = setTimeout(() => setIsFlashing(false), 300);
      return () => clearTimeout(timer);
    }
  }, [updateCount]);

  // 定期获取账户余额
  useEffect(() => {
    const fetchBalance = async () => {
      try {
        const response = await fetch(`${apiBaseUrl}/api/account-balance`);
        if (response.ok) {
          const data = await response.json();
          setAccountBalance(data);
        }
      } catch (error) {
        // 静默失败
      }
    };

    // 初始获取
    fetchBalance();
    // 每 10 秒更新一次
    const interval = setInterval(fetchBalance, 10000);
    return () => clearInterval(interval);
  }, [apiBaseUrl]);

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

  // 格式化余额显示
  const formatBalance = (balance?: number) => {
    if (balance === undefined || balance === null) return '--';
    return balance >= 1000 ? `${(balance / 1000).toFixed(1)}k` : balance.toFixed(0);
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
          {/* 账户余额 */}
          {accountBalance && (
            <div className="flex items-center gap-2 px-3 py-1 rounded bg-[--bg-tertiary]" title={`账户余额\nKalshi: ${accountBalance.kalshi.available ? `$${formatBalance(accountBalance.kalshi.balance)}` : '未配置或获取失败'}\nPolymarket: ${accountBalance.polymarket.available ? `$${formatBalance(accountBalance.polymarket.balance)}` : '未配置或获取失败'}`}>
              <span className="text-xs text-[--text-muted]">💰</span>
              <span className={`font-mono text-xs ${accountBalance.kalshi.available ? 'text-blue-400' : 'text-gray-500'}`} title={accountBalance.kalshi.error || ''}>
                K:${accountBalance.kalshi.available ? formatBalance(accountBalance.kalshi.balance) : '--'}
              </span>
              <span className={`font-mono text-xs ${accountBalance.polymarket.available ? 'text-purple-400' : 'text-gray-500'}`} title={accountBalance.polymarket.error || 'Polymarket 余额'}>
                P:${accountBalance.polymarket.available ? formatBalance(accountBalance.polymarket.balance) : '--'}
              </span>
            </div>
          )}

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
            <span className="text-xs text-[--text-muted]">⚡ WS:</span>
            <span className={`font-mono text-xs ${getLatencyColor(dataCoverage.kalshi_latency_ms)}`}>
              K:{formatLatency(dataCoverage.kalshi_latency_ms)}
            </span>
            <span className={`font-mono text-xs ${getLatencyColor(dataCoverage.polymarket_latency_ms)}`}>
              P:{formatLatency(dataCoverage.polymarket_latency_ms)}
            </span>
          </div>

          {/* API 延迟 (下单接口速度) */}
          <div className="flex items-center gap-2 px-3 py-1 rounded bg-[--bg-tertiary]" title="API延迟：下单接口的响应时间（通过定期ping测试）">
            <span className="text-xs text-[--text-muted]">🔗 API:</span>
            <span className={`font-mono text-xs ${getLatencyColor(metrics?.api_latency.kalshi_ms)}`}>
              K:{formatLatency(metrics?.api_latency.kalshi_ms)}
            </span>
            <span className={`font-mono text-xs ${getLatencyColor(metrics?.api_latency.polymarket_ms)}`}>
              P:{formatLatency(metrics?.api_latency.polymarket_ms)}
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

          {/* 用户信息和退出 */}
          {username && onLogout && (
            <div className="flex items-center gap-2 pl-3 border-l border-[--border-color]">
              <span className="text-xs text-[--text-muted]">👤 {username}</span>
              <button
                onClick={onLogout}
                className="px-2 py-1 text-xs text-[--text-muted] hover:text-red-400 hover:bg-red-500/10 rounded transition-colors"
                title="退出登录"
              >
                退出
              </button>
            </div>
          )}
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
