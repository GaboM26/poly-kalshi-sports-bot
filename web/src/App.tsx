import { useMemo, useState, useEffect } from 'react';
import { Header } from './components/Header';
import { OpportunityList } from './components/OpportunityList';
import { LogPanel } from './components/LogPanel';
import { TrackingPanel } from './components/TrackingPanel';
import { ArbitrageHistory } from './components/ArbitrageHistory';
import { HistoryExplorer } from './components/HistoryExplorer';
import { OrderPanel } from './components/OrderPanel';
import { OrderForm } from './components/OrderForm';
import { Login } from './components/Login';
import { useWebSocket } from './hooks/useWebSocket';
import { MatchedMarketData } from './types';

function App() {
  // 登录状态管理
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [currentUsername, setCurrentUsername] = useState<string | null>(null);

  // 检查本地存储的 token
  useEffect(() => {
    const token = localStorage.getItem('auth_token');
    const username = localStorage.getItem('username');
    if (token && username) {
      setCurrentUsername(username);
      setIsAuthenticated(true);
    }
  }, []);

  // 登录成功处理
  const handleLoginSuccess = (_token: string, username: string) => {
    setCurrentUsername(username);
    setIsAuthenticated(true);
  };

  // 退出登录
  const handleLogout = () => {
    localStorage.removeItem('auth_token');
    localStorage.removeItem('username');
    setCurrentUsername(null);
    setIsAuthenticated(false);
  };
  const wsUrl = useMemo(() => {
    const devPorts = ['5175', '5176', '5177', '5173'];
    const isDev = devPorts.includes(window.location.port);
    return isDev 
      ? 'ws://localhost:3000/ws'
      : `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}/ws`;
  }, []);

  const apiBaseUrl = useMemo(() => {
    const devPorts = ['5175', '5176', '5177', '5173'];
    const isDev = devPorts.includes(window.location.port);
    return isDev 
      ? 'http://localhost:3000'
      : `${window.location.protocol}//${window.location.host}`;
  }, []);
  
  const { matchedMarkets, logs, isConnected, stats, lastUpdateTime, updateCount, dataCoverage } = useWebSocket(wsUrl);
  // 从匹配市场计算总利润（只计算有套利机会的市场）
  const totalProfit = matchedMarkets
    .filter(m => m.has_opportunity)
    .reduce((sum, m) => sum + m.expected_profit, 0);
  const [selectedMarket, setSelectedMarket] = useState<MatchedMarketData | null>(null);
  const [rightPanelTab, setRightPanelTab] = useState<'detail' | 'tracking' | 'history'>('detail');
  const [showHistoryExplorer, setShowHistoryExplorer] = useState(false);

  // 如果未登录，显示登录页面
  if (!isAuthenticated) {
    return <Login onLoginSuccess={handleLoginSuccess} apiBaseUrl={apiBaseUrl} />;
  }

  return (
    <div className="h-screen flex flex-col overflow-hidden">
      <Header 
        isConnected={isConnected} 
        stats={stats} 
        totalProfit={totalProfit}
        lastUpdateTime={lastUpdateTime}
        updateCount={updateCount}
        dataCoverage={dataCoverage}
        apiBaseUrl={apiBaseUrl}
        onLogout={handleLogout}
        username={currentUsername}
      />
      
      <main className="flex-1 flex overflow-hidden">
        {/* 左侧：市场列表 + 订单管理 */}
        <div className="flex-1 flex flex-col p-4 overflow-hidden gap-4">
          {/* 上部：市场列表 (70%) */}
          <div className="flex-[7] overflow-hidden">
            <OpportunityList 
              matchedMarkets={matchedMarkets}
              onSelectMarket={setSelectedMarket}
              apiBaseUrl={apiBaseUrl}
            />
          </div>
          {/* 下部：订单管理 (30%) */}
          <div className="flex-[3] overflow-hidden">
            <OrderPanel apiBaseUrl={apiBaseUrl} />
          </div>
        </div>
        
        {/* 右侧面板：详细信息/追踪/历史 + 日志 */}
        <aside className="w-96 flex-shrink-0 border-l border-[--border-color] bg-[--bg-secondary] flex flex-col">
          {/* 标签页切换 */}
          <div className="flex border-b border-[--border-color]">
            <button
              className={`flex-1 px-3 py-2 text-sm font-medium transition-colors ${
                rightPanelTab === 'detail'
                  ? 'text-[--accent-purple] border-b-2 border-[--accent-purple] bg-[--bg-tertiary]'
                  : 'text-[--text-muted] hover:text-[--text-secondary]'
              }`}
              onClick={() => setRightPanelTab('detail')}
            >
              📋 详情
            </button>
            <button
              className={`flex-1 px-3 py-2 text-sm font-medium transition-colors ${
                rightPanelTab === 'tracking'
                  ? 'text-[--accent-green] border-b-2 border-[--accent-green] bg-[--bg-tertiary]'
                  : 'text-[--text-muted] hover:text-[--text-secondary]'
              }`}
              onClick={() => setRightPanelTab('tracking')}
            >
              🎯 追踪
            </button>
            <button
              className={`flex-1 px-3 py-2 text-sm font-medium transition-colors ${
                rightPanelTab === 'history'
                  ? 'text-[--accent-yellow] border-b-2 border-[--accent-yellow] bg-[--bg-tertiary]'
                  : 'text-[--text-muted] hover:text-[--text-secondary]'
              }`}
              onClick={() => setRightPanelTab('history')}
            >
              📜 历史
            </button>
          </div>

          {/* 上方：详细信息或追踪面板或历史 */}
          <div className="flex-1 overflow-y-auto border-b border-[--border-color]">
            {rightPanelTab === 'detail' ? (
              <div className="p-3">
                {selectedMarket ? (
                  <div className="space-y-3">
                    {/* 标题栏 */}
                    <div className="pb-2 border-b border-[--border-color]">
                      <h3 className="text-xs font-semibold text-[--text-muted] uppercase tracking-wider">市场详情</h3>
                    </div>
                    
                    {/* 事件信息 - 紧凑卡片 */}
                    <div className="bg-[--bg-tertiary] rounded p-2.5">
                      <div className="text-[10px] text-[--text-muted] mb-1">事件</div>
                      <div className="text-xs text-[--text-primary] font-medium leading-tight">
                        {selectedMarket.event_name}
                      </div>
                      {selectedMarket.team_name && (
                        <div className="text-xs text-[--accent-yellow] mt-1 font-medium">
                          {selectedMarket.team_name}
                        </div>
                      )}
                      {selectedMarket.end_time && (
                        <div className="text-[10px] text-[--text-muted] mt-1.5 flex items-center gap-1">
                          <span>🕐</span>
                          <span>{formatDateTime(selectedMarket.end_time)}</span>
                        </div>
                      )}
                    </div>

                    {/* 数据状态 + 利润信息 - 合并显示 */}
                    <div className="grid grid-cols-2 gap-2">
                      {/* 数据状态 */}
                      <div className="bg-[--bg-tertiary] rounded p-2">
                        <div className="text-[10px] text-[--text-muted] mb-1.5">数据状态</div>
                        <div className="flex flex-col gap-1">
                          <span className={`px-1.5 py-0.5 rounded text-[10px] ${selectedMarket.kalshi_ready ? 'bg-blue-500/20 text-blue-400' : 'bg-gray-500/20 text-gray-400'}`}>
                            Kalshi {selectedMarket.kalshi_ready ? '✓' : '○'}
                          </span>
                          <span className={`px-1.5 py-0.5 rounded text-[10px] ${selectedMarket.poly_ready ? 'bg-purple-500/20 text-purple-400' : 'bg-gray-500/20 text-gray-400'}`}>
                            Poly {selectedMarket.poly_ready ? '✓' : '○'}
                          </span>
                        </div>
                      </div>

                      {/* 利润信息 */}
                      {selectedMarket.has_opportunity ? (
                        <div className="bg-gradient-to-br from-green-500/10 to-emerald-500/5 rounded p-2 border border-green-500/20">
                          <div className="text-[10px] text-[--text-muted] mb-1">套利机会</div>
                          <div className="text-base font-bold text-[--accent-green] leading-tight">
                            {selectedMarket.profit_margin.toFixed(2)}%
                          </div>
                          <div className="text-[10px] text-green-400 mt-0.5">
                            净利润: ${selectedMarket.expected_profit.toFixed(2)}
                          </div>
                        </div>
                      ) : (
                        <div className="bg-[--bg-tertiary] rounded p-2 flex items-center justify-center">
                          <span className="text-[10px] text-[--text-muted]">无套利机会</span>
                        </div>
                      )}
                    </div>

                    {/* 价格对比 - 紧凑布局 */}
                    <div className="bg-[--bg-tertiary] rounded p-2.5">
                      <div className="text-[10px] text-[--text-muted] mb-2">价格对比</div>
                      
                      {/* Kalshi */}
                      <div className="mb-2 pb-2 border-b border-[--border-color]">
                        <div className="flex items-center justify-between mb-1">
                          <span className="text-[10px] text-blue-400 font-medium">Kalshi</span>
                        </div>
                        <div className="flex justify-between items-center">
                          <div className="flex items-center gap-1">
                            <span className="text-[9px] text-[--text-muted]">Yes</span>
                            <span className="text-xs text-green-400 font-mono font-semibold">
                              {(selectedMarket.kalshi_yes_price * 100).toFixed(0)}¢
                            </span>
                          </div>
                          <div className="flex items-center gap-1">
                            <span className="text-[9px] text-[--text-muted]">No</span>
                            <span className="text-xs text-red-400 font-mono font-semibold">
                              {(selectedMarket.kalshi_no_price * 100).toFixed(0)}¢
                            </span>
                          </div>
                        </div>
                      </div>

                      {/* Polymarket */}
                      <div>
                        <div className="flex items-center justify-between mb-1">
                          <span className="text-[10px] text-purple-400 font-medium">Polymarket</span>
                        </div>
                        <div className="flex justify-between items-center">
                          <div className="flex items-center gap-1">
                            <span className="text-[9px] text-[--text-muted]">Yes</span>
                            <span className="text-xs text-green-400 font-mono font-semibold">
                              {(selectedMarket.poly_yes_price * 100).toFixed(0)}¢
                            </span>
                          </div>
                          <div className="flex items-center gap-1">
                            <span className="text-[9px] text-[--text-muted]">No</span>
                            <span className="text-xs text-red-400 font-mono font-semibold">
                              {(selectedMarket.poly_no_price * 100).toFixed(0)}¢
                            </span>
                          </div>
                        </div>
                      </div>
                    </div>

                    {/* 策略和费用 */}
                    {selectedMarket.has_opportunity && (
                      <div className="bg-[--bg-tertiary] rounded p-2.5 space-y-2">
                        {selectedMarket.arbitrage_type && (
                          <div>
                            <div className="text-[10px] text-[--text-muted] mb-1">套利策略</div>
                            <div className="text-xs text-[--text-primary] font-mono">
                              {selectedMarket.arbitrage_type}
                            </div>
                          </div>
                        )}
                        
                        {/* 费用明细 */}
                        <div className="pt-2 border-t border-[--border-color]">
                          <div className="text-[10px] text-[--text-muted] mb-1.5">费用明细</div>
                          <div className="space-y-1 text-[10px]">
                            {selectedMarket.kalshi_contracts !== undefined && (
                              <div className="flex justify-between">
                                <span className="text-[--text-muted]">Kalshi 合约数</span>
                                <span className="text-blue-400 font-mono">{Math.round(selectedMarket.kalshi_contracts)}</span>
                              </div>
                            )}
                            {selectedMarket.kalshi_fee !== undefined && (
                              <div className="flex justify-between">
                                <span className="text-[--text-muted]">Kalshi 手续费</span>
                                <span className="text-orange-400 font-mono">-${selectedMarket.kalshi_fee.toFixed(2)}</span>
                              </div>
                            )}
                            {selectedMarket.gross_profit !== undefined && (
                              <div className="flex justify-between">
                                <span className="text-[--text-muted]">毛利润</span>
                                <span className="text-[--text-secondary] font-mono">${selectedMarket.gross_profit.toFixed(2)}</span>
                              </div>
                            )}
                            <div className="flex justify-between pt-1 border-t border-[--border-color]">
                              <span className="text-[--text-muted] font-medium">净利润</span>
                              <span className="text-green-400 font-mono font-medium">${selectedMarket.expected_profit.toFixed(2)}</span>
                            </div>
                          </div>
                        </div>
                      </div>
                    )}

                    {/* 下单区域 */}
                    <div className="pt-2 border-t border-[--border-color]">
                      <div className="pb-2">
                        <h3 className="text-xs font-semibold text-[--text-muted] uppercase tracking-wider">交易下单</h3>
                      </div>
                      <OrderForm 
                        market={selectedMarket} 
                        apiBaseUrl={apiBaseUrl}
                      />
                    </div>
                  </div>
                ) : (
                  <div className="flex flex-col items-center justify-center h-full py-12">
                    <div className="text-3xl mb-2">👆</div>
                    <div className="text-xs text-[--text-secondary]">选择一个市场</div>
                    <div className="text-[10px] text-[--text-muted] mt-1">查看详细信息和交易</div>
                  </div>
                )}
              </div>
            ) : rightPanelTab === 'tracking' ? (
              <TrackingPanel apiBaseUrl={apiBaseUrl} />
            ) : (
              <ArbitrageHistory apiBaseUrl={apiBaseUrl} onOpenExplorer={() => setShowHistoryExplorer(true)} />
            )}
          </div>
          
          {/* 下方：日志 */}
          <div className="h-56 flex-shrink-0 px-3 pb-3">
            <LogPanel logs={logs} />
          </div>
        </aside>
      </main>

      {/* 历史探索弹窗 */}
      {showHistoryExplorer && (
        <HistoryExplorer 
          apiBaseUrl={apiBaseUrl} 
          onClose={() => setShowHistoryExplorer(false)} 
        />
      )}
    </div>
  );
}

