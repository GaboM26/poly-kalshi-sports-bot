import { useEffect, useMemo, useState, useCallback } from 'react';
import { Header } from './components/Header';
import { Login } from './components/Login';
import { OpportunityList } from './components/OpportunityList';
import { TrackingPanel } from './components/TrackingPanel';
import { ArbitrageHistory } from './components/ArbitrageHistory';
import { HistoryExplorer } from './components/HistoryExplorer';
import { OrderPanel } from './components/OrderPanel';
import { OrderForm } from './components/OrderForm';
import { MetricsPanel } from './components/MetricsPanel';
import { PolyDebugOrder } from './components/PolyDebugOrder';
import { useWebSocket } from './hooks/useWebSocket';
import { MatchedMarketData, OrderbookDepthResponse } from './types';

function App() {
  // 登录状态管理
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [currentUsername, setCurrentUsername] = useState<string | null>(null);

  // 初始化时检查 localStorage 中的 token
  useEffect(() => {
    const token = localStorage.getItem('auth_token');
    const username = localStorage.getItem('username');
    if (token && username) {
      setIsAuthenticated(true);
      setCurrentUsername(username);
    }
  }, []);

  // 登录成功处理
  const handleLoginSuccess = (token: string, username: string) => {
    localStorage.setItem('auth_token', token);
    localStorage.setItem('username', username);
    setIsAuthenticated(true);
    setCurrentUsername(username);
  };

  // 退出登录
  const handleLogout = () => {
    localStorage.removeItem('auth_token');
    localStorage.removeItem('username');
    setIsAuthenticated(false);
    setCurrentUsername(null);
  };

  const wsUrl = useMemo(() => {
    const devPorts = ['5175', '5176', '5177', '5173'];
    const isDev = devPorts.includes(window.location.port);
    return isDev 
      ? 'ws://localhost:8000/ws'
      : `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}/ws`;
  }, []);

  const apiBaseUrl = useMemo(() => {
    const devPorts = ['5175', '5176', '5177', '5173'];
    const isDev = devPorts.includes(window.location.port);
    return isDev 
      ? 'http://localhost:8000'
      : `${window.location.protocol}//${window.location.host}`;
  }, []);
  
  const { matchedMarkets, isConnected, stats, lastUpdateTime, updateCount, dataCoverage, metrics } = useWebSocket(wsUrl);
  // 从匹配市场计算总利润（只计算有套利机会的市场）
  const totalProfit = matchedMarkets
    .filter(m => m.has_opportunity)
    .reduce((sum, m) => sum + m.expected_profit, 0);
  const [selectedMarket, setSelectedMarket] = useState<MatchedMarketData | null>(null);
  const [rightPanelTab, setRightPanelTab] = useState<'detail' | 'tracking'>('detail');
  const [leftBottomTab, setLeftBottomTab] = useState<'positions' | 'history'>('positions');
  const [showHistoryExplorer, setShowHistoryExplorer] = useState(false);
  const [showPolyDebug, setShowPolyDebug] = useState(false);
  const [orderbookDepth, setOrderbookDepth] = useState<OrderbookDepthResponse | null>(null);

  // 获取订单簿深度
  const fetchOrderbookDepth = useCallback(async (market: MatchedMarketData) => {
    try {
      const params = new URLSearchParams();
      if (market.kalshi_market_id) {
        params.append('kalshi_ticker', market.kalshi_market_id);
      }
      // Polymarket: Yes 深度用 own token，No 深度用 opponent token
      if (market.poly_token_id) {
        params.append('poly_token_id', market.poly_token_id);
      }
      if (market.poly_opponent_token_id) {
        params.append('poly_opponent_token_id', market.poly_opponent_token_id);
      }
      const response = await fetch(`${apiBaseUrl}/api/orderbook/depth?${params}`);
      if (response.ok) {
        const data = await response.json();
        setOrderbookDepth(data);
      }
    } catch (error) {
      console.error('获取订单簿深度失败:', error);
    }
  }, [apiBaseUrl]);

  // 选中市场变化时获取深度
  useEffect(() => {
    if (selectedMarket) {
      fetchOrderbookDepth(selectedMarket);
      // 每 3 秒刷新一次深度
      const interval = setInterval(() => fetchOrderbookDepth(selectedMarket), 3000);
      return () => clearInterval(interval);
    } else {
      setOrderbookDepth(null);
    }
  }, [selectedMarket, fetchOrderbookDepth]);

  // 未登录时显示登录页面
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
        metrics={metrics}
        apiBaseUrl={apiBaseUrl}
        onLogout={handleLogout}
        username={currentUsername}
      />
      
      <main className="flex-1 flex overflow-hidden gap-1 p-1 bg-[--bg-primary]">
        {/* 左侧：市场列表 + 持仓/历史记录 */}
        <div className="flex-1 flex flex-col overflow-hidden gap-1">
          {/* 上部：市场列表 (55%) */}
          <div className="flex-1 min-h-0 overflow-hidden flex flex-col bg-[--bg-secondary] rounded border border-[--border-color] shadow-lg" style={{flexBasis: '55%'}}>
            <OpportunityList 
              matchedMarkets={matchedMarkets}
              onSelectMarket={setSelectedMarket}
              apiBaseUrl={apiBaseUrl}
            />
          </div>
          {/* 下部：持仓/历史记录标签页 (45%) */}
          <div className="flex-1 min-h-0 overflow-hidden flex flex-col bg-[--bg-secondary] rounded border border-[--border-color] shadow-lg" style={{flexBasis: '45%'}}>
            {/* 标签页切换 */}
            <div className="flex border-b-2 border-[--border-color] flex-shrink-0 bg-[--bg-tertiary]">
              <button
                className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors border-b-2 ${
                  leftBottomTab === 'positions'
                    ? 'text-[--accent-blue] border-[--accent-blue] bg-[--bg-secondary]'
                    : 'text-[--text-muted] hover:text-[--text-secondary] border-transparent'
                }`}
                onClick={() => setLeftBottomTab('positions')}
              >
                💼 持仓
              </button>
              <button
                className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors border-b-2 ${
                  leftBottomTab === 'history'
                    ? 'text-[--accent-yellow] border-[--accent-yellow] bg-[--bg-secondary]'
                    : 'text-[--text-muted] hover:text-[--text-secondary] border-transparent'
                }`}
                onClick={() => setLeftBottomTab('history')}
              >
                📜 套利记录
              </button>
            </div>
            
            {/* 标签页内容 */}
            <div className="flex-1 overflow-hidden">
              {leftBottomTab === 'positions' ? (
                <OrderPanel apiBaseUrl={apiBaseUrl} />
              ) : (
                <ArbitrageHistory apiBaseUrl={apiBaseUrl} onOpenExplorer={() => setShowHistoryExplorer(true)} />
              )}
            </div>
          </div>
        </div>
        
        {/* 右侧面板：详细信息/追踪 + 性能 */}
        <aside className="w-96 flex-shrink-0 flex flex-col gap-1">
          {/* 上部：标签页区域 (70%) */}
          <div className="flex-1 min-h-0 flex flex-col bg-[--bg-secondary] rounded border border-[--border-color] shadow-lg" style={{flexBasis: '70%'}}>
            {/* 标签页切换 */}
            <div className="flex border-b-2 border-[--border-color] flex-shrink-0 bg-[--bg-tertiary]">
              <button
                className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors border-b-2 ${
                  rightPanelTab === 'detail'
                    ? 'text-[--accent-purple] border-[--accent-purple] bg-[--bg-secondary]'
                    : 'text-[--text-muted] hover:text-[--text-secondary] border-transparent'
                }`}
                onClick={() => setRightPanelTab('detail')}
              >
                📋 详情
              </button>
              <button
                className={`flex-1 px-3 py-1.5 text-xs font-medium transition-colors border-b-2 ${
                  rightPanelTab === 'tracking'
                    ? 'text-[--accent-green] border-[--accent-green] bg-[--bg-secondary]'
                    : 'text-[--text-muted] hover:text-[--text-secondary] border-transparent'
                }`}
                onClick={() => setRightPanelTab('tracking')}
              >
                🎯 追踪
              </button>
            </div>

            {/* 标签页内容 */}
            <div className="flex-1 overflow-y-auto overflow-x-hidden">
              {rightPanelTab === 'detail' ? (
                selectedMarket ? (
                  <div className="p-2 space-y-2">
                      {/* 标题栏 */}
                      <div className="pb-1.5 border-b border-[--border-color]">
                        <h3 className="text-[10px] font-semibold text-[--text-muted] uppercase tracking-wider">市场详情</h3>
                      </div>
                      
                      {/* 事件信息 - 紧凑卡片 */}
                      <div className="bg-[--bg-tertiary] rounded p-2">
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
                      <div className="grid grid-cols-2 gap-1.5">
                        {/* 数据状态 */}
                        <div className="bg-[--bg-tertiary] rounded p-1.5">
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
                          <div className="bg-gradient-to-br from-green-500/10 to-emerald-500/5 rounded p-1.5 border border-green-500/20">
                            <div className="text-[10px] text-[--text-muted] mb-1">套利机会</div>
                            <div className="text-base font-bold text-[--accent-green] leading-tight">
                              {selectedMarket.profit_margin.toFixed(2)}%
                            </div>
                            <div className="text-[10px] text-green-400 mt-0.5">
                              净利润: ${selectedMarket.expected_profit.toFixed(2)}
                            </div>
                          </div>
                        ) : (
                          <div className="bg-[--bg-tertiary] rounded p-1.5 border border-gray-500/20">
                            <div className="text-[10px] text-[--text-muted] mb-1">套利结果</div>
                            <div className="text-base font-bold text-gray-400 leading-tight">
                              {selectedMarket.profit_margin.toFixed(2)}%
                            </div>
                            <div className="text-[10px] text-gray-400 mt-0.5">
                              净利润: ${selectedMarket.expected_profit.toFixed(2)}
                            </div>
                          </div>
                        )}
                      </div>

                      {/* 价格与深度 - 两平台并排 */}
                      <div className="grid grid-cols-2 gap-2">
                        {/* Kalshi */}
                        <div className="bg-[--bg-tertiary] rounded p-2">
                          <div className="text-[10px] text-blue-400 font-medium mb-1.5">Kalshi</div>
                          <div className="space-y-1">
                            {/* Yes */}
                            <div className="space-y-0.5">
                              <div className="flex items-center justify-between">
                                <span className="text-[9px] text-green-400 font-medium">Yes</span>
                                <span className="text-xs text-[--text-primary] font-mono font-semibold">
                                  {(selectedMarket.kalshi_yes_price * 100).toFixed(0)}¢
                                </span>
                              </div>
                              <div className="flex items-center justify-between text-[9px]">
                                <span className="text-cyan-400 font-mono">
                                  ×{orderbookDepth?.kalshi?.yes?.size?.toFixed(0) ?? '-'}
                                </span>
                                <span className="text-yellow-400 font-mono">
                                  ${orderbookDepth?.kalshi?.yes?.price != null && orderbookDepth?.kalshi?.yes?.size != null 
                                    ? (orderbookDepth.kalshi.yes.price * orderbookDepth.kalshi.yes.size).toFixed(0) 
                                    : '-'}
                                </span>
                              </div>
                            </div>
                            {/* No */}
                            <div className="space-y-0.5 pt-1 border-t border-[--border-color]">
                              <div className="flex items-center justify-between">
                                <span className="text-[9px] text-red-400 font-medium">No</span>
                                <span className="text-xs text-[--text-primary] font-mono font-semibold">
                                  {(selectedMarket.kalshi_no_price * 100).toFixed(0)}¢
                                </span>
                              </div>
                              <div className="flex items-center justify-between text-[9px]">
                                <span className="text-cyan-400 font-mono">
                                  ×{orderbookDepth?.kalshi?.no?.size?.toFixed(0) ?? '-'}
                                </span>
                                <span className="text-yellow-400 font-mono">
                                  ${orderbookDepth?.kalshi?.no?.price != null && orderbookDepth?.kalshi?.no?.size != null 
                                    ? (orderbookDepth.kalshi.no.price * orderbookDepth.kalshi.no.size).toFixed(0) 
                                    : '-'}
                                </span>
                              </div>
                            </div>
                          </div>
                        </div>

                        {/* Polymarket */}
                        <div className="bg-[--bg-tertiary] rounded p-2">
                          <div className="text-[10px] text-purple-400 font-medium mb-1.5">Polymarket</div>
                          <div className="space-y-1">
                            {/* Yes */}
                            <div className="space-y-0.5">
                              <div className="flex items-center justify-between">
                                <span className="text-[9px] text-green-400 font-medium">Yes</span>
                                <span className="text-xs text-[--text-primary] font-mono font-semibold">
                                  {(selectedMarket.poly_yes_price * 100).toFixed(0)}¢
                                </span>
                              </div>
                              <div className="flex items-center justify-between text-[9px]">
                                <span className="text-cyan-400 font-mono">
                                  ×{orderbookDepth?.polymarket?.yes?.size?.toFixed(0) ?? '-'}
                                </span>
                                <span className="text-yellow-400 font-mono">
                                  ${orderbookDepth?.polymarket?.yes?.price != null && orderbookDepth?.polymarket?.yes?.size != null 
                                    ? (orderbookDepth.polymarket.yes.price * orderbookDepth.polymarket.yes.size).toFixed(0) 
                                    : '-'}
                                </span>
                              </div>
                            </div>
                            {/* No */}
                            <div className="space-y-0.5 pt-1 border-t border-[--border-color]">
                              <div className="flex items-center justify-between">
                                <span className="text-[9px] text-red-400 font-medium">No</span>
                                <span className="text-xs text-[--text-primary] font-mono font-semibold">
                                  {(selectedMarket.poly_no_price * 100).toFixed(0)}¢
                                </span>
                              </div>
                              <div className="flex items-center justify-between text-[9px]">
                                <span className="text-cyan-400 font-mono">
                                  ×{orderbookDepth?.polymarket?.no?.size?.toFixed(0) ?? '-'}
                                </span>
                                <span className="text-yellow-400 font-mono">
                                  ${orderbookDepth?.polymarket?.no?.price != null && orderbookDepth?.polymarket?.no?.size != null 
                                    ? (orderbookDepth.polymarket.no.price * orderbookDepth.polymarket.no.size).toFixed(0) 
                                    : '-'}
                                </span>
                              </div>
                            </div>
                          </div>
                        </div>
                      </div>

                      {/* 套利计算详情 - 始终显示 */}
                      <div className="bg-[--bg-tertiary] rounded p-2 space-y-1.5">
                        {selectedMarket.arbitrage_type && (
                          <div>
                            <div className="text-[10px] text-[--text-muted] mb-1">套利策略</div>
                            <div className={`text-xs font-mono ${selectedMarket.has_opportunity ? 'text-[--text-primary]' : 'text-gray-400'}`}>
                              {selectedMarket.arbitrage_type}
                            </div>
                          </div>
                        )}
                        
                        {/* 费用明细 */}
                        <div className={selectedMarket.arbitrage_type ? 'pt-2 border-t border-[--border-color]' : ''}>
                          <div className="text-[10px] text-[--text-muted] mb-1.5">套利计算详情</div>
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
                                <span className={`font-mono ${selectedMarket.gross_profit >= 0 ? 'text-[--text-secondary]' : 'text-red-400'}`}>
                                  ${selectedMarket.gross_profit.toFixed(2)}
                                </span>
                              </div>
                            )}
                            <div className="flex justify-between pt-1 border-t border-[--border-color]">
                              <span className="text-[--text-muted] font-medium">净利润</span>
                              <span className={`font-mono font-medium ${selectedMarket.expected_profit >= 0 ? (selectedMarket.has_opportunity ? 'text-green-400' : 'text-gray-400') : 'text-red-400'}`}>
                                ${selectedMarket.expected_profit.toFixed(2)}
                              </span>
                            </div>
                          </div>
                        </div>
                      </div>

                      {/* 下单区域 */}
                      <div className="pt-1.5 border-t border-[--border-color]">
                        <div className="pb-1.5">
                          <h3 className="text-[10px] font-semibold text-[--text-muted] uppercase tracking-wider">交易下单</h3>
                        </div>
                        <OrderForm 
                          market={selectedMarket} 
                          apiBaseUrl={apiBaseUrl}
                        />
                      </div>
                  </div>
                ) : (
                  <div className="flex flex-col items-center justify-center h-full">
                    <div className="text-3xl mb-2">👆</div>
                    <div className="text-xs text-[--text-secondary]">选择一个市场</div>
                    <div className="text-[10px] text-[--text-muted] mt-1">查看详细信息和交易</div>
                  </div>
                )
              ) : (
                <TrackingPanel apiBaseUrl={apiBaseUrl} />
              )}
            </div>
          </div>
          
          {/* 下部：性能监控 (30%) */}
          <div className="flex-1 min-h-0 overflow-hidden flex flex-col bg-[--bg-secondary] rounded border border-[--border-color] shadow-lg" style={{flexBasis: '30%'}}>
            <MetricsPanel metrics={metrics} />
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

      {/* Polymarket 调试下单弹窗 */}
      {showPolyDebug && (
        <PolyDebugOrder
          apiBaseUrl={apiBaseUrl}
          onClose={() => setShowPolyDebug(false)}
        />
      )}

      {/* 调试按钮 - 固定在右下角 */}
      <button
        onClick={() => setShowPolyDebug(true)}
        className="fixed bottom-4 right-4 w-12 h-12 rounded-full bg-purple-500 hover:bg-purple-600 text-white shadow-lg flex items-center justify-center text-xl z-40"
        title="Poly 手动下单调试"
      >
        🔧
      </button>
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
