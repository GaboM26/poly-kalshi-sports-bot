import { useEffect, useState, useCallback } from 'react';
import { DataCoverage, AccountBalance, MetricsReport } from '../types';
import { 
  getAutoTradeStatus, 
  enableAutoTrade, 
  disableAutoTrade, 
  resetAutoTradeCount, 
  updateAutoTradeSettings,
  AutoTradeStatus,
  getAppSettings,
  updateAppSettings,
  AppSettings
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
  const [maxTradeCountInput, setMaxTradeCountInput] = useState('2');
  const [maxContractsInput, setMaxContractsInput] = useState('100');
  const [minContractsInput, setMinContractsInput] = useState('10');
  const [autoTradeMessage, setAutoTradeMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);

  // 应用设置状态
  const [appSettings, setAppSettings] = useState<AppSettings | null>(null);
  const [showSettingsModal, setShowSettingsModal] = useState(false);
  const [settingsLoading, setSettingsLoading] = useState(false);
  const [settingsMessage, setSettingsMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);
  // 编辑用的临时值
  const [editRefreshInterval, setEditRefreshInterval] = useState('5');
  const [editMinProfit, setEditMinProfit] = useState('1.0');
  const [editDefaultBet, setEditDefaultBet] = useState('10');
  const [editTrackingThreshold, setEditTrackingThreshold] = useState('2.0');

  // 获取自动下单状态
  const fetchAutoTradeStatus = useCallback(async () => {
    try {
      const data = await getAutoTradeStatus(apiBaseUrl);
      setAutoTradeStatus(data);
      setDurationInput(String(data.min_duration_ms));
      setMaxTradeCountInput(String(data.max_trade_count));
      setMaxContractsInput(String(data.max_contracts));
      setMinContractsInput(String(data.min_contracts));
    } catch (error) {
      console.error('获取自动下单状态失败:', error);
    }
  }, [apiBaseUrl]);

  // 获取应用设置
  const fetchAppSettings = useCallback(async () => {
    try {
      const data = await getAppSettings(apiBaseUrl);
      setAppSettings(data);
      setEditRefreshInterval(String(data.refresh_interval));
      setEditMinProfit(String(data.min_profit_margin));
      setEditDefaultBet(String(data.default_bet_amount));
      setEditTrackingThreshold(String(data.tracking_threshold));
    } catch (error) {
      console.error('获取应用设置失败:', error);
    }
  }, [apiBaseUrl]);

  // 定期刷新自动下单状态
  useEffect(() => {
    fetchAutoTradeStatus();
    const interval = setInterval(fetchAutoTradeStatus, 2000);
    return () => clearInterval(interval);
  }, [fetchAutoTradeStatus]);

  // 初始加载应用设置
  useEffect(() => {
    fetchAppSettings();
  }, [fetchAppSettings]);

  // 显示消息
  const showAutoTradeMsg = (text: string, type: 'success' | 'error') => {
    setAutoTradeMessage({ text, type });
    setTimeout(() => setAutoTradeMessage(null), 3000);
  };

  // 显示设置消息
  const showSettingsMsg = (text: string, type: 'success' | 'error') => {
    setSettingsMessage({ text, type });
    setTimeout(() => setSettingsMessage(null), 3000);
  };

  // 更新应用设置
  const handleUpdateSettings = async () => {
    const refreshInterval = parseInt(editRefreshInterval);
    const minProfit = parseFloat(editMinProfit);
    const defaultBet = parseFloat(editDefaultBet);
    const trackingThreshold = parseFloat(editTrackingThreshold);

    if (isNaN(refreshInterval) || refreshInterval < 1) {
      showSettingsMsg('刷新间隔至少1秒', 'error');
      return;
    }
    if (isNaN(minProfit) || minProfit < 0) {
      showSettingsMsg('最小利润率不能为负', 'error');
      return;
    }
    if (isNaN(defaultBet) || defaultBet <= 0) {
      showSettingsMsg('默认金额必须大于0', 'error');
      return;
    }
    if (isNaN(trackingThreshold) || trackingThreshold < 0) {
      showSettingsMsg('追踪阈值不能为负', 'error');
      return;
    }

    setSettingsLoading(true);
    try {
      const result = await updateAppSettings(apiBaseUrl, {
        refresh_interval: refreshInterval,
        min_profit_margin: minProfit,
        default_bet_amount: defaultBet,
        tracking_threshold: trackingThreshold,
      });
      if (result.success) {
        showSettingsMsg('设置已保存（重启后完全生效）', 'success');
        await fetchAppSettings();
      } else {
        showSettingsMsg(result.error || '保存失败', 'error');
      }
    } catch (error) {
      showSettingsMsg('保存失败', 'error');
    }
    setSettingsLoading(false);
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

  // 更新最大下单次数
  const handleUpdateMaxTradeCount = async () => {
    const count = parseInt(maxTradeCountInput);
    if (isNaN(count) || count < 1) {
      showAutoTradeMsg('请输入有效的次数（≥1）', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings(apiBaseUrl, { max_trade_count: count });
      if (result.success) showAutoTradeMsg('最大次数已更新', 'success');
      else showAutoTradeMsg(result.error || '更新失败', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('更新失败', 'error');
    }
    setAutoTradeLoading(false);
  };

  // 切换灵活下单模式
  const handleToggleFlexibleMode = async () => {
    if (!autoTradeStatus) return;
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings(apiBaseUrl, { 
        flexible_mode: !autoTradeStatus.flexible_mode 
      });
      if (result.success) {
        showAutoTradeMsg(
          autoTradeStatus.flexible_mode ? '已切换为固定模式' : '已切换为灵活模式', 
          'success'
        );
      } else {
        showAutoTradeMsg(result.error || '切换失败', 'error');
      }
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('切换失败', 'error');
    }
    setAutoTradeLoading(false);
  };

  // 更新最大合同数
  const handleUpdateMaxContracts = async () => {
    const count = parseInt(maxContractsInput);
    if (isNaN(count) || count < 1) {
      showAutoTradeMsg('请输入有效的合同数（≥1）', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings(apiBaseUrl, { max_contracts: count });
      if (result.success) showAutoTradeMsg('最大合同数已更新', 'success');
      else showAutoTradeMsg(result.error || '更新失败', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('更新失败', 'error');
    }
    setAutoTradeLoading(false);
  };

  // 更新最低合同数
  const handleUpdateMinContracts = async () => {
    const count = parseInt(minContractsInput);
    if (isNaN(count) || count < 1) {
      showAutoTradeMsg('请输入有效的合同数（≥1）', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings(apiBaseUrl, { min_contracts: count });
      if (result.success) showAutoTradeMsg('最低合同数已更新', 'success');
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

          {/* 设置按钮 */}
          <button
            onClick={() => setShowSettingsModal(true)}
            className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-semibold bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors"
            title="应用设置"
          >
            <span>⚙️</span>
            <span>设置</span>
          </button>

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
                    {autoTradeStatus.trade_count}
                  </span>
                  <span className="text-[--text-muted]">/</span>
                  <input
                    type="number"
                    value={maxTradeCountInput}
                    onChange={(e) => setMaxTradeCountInput(e.target.value)}
                    className="w-12 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-center"
                    min="1"
                  />
                  <button
                    onClick={handleUpdateMaxTradeCount}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                  >
                    更新
                  </button>
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

              {/* 分隔线 */}
              <div className="border-t border-[--border-color] my-2"></div>

              {/* 下单模式 */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">下单模式</span>
                <div className="flex items-center gap-2">
                  <span className={`text-xs px-2 py-1 rounded font-semibold ${
                    autoTradeStatus.flexible_mode
                      ? 'bg-purple-500/20 text-purple-400'
                      : 'bg-blue-500/20 text-blue-400'
                  }`}>
                    {autoTradeStatus.flexible_mode ? '灵活模式' : '固定模式'}
                  </span>
                  <button
                    onClick={handleToggleFlexibleMode}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-gray-500/20 text-gray-300 hover:bg-gray-500/30 transition-colors disabled:opacity-50"
                  >
                    切换
                  </button>
                </div>
              </div>

              {/* 最大合同数 */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">单次最大合同</span>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={maxContractsInput}
                    onChange={(e) => setMaxContractsInput(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-center"
                    min="1"
                  />
                  <span className="text-xs text-[--text-muted]">份</span>
                  <button
                    onClick={handleUpdateMaxContracts}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                  >
                    更新
                  </button>
                </div>
              </div>

              {/* 最低合同数 */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">最低合同门槛</span>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={minContractsInput}
                    onChange={(e) => setMinContractsInput(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-center"
                    min="1"
                  />
                  <span className="text-xs text-[--text-muted]">份</span>
                  <button
                    onClick={handleUpdateMinContracts}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                  >
                    更新
                  </button>
                </div>
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
                💡 {autoTradeStatus.flexible_mode 
                  ? `灵活模式：深度10-20时下${autoTradeStatus.min_contracts}份，深度≥20时下深度的一半（上限${autoTradeStatus.max_contracts}份）` 
                  : `固定模式：每次固定下${autoTradeStatus.min_contracts}份合同`
                }。双平台深度都需≥{autoTradeStatus.min_contracts}份才会下单。
              </p>
            </div>
          </div>
        </div>
      )}

      {/* 应用设置弹窗 */}
      {showSettingsModal && appSettings && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowSettingsModal(false)}>
          <div 
            className="bg-[--bg-secondary] border border-[--border-color] rounded-lg shadow-xl w-96 max-w-[90vw]"
            onClick={(e) => e.stopPropagation()}
          >
            {/* 弹窗头部 */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-[--border-color]">
              <div className="flex items-center gap-2">
                <span className="text-base">⚙️</span>
                <span className="font-semibold text-[--text-primary] text-sm">应用设置</span>
              </div>
              <button
                onClick={() => setShowSettingsModal(false)}
                className="text-[--text-muted] hover:text-[--text-primary] text-lg leading-none"
              >
                ×
              </button>
            </div>

            {/* 弹窗内容 */}
            <div className="p-4 space-y-4">
              {/* 刷新间隔 */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm text-[--text-secondary]">数据刷新间隔</span>
                  <p className="text-[10px] text-[--text-muted]">定时扫描市场的间隔</p>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={editRefreshInterval}
                    onChange={(e) => setEditRefreshInterval(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="1"
                  />
                  <span className="text-xs text-[--text-muted]">秒</span>
                </div>
              </div>

              {/* 最小利润率 */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm text-[--text-secondary]">最小利润率</span>
                  <p className="text-[10px] text-[--text-muted]">低于此利润率的机会不显示</p>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={editMinProfit}
                    onChange={(e) => setEditMinProfit(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="0"
                    step="0.1"
                  />
                  <span className="text-xs text-[--text-muted]">%</span>
                </div>
              </div>

              {/* 默认下注金额 */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm text-[--text-secondary]">默认下注金额</span>
                  <p className="text-[10px] text-[--text-muted]">套利计算使用的默认金额</p>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-[--text-muted]">$</span>
                  <input
                    type="number"
                    value={editDefaultBet}
                    onChange={(e) => setEditDefaultBet(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="1"
                    step="1"
                  />
                </div>
              </div>

              {/* 追踪阈值 */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm text-[--text-secondary]">追踪阈值</span>
                  <p className="text-[10px] text-[--text-muted]">超过此利润率开始记录追踪</p>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={editTrackingThreshold}
                    onChange={(e) => setEditTrackingThreshold(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="0"
                    step="0.1"
                  />
                  <span className="text-xs text-[--text-muted]">%</span>
                </div>
              </div>

              {/* 保存按钮 */}
              <div className="flex justify-end pt-2">
                <button
                  onClick={handleUpdateSettings}
                  disabled={settingsLoading}
                  className="px-4 py-1.5 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors text-xs font-semibold disabled:opacity-50"
                >
                  {settingsLoading ? '保存中...' : '保存设置'}
                </button>
              </div>

              {/* 消息提示 */}
              {settingsMessage && (
                <div className={`text-xs px-3 py-2 rounded ${
                  settingsMessage.type === 'success'
                    ? 'bg-green-500/20 text-green-400'
                    : 'bg-red-500/20 text-red-400'
                }`}>
                  {settingsMessage.text}
                </div>
              )}
            </div>

            {/* 弹窗底部说明 */}
            <div className="px-4 py-3 border-t border-[--border-color] bg-[--bg-tertiary] rounded-b-lg">
              <p className="text-[10px] text-[--text-muted] leading-relaxed">
                💡 设置保存后会立即生效。部分设置（如刷新间隔）需要重启服务才能完全生效。
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