function formatDateTime(dateString: string): string {
  const date = new Date(dateString);
  const now = new Date();
  const diffMs = date.getTime() - now.getTime();
  const diffHours = Math.floor(diffMs / (1000 * 60 * 60));
  const diffMinutes = Math.floor((diffMs % (1000 * 60 * 60)) / (1000 * 60));
  
  // 格式化日期时间
  const dateStr = date.toLocaleDateString('zh-CN', { 
    month: '2-digit', 
    day: '2-digit',
    weekday: 'short'
  });
  const timeStr = date.toLocaleTimeString('zh-CN', { 
    hour: '2-digit', 
    minute: '2-digit',
    hour12: false
  });
  
  // 如果在24小时内，显示倒计时
  if (diffMs > 0 && diffHours < 24) {
    if (diffHours > 0) {
      return `${dateStr} ${timeStr} (${diffHours}小时${diffMinutes}分钟后)`;
    } else if (diffMinutes > 0) {
      return `${dateStr} ${timeStr} (${diffMinutes}分钟后)`;
    } else {
      return `${dateStr} ${timeStr} (即将开始)`;
    }
  }
  
  // 如果已经过去
  if (diffMs < 0) {
    return `${dateStr} ${timeStr} (已开始)`;
  }
  
  return `${dateStr} ${timeStr}`;
}

export default App;
