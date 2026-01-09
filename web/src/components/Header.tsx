import { useEffect, useState, useCallback } from 'react';
import { DataCoverage, AccountBalance, MetricsReport } from '../types';
import { 
  getAutoTradeStatus, 
  enableAutoTrade, 
  disableAutoTrade, 
  resetAutoTradeCount, 
  updateAutoTradeSettings,
  AutoTradeStatus 
} from '../utils/api';

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
  
  // 自动下单状态
  const [autoTradeStatus, setAutoTradeStatus] = useState<AutoTradeStatus | null>(null);
  const [showAutoTradeModal, setShowAutoTradeModal] = useState(false);
  const [autoTradeLoading, setAutoTradeLoading] = useState(false);
  const [durationInput, setDurationInput] = useState('500');
  const [autoTradeMessage, setAutoTradeMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);

  // 获取自动下单状态
  const fetchAutoTradeStatus = useCallback(async () => {
    try {
      const data = await getAutoTradeStatus(apiBaseUrl);
      setAutoTradeStatus(data);
      setDurationInput(String(data.min_duration_ms));
    } catch (error) {
      console.error('获取自动下单状态失败:', error);
    }
  }, [apiBaseUrl]);

  // 定期刷新自动下单状态
  useEffect(() => {
    fetchAutoTradeStatus();
    const interval = setInterval(fetchAutoTradeStatus, 2000);
    return () => clearInterval(interval);
  }, [fetchAutoTradeStatus]);

  // 显示消息
  const showAutoTradeMsg = (text: string, type: 'success' | 'error') => {
    setAutoTradeMessage({ text, type });
    setTimeout(() => setAutoTradeMessage(null), 3000);
  };

  // 切换自动下单开关
  const handleAutoTradeToggle = async () => {
    if (!autoTradeStatus) return;
    setAutoTradeLoading(true);
    try {
      if (autoTradeStatus.enabled) {
        const result = await disableAutoTrade(apiBaseUrl);
        if (result.success) showAutoTradeMsg('已关闭自动下单', 'success');
        else showAutoTradeMsg(result.error || '操作失败', 'error');
      } else {
        const result = await enableAutoTrade(apiBaseUrl);
        if (result.success) showAutoTradeMsg('已开启自动下单', 'success');
        else showAutoTradeMsg(result.error || '操作失败', 'error');
      }
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('操作失败', 'error');
    }
    setAutoTradeLoading(false);
  };

  // 重置次数
  const handleAutoTradeReset = async () => {
    setAutoTradeLoading(true);
    try {
      const result = await resetAutoTradeCount(apiBaseUrl);
      if (result.success) showAutoTradeMsg('次数已重置', 'success');
      else showAutoTradeMsg(result.error || '重置失败', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('重置失败', 'error');
    }
    setAutoTradeLoading(false);
  };

  // 更新持续时间阈值
  const handleUpdateDuration = async () => {
    const duration = parseInt(durationInput);
    if (isNaN(duration) || duration < 0) {
      showAutoTradeMsg('请输入有效的时间', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings(apiBaseUrl, { min_duration_ms: duration });
      if (result.success) showAutoTradeMsg('设置已更新', 'success');
      else showAutoTradeMsg(result.error || '更新失败', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('更新失败', 'error');
    }
    setAutoTradeLoading(false);
  };
  
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

          <div className="h-3 w-px bg-[--border-color]" />

          {/* 自动下单按钮 */}
          {autoTradeStatus && (
            <button
              onClick={() => setShowAutoTradeModal(true)}
              className={`flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-semibold transition-colors ${
                autoTradeStatus.enabled
                  ? 'bg-green-500/20 text-green-400 hover:bg-green-500/30'
                  : 'bg-gray-500/20 text-gray-400 hover:bg-gray-500/30'
              }`}
              title="自动下单设置"
            >
              <span>🤖</span>
              <span>{autoTradeStatus.enabled ? '自动' : '手动'}</span>
              <span className="font-mono">{autoTradeStatus.trade_count}/{autoTradeStatus.max_trade_count}</span>
            </button>
          )}

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

      {/* 自动下单设置弹窗 */}
      {showAutoTradeModal && autoTradeStatus && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowAutoTradeModal(false)}>
          <div 
            className="bg-[--bg-secondary] border border-[--border-color] rounded-lg shadow-xl w-80 max-w-[90vw]"
            onClick={(e) => e.stopPropagation()}
          >
            {/* 弹窗头部 */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-[--border-color]">
              <div className="flex items-center gap-2">
                <span className="text-base">🤖</span>
                <span className="font-semibold text-[--text-primary] text-sm">自动下单设置</span>
              </div>
              <button
                onClick={() => setShowAutoTradeModal(false)}
                className="text-[--text-muted] hover:text-[--text-primary] text-lg leading-none"
              >
                ×
              </button>
            </div>

            {/* 弹窗内容 */}
            <div className="p-4 space-y-4">
              {/* 开关状态 */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">当前状态</span>
                <div className="flex items-center gap-2">
                  <span className={`text-xs px-2 py-1 rounded font-semibold ${
                    autoTradeStatus.enabled
                      ? 'bg-green-500/20 text-green-400'
                      : 'bg-gray-500/20 text-gray-400'
                  }`}>
                    {autoTradeStatus.enabled ? '运行中' : '已关闭'}
                  </span>
                  <button
                    onClick={handleAutoTradeToggle}
                    disabled={autoTradeLoading}
                    className={`text-xs px-3 py-1 rounded font-semibold transition-colors ${
                      autoTradeStatus.enabled
                        ? 'bg-red-500/20 text-red-400 hover:bg-red-500/30'
                        : 'bg-green-500/20 text-green-400 hover:bg-green-500/30'
                    } ${autoTradeLoading ? 'opacity-50 cursor-not-allowed' : ''}`}
                  >
                    {autoTradeStatus.enabled ? '关闭' : '开启'}
                  </button>
                </div>
              </div>

              {/* 下单次数 */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">已下单次数</span>
                <div className="flex items-center gap-2">
                  <span className={`text-sm font-mono font-semibold ${
                    autoTradeStatus.remaining > 0 ? 'text-green-400' : 'text-red-400'
                  }`}>
                    {autoTradeStatus.trade_count} / {autoTradeStatus.max_trade_count}
                  </span>
                  <button
                    onClick={handleAutoTradeReset}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-orange-500/20 text-orange-400 hover:bg-orange-500/30 transition-colors disabled:opacity-50"
                  >
                    重置
                  </button>
                </div>
              </div>

              {/* 持续时间阈值 */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">最小持续时间</span>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={durationInput}
                    onChange={(e) => setDurationInput(e.target.value)}
                    className="w-20 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="0"
                    step="100"
                  />
                  <span className="text-xs text-[--text-muted]">ms</span>
                  <button
                    onClick={handleUpdateDuration}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                  >
                    更新
                  </button>
                </div>
              </div>

              {/* 单次最大金额 */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">单次最大金额</span>
                <span className="text-sm font-mono text-[--text-primary]">${autoTradeStatus.max_amount}</span>
              </div>

              {/* 上次下单时间 */}
              {autoTradeStatus.last_trade_time && (
                <div className="flex items-center justify-between">
                  <span className="text-sm text-[--text-secondary]">上次下单</span>
                  <span className="text-xs text-[--text-muted]">
                    {new Date(autoTradeStatus.last_trade_time).toLocaleString()}
                  </span>
                </div>
              )}

              {/* 消息提示 */}
              {autoTradeMessage && (
                <div className={`text-xs px-3 py-2 rounded ${
                  autoTradeMessage.type === 'success'
                    ? 'bg-green-500/20 text-green-400'
                    : 'bg-red-500/20 text-red-400'
                }`}>
                  {autoTradeMessage.text}
                </div>
              )}
            </div>

            {/* 弹窗底部说明 */}
            <div className="px-4 py-3 border-t border-[--border-color] bg-[--bg-tertiary] rounded-b-lg">
              <p className="text-[10px] text-[--text-muted] leading-relaxed">
                💡 测试阶段限制最多 {autoTradeStatus.max_trade_count} 次自动下单。
                只有持续时间超过 {autoTradeStatus.min_duration_ms}ms 的套利机会才会触发下单。
              </p>
            </div>
          </div>
        </div>
      )}
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
