import { useMemo, useState } from 'react';
import { Header } from './components/Header';
import { OpportunityList } from './components/OpportunityList';
import { LogPanel } from './components/LogPanel';
import { TrackingPanel } from './components/TrackingPanel';
import { ArbitrageHistory } from './components/ArbitrageHistory';
import { useWebSocket } from './hooks/useWebSocket';
import { MatchedMarketData } from './types';

function App() {
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

  return (
    <div className="min-h-screen flex flex-col">
      <Header 
        isConnected={isConnected} 
        stats={stats} 
        totalProfit={totalProfit}
        lastUpdateTime={lastUpdateTime}
        updateCount={updateCount}
        dataCoverage={dataCoverage}
      />
      
      <main className="flex-1 flex overflow-hidden">
        {/* 左侧套利列表 */}
        <div className="flex-1 p-4 overflow-auto">
          <OpportunityList 
            matchedMarkets={matchedMarkets}
            onSelectMarket={setSelectedMarket}
          />
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
              <div className="p-4">
                {selectedMarket ? (
                  <div className="card">
                    <h3 className="text-lg font-semibold mb-4 text-[--text-primary]">市场详情</h3>
                    
                    <div className="space-y-4">
                      {/* 事件信息 */}
                      <div>
                        <div className="text-xs text-[--text-muted] mb-1">事件</div>
                        <div className="text-sm text-[--text-primary] font-medium">
                          {selectedMarket.event_name}
                        </div>
                        {selectedMarket.team_name && (
                          <div className="text-xs text-[--accent-yellow] mt-1">
                            {selectedMarket.team_name}
                          </div>
                        )}
                        {selectedMarket.end_time && (
                          <div className="text-xs text-[--text-muted] mt-2 flex items-center gap-1">
                            <span>🕐</span>
                            <span>比赛时间: {formatDateTime(selectedMarket.end_time)}</span>
                          </div>
                        )}
                      </div>

                      {/* 数据状态 */}
                      <div className="flex gap-2">
                        <span className={`px-2 py-1 rounded text-xs ${selectedMarket.kalshi_ready ? 'bg-blue-500/20 text-blue-400' : 'bg-gray-500/20 text-gray-400'}`}>
                          Kalshi {selectedMarket.kalshi_ready ? '✓' : '○'}
                        </span>
                        <span className={`px-2 py-1 rounded text-xs ${selectedMarket.poly_ready ? 'bg-purple-500/20 text-purple-400' : 'bg-gray-500/20 text-gray-400'}`}>
                          Poly {selectedMarket.poly_ready ? '✓' : '○'}
                        </span>
                      </div>

                      {/* 利润信息 */}
                      {selectedMarket.has_opportunity && (
                        <div className="grid grid-cols-2 gap-3">
                          <div>
                            <div className="text-xs text-[--text-muted] mb-1">利润率</div>
                            <div className="text-lg font-bold text-[--accent-green]">
                              {selectedMarket.profit_margin.toFixed(2)}%
                            </div>
                          </div>
                          <div>
                            <div className="text-xs text-[--text-muted] mb-1">预期利润</div>
                            <div className="text-lg font-bold text-[--accent-green]">
                              ${selectedMarket.expected_profit.toFixed(2)}
                            </div>
                          </div>
                        </div>
                      )}

                      {/* Kalshi 市场 */}
                      <div>
                        <div className="text-xs text-[--text-muted] mb-2">Kalshi 市场</div>
                        <div className="bg-[--bg-tertiary] rounded p-3 space-y-2">
                          <div className="flex justify-between text-sm">
                            <span className="text-[--text-secondary]">Yes 价格:</span>
                            <span className="text-green-400 font-mono">
                              {(selectedMarket.kalshi_yes_price * 100).toFixed(0)}¢
                            </span>
                          </div>
                          <div className="flex justify-between text-sm">
                            <span className="text-[--text-secondary]">No 价格:</span>
                            <span className="text-red-400 font-mono">
                              {(selectedMarket.kalshi_no_price * 100).toFixed(0)}¢
                            </span>
                          </div>
                        </div>
                      </div>

                      {/* Polymarket 市场 */}
                      <div>
                        <div className="text-xs text-[--text-muted] mb-2">Polymarket 市场</div>
                        <div className="bg-[--bg-tertiary] rounded p-3 space-y-2">
                          <div className="flex justify-between text-sm">
                            <span className="text-[--text-secondary]">Yes 价格:</span>
                            <span className="text-green-400 font-mono">
                              {(selectedMarket.poly_yes_price * 100).toFixed(0)}¢
                            </span>
                          </div>
                          <div className="flex justify-between text-sm">
                            <span className="text-[--text-secondary]">No 价格:</span>
                            <span className="text-red-400 font-mono">
                              {(selectedMarket.poly_no_price * 100).toFixed(0)}¢
                            </span>
                          </div>
                        </div>
                      </div>

                      {/* 策略 */}
                      {selectedMarket.has_opportunity && selectedMarket.arbitrage_type && (
                        <div>
                          <div className="text-xs text-[--text-muted] mb-1">套利策略</div>
                          <div className="text-sm text-[--text-primary] bg-[--bg-tertiary] rounded px-3 py-2">
                            {selectedMarket.arbitrage_type}
                          </div>
                        </div>
                      )}
                    </div>
                  </div>
                ) : (
                  <div className="card p-8 text-center">
                    <div className="text-4xl mb-3">👆</div>
                    <div className="text-[--text-secondary]">选择一个市场</div>
                    <div className="text-[--text-muted] text-xs mt-1">查看详细信息</div>
                  </div>
                )}
              </div>
            ) : rightPanelTab === 'tracking' ? (
              <TrackingPanel apiBaseUrl={apiBaseUrl} />
            ) : (
              <ArbitrageHistory apiBaseUrl={apiBaseUrl} />
            )}
          </div>
          
          {/* 下方：日志 */}
          <div className="h-48 flex-shrink-0">
            <LogPanel logs={logs} />
          </div>
        </aside>
      </main>
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
