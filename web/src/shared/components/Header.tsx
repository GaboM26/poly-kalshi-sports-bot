import { ReactNode, useEffect, useState } from 'react';
import { apiUrl } from '../api';
import { DataCoverage, AccountBalance, MetricsReport } from '../../types';

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
  onLogout?: () => void;
  username?: string | null;
  /** Controls of the active strategy, shown before the user menu. */
  controls?: ReactNode;
}

const AUTO_REFRESH_STORAGE_KEY = 'polytaoli_auto_refresh';
const MIN_AUTO_REFRESH_SECONDS = 5;

export function Header({ isConnected, stats, totalProfit, lastUpdateTime: _lastUpdateTime, updateCount, dataCoverage, metrics, onLogout, username, controls }: HeaderProps) {
  const [isFlashing, setIsFlashing] = useState(false);
  const [accountBalance, setAccountBalance] = useState<AccountBalance | null>(null);

  // Auto-refresh: reload the whole page on a user-configurable interval.
  // Persisted to localStorage since a page reload wipes React state.
  const [refreshSecondsInput, setRefreshSecondsInput] = useState('30');
  const [autoRefreshEnabled, setAutoRefreshEnabled] = useState(false);
  const [refreshCountdown, setRefreshCountdown] = useState<number | null>(null);

  // Load the saved auto-refresh preference once on mount.
  useEffect(() => {
    try {
      const saved = localStorage.getItem(AUTO_REFRESH_STORAGE_KEY);
      if (!saved) return;
      const parsed = JSON.parse(saved);
      if (typeof parsed.seconds === 'number' && parsed.seconds >= MIN_AUTO_REFRESH_SECONDS) {
        setRefreshSecondsInput(String(parsed.seconds));
      }
      if (parsed.enabled) {
        setAutoRefreshEnabled(true);
      }
    } catch {
      // Private browsing or blocked storage - auto-refresh just stays off.
    }
  }, []);

  // Persist the preference whenever it changes.
  useEffect(() => {
    try {
      const seconds = parseInt(refreshSecondsInput);
      localStorage.setItem(
        AUTO_REFRESH_STORAGE_KEY,
        JSON.stringify({
          enabled: autoRefreshEnabled,
          seconds: isNaN(seconds) ? 30 : seconds,
        })
      );
    } catch {
      // Ignore - not critical if the preference doesn't persist.
    }
  }, [autoRefreshEnabled, refreshSecondsInput]);

  // Drive the countdown and trigger the reload when it hits zero.
  useEffect(() => {
    if (!autoRefreshEnabled) {
      setRefreshCountdown(null);
      return;
    }
    const seconds = parseInt(refreshSecondsInput);
    if (isNaN(seconds) || seconds < MIN_AUTO_REFRESH_SECONDS) {
      return;
    }
    setRefreshCountdown(seconds);
    const tick = setInterval(() => {
      setRefreshCountdown((prev) => {
        if (prev === null) return null;
        if (prev <= 1) {
          window.location.reload();
          return 0;
        }
        return prev - 1;
      });
    }, 1000);
    return () => clearInterval(tick);
  }, [autoRefreshEnabled, refreshSecondsInput]);

  const handleManualRefresh = () => window.location.reload();

  const handleToggleAutoRefresh = () => {
    const seconds = parseInt(refreshSecondsInput);
    if (!autoRefreshEnabled && (isNaN(seconds) || seconds < MIN_AUTO_REFRESH_SECONDS)) {
      return;
    }
    setAutoRefreshEnabled((prev) => !prev);
  };
  
  // Flash when new data arrives.
  useEffect(() => {
    if (updateCount > 0) {
      setIsFlashing(true);
      const timer = setTimeout(() => setIsFlashing(false), 300);
      return () => clearTimeout(timer);
    }
  }, [updateCount]);

  // Fetch account balances periodically.
  useEffect(() => {
    const fetchBalance = async () => {
      try {
        const response = await fetch(apiUrl(`/api/account-balance`));
        if (response.ok) {
          const data = await response.json();
          setAccountBalance(data);
        }
      } catch (error) {
        // Fail silently.
      }
    };

    // Fetch initially.
    fetchBalance();
    // Account endpoints are rate-limited; refresh at a sustainable cadence.
    const interval = setInterval(fetchBalance, 60000);
    return () => clearInterval(interval);
  }, []);

  // Refresh displayed time every second.
  const [, setTick] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => setTick(t => t + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  // Calculate coverage percentage.
  const getCoveragePercent = () => {
    if (dataCoverage.total_markets === 0) return 0;
    return Math.round((dataCoverage.both_ready / dataCoverage.total_markets) * 100);
  };

  const coveragePercent = getCoveragePercent();

  // Format displayed latency.
  const formatLatency = (latencyMs?: number) => {
    if (latencyMs === undefined || latencyMs === null) return '--';
    if (latencyMs < 1000) return `${latencyMs}ms`;
    return `${(latencyMs / 1000).toFixed(1)}s`;
  };

  // Get latency color.
  const getLatencyColor = (latencyMs?: number) => {
    if (latencyMs === undefined || latencyMs === null) return 'text-gray-500';
    if (latencyMs < 100) return 'text-green-400';
    if (latencyMs < 500) return 'text-yellow-400';
    if (latencyMs < 2000) return 'text-orange-400';
    return 'text-red-400';
  };

  // Format displayed balance.
  const formatBalance = (balance?: number) => {
    if (balance === undefined || balance === null) return '--';
    return balance >= 1000 ? `${(balance / 1000).toFixed(1)}k` : balance.toFixed(0);
  };

  return (
    <header className="border-b border-[--border-color] bg-[--bg-secondary]">
      <div className="px-2 h-10 flex items-center justify-between gap-2 overflow-x-auto">
        {/* Compact logo */}
        <div className="flex items-center gap-1 flex-shrink-0">
          <span className="text-sm">📊</span>
          <span className="font-semibold text-[--text-primary] text-xs">Arbitrage Scanner</span>
        </div>

        {/* Compact statistics */}
        <div className="flex items-center gap-2 text-xs flex-shrink-0">
          {/* Account balances */}
          {accountBalance && (
            <div className="flex items-center gap-1 px-2 py-0.5 rounded bg-[--bg-tertiary]" title={`Account Balances\nKalshi: ${accountBalance.kalshi.available ? `$${formatBalance(accountBalance.kalshi.balance)}` : 'Not configured or unavailable'}\nPolymarket: ${accountBalance.polymarket.available ? `$${formatBalance(accountBalance.polymarket.balance)}` : 'Not configured or unavailable'}`}>
              <span className="text-[10px] text-[--text-muted]">💰</span>
              <span className={`font-mono text-[10px] ${accountBalance.kalshi.available ? 'text-blue-400' : 'text-gray-500'}`} title={accountBalance.kalshi.error || ''}>
                K:${accountBalance.kalshi.available ? formatBalance(accountBalance.kalshi.balance) : '--'}
              </span>
              <span className={`font-mono text-[10px] ${accountBalance.polymarket.available ? 'text-purple-400' : 'text-gray-500'}`} title={accountBalance.polymarket.error || 'Polymarket balance'}>
                P:${accountBalance.polymarket.available ? formatBalance(accountBalance.polymarket.balance) : '--'}
              </span>
            </div>
          )}

          {/* Data coverage */}
          <div className="flex items-center gap-1 px-2 py-0.5 rounded bg-[--bg-tertiary]" title="Data coverage: authoritative Kalshi and Polymarket REST quote availability">
            <span className={`font-mono text-[10px] ${dataCoverage.kalshi_connected ? 'text-blue-400' : 'text-gray-500'}`}>
              K:REST {dataCoverage.kalshi_coverage}
            </span>
            <span className={`font-mono text-[10px] ${dataCoverage.polymarket_connected ? 'text-purple-400' : 'text-gray-500'}`}>
              P:REST {dataCoverage.polymarket_coverage}
            </span>
            <span className={`font-mono text-[10px] font-semibold ${coveragePercent >= 80 ? 'text-green-400' : coveragePercent >= 50 ? 'text-yellow-400' : 'text-red-400'}`}>
              ✓{dataCoverage.both_ready}/{dataCoverage.total_markets}
            </span>
          </div>

          {/* Platform latency */}
          <div className="flex items-center gap-1 px-2 py-0.5 rounded bg-[--bg-tertiary]" title="Platform data freshness">
            <span className="text-[10px] text-[--text-muted]">Feed:</span>
            <span className={`font-mono text-[10px] ${getLatencyColor(dataCoverage.kalshi_latency_ms)}`}>
              K:REST {formatLatency(dataCoverage.kalshi_latency_ms)}
            </span>
            <span className={`font-mono text-[10px] ${getLatencyColor(dataCoverage.polymarket_latency_ms)}`}>
              P:REST {formatLatency(dataCoverage.polymarket_latency_ms)}
            </span>
          </div>

          {/* API latency */}
          <div className="flex items-center gap-1 px-2 py-0.5 rounded bg-[--bg-tertiary]" title="API latency">
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
          
          {/* Connection status */}
          <div className="flex items-center gap-1">
            <span className={`status-dot ${isConnected ? 'status-connected animate-pulse-dot' : 'status-disconnected'}`} />
            <span className={`text-[10px] ${isConnected ? 'text-green-400' : 'text-red-400'}`}>
              {isConnected ? '● Live' : 'Offline'}
            </span>
          </div>

          <div className="h-3 w-px bg-[--border-color]" />

          {/* Page refresh controls */}
          <div className="flex items-center gap-1">
            <button
              onClick={handleManualRefresh}
              className="px-1.5 py-0.5 rounded text-[10px] font-semibold bg-gray-500/20 text-gray-300 hover:bg-gray-500/30 transition-colors"
              title="Refresh the page now"
            >
              🔄
            </button>
            <input
              type="number"
              value={refreshSecondsInput}
              onChange={(e) => setRefreshSecondsInput(e.target.value)}
              min={MIN_AUTO_REFRESH_SECONDS}
              className="w-12 text-[10px] px-1 py-0.5 rounded bg-[--bg-tertiary] border border-[--border-color] text-[--text-primary] text-center focus:outline-none focus:border-blue-500"
              title={`Auto-refresh interval in seconds (min ${MIN_AUTO_REFRESH_SECONDS})`}
            />
            <button
              onClick={handleToggleAutoRefresh}
              className={`px-1.5 py-0.5 rounded text-[10px] font-semibold font-mono transition-colors ${
                autoRefreshEnabled
                  ? 'bg-green-500/20 text-green-400 hover:bg-green-500/30'
                  : 'bg-gray-500/20 text-gray-400 hover:bg-gray-500/30'
              }`}
              title={autoRefreshEnabled ? 'Auto-refresh is on — click to stop' : 'Click to reload the page automatically on this interval'}
            >
              {autoRefreshEnabled ? `⏱ ${refreshCountdown ?? refreshSecondsInput}s` : 'Auto: Off'}
            </button>
          </div>

          <div className="h-3 w-px bg-[--border-color]" />

          {/* Strategy-specific controls */}
          {controls}

          {/* User information */}
          {username && (
            <div className="flex items-center gap-1 pl-2 border-l border-[--border-color]">
              <span className="text-[10px] text-[--text-secondary]">👤 {username}</span>
              {onLogout && (
                <button
                  onClick={onLogout}
                  className="text-[10px] text-[--text-muted] hover:text-red-400 transition-colors px-1"
                  title="Log out"
                >
                  Log out
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
