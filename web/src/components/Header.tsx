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

export function Header({ isConnected, stats, totalProfit, lastUpdateTime: _lastUpdateTime, updateCount, dataCoverage, metrics, apiBaseUrl, onLogout, username }: HeaderProps) {
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
      <div className="px-2 h-10 flex items-center justify-between gap-2 overflow-x-auto">
        {/* Logo - 紧凑版 */}
        <div className="flex items-center gap-1 flex-shrink-0">
          <span className="text-sm">📊</span>
          <span className="font-semibold text-[--text-primary] text-xs">Arbitrage Scanner</span>
        </div>

        {/* 统计信息 - 紧凑版 */}
        <div className="flex items-center gap-2 text-xs flex-shrink-0">
          {/* 账户余额 */}
          {accountBalance && (
            <div className="flex items-center gap-1 px-2 py-0.5 rounded bg-[--bg-tertiary]" title={`账户余额\nKalshi: ${accountBalance.kalshi.available ? `$${formatBalance(accountBalance.kalshi.balance)}` : '未配置或获取失败'}\nPolymarket: ${accountBalance.polymarket.available ? `$${formatBalance(accountBalance.polymarket.balance)}` : '未配置或获取失败'}`}>
              <span className="text-[10px] text-[--text-muted]">💰</span>
              <span className={`font-mono text-[10px] ${accountBalance.kalshi.available ? 'text-blue-400' : 'text-gray-500'}`} title={accountBalance.kalshi.error || ''}>
                K:${accountBalance.kalshi.available ? formatBalance(accountBalance.kalshi.balance) : '--'}
              </span>
              <span className={`font-mono text-[10px] ${accountBalance.polymarket.available ? 'text-purple-400' : 'text-gray-500'}`} title={accountBalance.polymarket.error || 'Polymarket 余额'}>
                P:${accountBalance.polymarket.available ? formatBalance(accountBalance.polymarket.balance) : '--'}
              </span>
            </div>
          )}

          {/* 数据覆盖率 */}
          <div className="flex items-center gap-1 px-2 py-0.5 rounded bg-[--bg-tertiary]" title="数据覆盖率：两个平台都有实时数据的市场对数量">
            <span className={`font-mono text-[10px] ${dataCoverage.kalshi_connected ? 'text-blue-400' : 'text-gray-500'}`}>
              K:{dataCoverage.kalshi_coverage}/{dataCoverage.polymarket_coverage}
            </span>
            <span className={`font-mono text-[10px] ${dataCoverage.polymarket_connected ? 'text-purple-400' : 'text-gray-500'}`}>
              P:{dataCoverage.polymarket_coverage}/{dataCoverage.total_markets}
            </span>
            <span className={`font-mono text-[10px] font-semibold ${coveragePercent >= 80 ? 'text-green-400' : coveragePercent >= 50 ? 'text-yellow-400' : 'text-red-400'}`}>
              ✓{dataCoverage.both_ready}/{dataCoverage.total_markets}
            </span>
          </div>

          {/* 平台延迟 */}
          <div className="flex items-center gap-1 px-2 py-0.5 rounded bg-[--bg-tertiary]" title="平台数据延迟">
            <span className="text-[10px] text-[--text-muted]">WS:</span>
            <span className={`font-mono text-[10px] ${getLatencyColor(dataCoverage.kalshi_latency_ms)}`}>
              K:{formatLatency(dataCoverage.kalshi_latency_ms)}
            </span>
            <span className={`font-mono text-[10px] ${getLatencyColor(dataCoverage.polymarket_latency_ms)}`}>
              P:{formatLatency(dataCoverage.polymarket_latency_ms)}
            </span>
          </div>

          {/* API 延迟 */}
          <div className="flex items-center gap-1 px-2 py-0.5 rounded bg-[--bg-tertiary]" title="API延迟">
            <span className="text-[10px] text-[--text-muted]">API:</span>
            <span className={`font-mono text-[10px] ${getLatencyColor(metrics?.api_latency.kalshi_ms)}`}>
              K:{formatLatency(metrics?.api_latency.kalshi_ms)}
            </span>
            <span className={`font-mono text-[10px] ${getLatencyColor(metrics?.api_latency.polymarket_ms)}`}>
              P:{formatLatency(metrics?.api_latency.polymarket_ms)}
            </span>
          </div>

          <div className="h-3 w-px bg-[--border-color]" />
          
          <StatItem 
            label="Opps" 
            value={stats.opportunitiesCount} 
            color="text-green-400" 
            highlight={isFlashing}
          />
          <StatItem label="Profit" value={`$${totalProfit.toFixed(0)}`} color="text-emerald-400" />
          
          <div className="h-3 w-px bg-[--border-color]" />
          
          {/* 连接状态 */}
          <div className="flex items-center gap-1">
            <span className={`status-dot ${isConnected ? 'status-connected animate-pulse-dot' : 'status-disconnected'}`} />
            <span className={`text-[10px] ${isConnected ? 'text-green-400' : 'text-red-400'}`}>
              {isConnected ? '● Live' : 'Offline'}
            </span>
          </div>

          {/* 用户信息 */}
          {username && (
            <div className="flex items-center gap-1 pl-2 border-l border-[--border-color]">
              <span className="text-[10px] text-[--text-secondary]">👤 {username}</span>
              {onLogout && (
                <button
                  onClick={onLogout}
                  className="text-[10px] text-[--text-muted] hover:text-red-400 transition-colors px-1"
                  title="退出登录"
                >
                  退出
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    </header>
  );
}

function StatItem({ label, value, color, highlight }: { label: string; value: string | number; color: string; highlight?: boolean }) {
  return (
    <div className="flex items-center gap-1">
      <span className="text-[--text-muted] text-[10px]">{label}</span>
      <span className={`${color} font-semibold tabular-nums text-[10px] transition-all duration-300 ${highlight ? 'scale-110 brightness-125' : ''}`}>
        {value}
      </span>
    </div>
  );
}
