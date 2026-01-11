import { useEffect, useState } from 'react';

interface ProfitHistoryEntry {
  time: string;
  profit_margin: number;
  kalshi_price: number;
  polymarket_price: number;
}

interface ArbitrageRecord {
  event_name: string;
  team_name: string;
  kalshi_market_id: string;
  polymarket_market_id: string;
  start_time: string;
  end_time?: string;
  duration_seconds?: number;
  max_profit_margin: number;
  max_profit_time?: string;
  profit_history?: ProfitHistoryEntry[];
}

interface AutoTradeRecord {
  id: number;
  event_name: string;
  team_name: string;
  kalshi_market_id: string;
  polymarket_market_id: string;
  kalshi_side: string;
  polymarket_side: string;
  kalshi_contracts: number;
  kalshi_price: number;
  kalshi_fee: number;
  polymarket_amount: number;
  polymarket_price: number;
  total_amount: number;
  profit_margin: number;
  /** 机会持续时间（下单前，毫秒） */
  duration_ms: number;
  /** 从发现机会到下单完成的总耗时（毫秒） */
  total_duration_ms: number;
  kalshi_success: boolean;
  polymarket_success: boolean;
  kalshi_order_id?: string;
  polymarket_order_id?: string;
  kalshi_error?: string;
  polymarket_error?: string;
  status: string; // "executed" | "partial" | "skipped"
  skip_reason?: string;
  created_at: string;
}

interface ArbitrageHistoryData {
  active: ArbitrageRecord[];
  completed: ArbitrageRecord[];
}

interface ArbitrageHistoryProps {
  apiBaseUrl: string;
  onOpenExplorer?: () => void;
}

export function ArbitrageHistory({ apiBaseUrl, onOpenExplorer }: ArbitrageHistoryProps) {
  const [data, setData] = useState<ArbitrageHistoryData>({ active: [], completed: [] });
  const [autoTradeRecords, setAutoTradeRecords] = useState<AutoTradeRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [selectedRecord, setSelectedRecord] = useState<ArbitrageRecord | null>(null);
  const [selectedAutoTrade, setSelectedAutoTrade] = useState<AutoTradeRecord | null>(null);
  const [activeTab, setActiveTab] = useState<'tracking' | 'trades'>('tracking');

  useEffect(() => {
    const fetchHistory = async () => {
      try {
        // Fetch arbitrage tracking history
        const response = await fetch(`${apiBaseUrl}/api/arbitrage-history`);
        if (response.ok) {
          const result = await response.json();
          setData({
            active: result?.active || [],
            completed: result?.completed || []
          });
        }
        
        // Fetch auto-trade history
        const autoTradeResponse = await fetch(`${apiBaseUrl}/api/auto-trade/history?limit=50`);
        if (autoTradeResponse.ok) {
          const result = await autoTradeResponse.json();
          setAutoTradeRecords(result?.records || []);
        }
      } catch (error) {
        console.error('Failed to fetch history:', error);
      } finally {
        setLoading(false);
      }
    };

    fetchHistory();
    // 每 5 秒刷新一次
    const interval = setInterval(fetchHistory, 5000);
    return () => clearInterval(interval);
  }, [apiBaseUrl]);

  if (loading) {
    return (
      <div className="p-4 text-center text-[--text-muted] h-full flex flex-col justify-center items-center">
        <div className="text-xl mb-2">⏳</div>
        <div className="text-xs">Loading...</div>
      </div>
    );
  }

  const formatDuration = (seconds?: number) => {
    if (!seconds) return '-';
    if (seconds < 60) return `${Math.round(seconds)}s`;
    if (seconds < 3600) return `${Math.round(seconds / 60)}m ${Math.round(seconds % 60)}s`;
    return `${Math.floor(seconds / 3600)}h ${Math.round((seconds % 3600) / 60)}m`;
  };

  const formatTime = (timeStr: string) => {
    const date = new Date(timeStr);
    return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  };

  const formatDate = (timeStr: string) => {
    const date = new Date(timeStr);
    return date.toLocaleDateString('zh-CN', { month: '2-digit', day: '2-digit' });
  };

  // 获取下单状态样式
  const getTradeStatus = (record: AutoTradeRecord) => {
    // 优先检查 status 字段
    if (record.status === 'skipped') {
      return { text: '已跳过', color: 'text-gray-400', bg: 'bg-gray-500/20', icon: '⏭️' };
    }
    if (record.status === 'executed' || (record.kalshi_success && record.polymarket_success)) {
      return { text: '成功', color: 'text-green-400', bg: 'bg-green-500/20', icon: '✅' };
    } else if (record.status === 'partial' || (record.kalshi_success !== record.polymarket_success)) {
      return { text: '部分成功', color: 'text-yellow-400', bg: 'bg-yellow-500/20', icon: '⚠️' };
    } else {
      return { text: '失败', color: 'text-red-400', bg: 'bg-red-500/20', icon: '❌' };
    }
  };

  return (
    <div className="h-full flex flex-col p-2 space-y-2 overflow-hidden">
      {/* 标题/工具栏 */}
      <div className="flex justify-between items-center flex-shrink-0 pb-1 border-b border-[--border-color]">
        <div className="flex items-center gap-2">
          <button
            onClick={() => setActiveTab('tracking')}
            className={`text-[10px] px-2 py-0.5 rounded transition-colors ${
              activeTab === 'tracking'
                ? 'bg-[--accent-blue]/20 text-[--accent-blue]'
                : 'text-[--text-muted] hover:text-[--text-secondary]'
            }`}
          >
            📊 追踪记录
          </button>
          <button
            onClick={() => setActiveTab('trades')}
            className={`text-[10px] px-2 py-0.5 rounded transition-colors flex items-center gap-1 ${
              activeTab === 'trades'
                ? 'bg-[--accent-green]/20 text-[--accent-green]'
                : 'text-[--text-muted] hover:text-[--text-secondary]'
            }`}
          >
            🤖 下单记录
            {autoTradeRecords.length > 0 && (
              <span className="bg-[--accent-green] text-black text-[8px] px-1 rounded-full">
                {autoTradeRecords.length}
              </span>
            )}
          </button>
        </div>
        {onOpenExplorer && activeTab === 'tracking' && (
          <button
            onClick={onOpenExplorer}
            className="text-[10px] px-2 py-0.5 bg-[--accent-purple]/10 text-[--accent-purple] rounded hover:bg-[--accent-purple]/20 transition-colors flex items-center gap-1"
          >
            <span>🔍</span> 高级搜索
          </button>
        )}
      </div>

      <div className="flex-1 overflow-y-auto space-y-3 pr-1">
        {/* 自动下单记录 */}
        {activeTab === 'trades' && (
          <div>
            {autoTradeRecords.length === 0 ? (
              <div className="text-center text-[--text-muted] py-8">
                <div className="text-2xl mb-2">🤖</div>
                <div className="text-xs">暂无自动下单记录</div>
                <div className="text-[10px] mt-1">开启自动下单后，执行记录将显示在这里</div>
              </div>
            ) : (
              <div className="space-y-1.5">
                {autoTradeRecords.map((record) => {
                  const status = getTradeStatus(record);
                  const isSkipped = record.status === 'skipped';
                  return (
                    <div
                      key={record.id}
                      className={`${status.bg} border border-[--border-color] rounded p-2 cursor-pointer hover:brightness-110 transition-all`}
                      onClick={() => setSelectedAutoTrade(record)}
                    >
                      <div className="flex justify-between items-start">
                        <div className="flex-1 min-w-0">
                          <div className="text-xs font-medium text-[--text-primary] truncate">{record.event_name}</div>
                          <div className="text-[10px] text-[--accent-yellow] truncate">{record.team_name}</div>
                        </div>
                        <div className="flex items-center gap-2 flex-shrink-0">
                          <span className={`text-xs font-bold tabular-nums ${status.color}`}>
                            {record.profit_margin.toFixed(2)}%
                          </span>
                          <span className={`text-[10px] px-1.5 py-0.5 rounded ${status.bg} ${status.color} font-semibold`}>
                            {status.icon} {status.text}
                          </span>
                        </div>
                      </div>
                      <div className="flex justify-between items-center text-[10px] text-[--text-muted] mt-1">
                        {isSkipped ? (
                          <span className="text-gray-400 truncate max-w-[180px]" title={record.skip_reason}>
                            {record.skip_reason || '未知原因'}
                          </span>
                        ) : (
                          <span>
                            K:{record.kalshi_contracts}合约 P:${record.polymarket_amount.toFixed(2)}
                            {record.total_duration_ms > 0 && (
                              <span className="text-[--text-muted] ml-1">⏱️{(record.total_duration_ms / 1000).toFixed(1)}s</span>
                            )}
                          </span>
                        )}
                        <span>{formatTime(record.created_at)}</span>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {activeTab === 'tracking' && (
          <>
        {/* 活跃套利 */}
        {data?.active?.length > 0 && (
          <div>
            <div className="text-[10px] font-medium text-[--accent-green] mb-1 flex items-center gap-1 sticky top-0 z-10 py-1">
              <span className="status-dot status-connected animate-pulse-dot w-1.5 h-1.5"></span>
              进行中 ({data.active.length})
            </div>
            <div className="space-y-1">
              {data.active.map((record, idx) => (
                <div
                  key={`active-${idx}`}
                  className="bg-[rgba(16,185,129,0.1)] border border-[--accent-green] rounded p-2 cursor-pointer hover:bg-[rgba(16,185,129,0.15)] transition-colors"
                  onClick={() => setSelectedRecord(record)}
                >
                  <div className="flex justify-between items-start">
                    <div className="text-xs font-medium text-[--text-primary] truncate">{record.event_name}</div>
                    <div className="text-[--accent-green] font-bold text-xs tabular-nums">
                      {record.max_profit_margin.toFixed(2)}%
                    </div>
                  </div>
                  <div className="flex justify-between items-center text-[10px] text-[--text-muted] mt-0.5">
                    <span className="text-[--accent-yellow] truncate max-w-[120px]">{record.team_name}</span>
                    <span>{formatTime(record.start_time)}</span>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* 历史套利 */}
        <div>
          <div className="text-[10px] font-medium text-[--text-secondary] mb-1 sticky top-0 z-10 py-1 flex justify-between items-center">
             <span>已完成 ({data?.completed?.length || 0})</span>
          </div>
          {(!data?.completed || data.completed.length === 0) ? (
            <div className="text-center text-[--text-muted] py-4">
              <div className="text-xs">暂无历史记录</div>
            </div>
          ) : (
            <div className="space-y-1">
              {data.completed.map((record, idx) => (
                <div
                  key={`completed-${idx}`}
                  className="bg-[--bg-tertiary] rounded p-2 cursor-pointer hover:bg-[--bg-secondary] transition-colors border border-transparent hover:border-[--border-color]"
                  onClick={() => setSelectedRecord(record)}
                >
                  <div className="flex justify-between items-start">
                    <div className="text-xs font-medium text-[--text-primary] truncate pr-2">{record.event_name}</div>
                    <div className="text-[--accent-yellow] font-bold text-xs tabular-nums whitespace-nowrap">
                      {record.max_profit_margin.toFixed(2)}%
                    </div>
                  </div>
                  <div className="flex justify-between items-center text-[10px] text-[--text-muted] mt-0.5">
                    <span className="text-[--accent-yellow] truncate max-w-[100px]">{record.team_name}</span>
                    <span className="whitespace-nowrap">
                      {formatTime(record.start_time)} • {formatDuration(record.duration_seconds)}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
        </>
        )}
      </div>

      {/* 详情弹窗 */}
      {selectedRecord && (
        <div 
          className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 backdrop-blur-sm"
          onClick={() => setSelectedRecord(null)}
        >
          <div 
            className="bg-[--bg-secondary] rounded-lg p-4 max-w-lg w-full mx-4 max-h-[80vh] overflow-y-auto border border-[--border-color] shadow-xl"
            onClick={e => e.stopPropagation()}
          >
            <div className="flex justify-between items-start mb-4 pb-2 border-b border-[--border-color]">
              <div>
                <h3 className="text-base font-semibold text-[--text-primary]">{selectedRecord.event_name}</h3>
                <span className="text-[--accent-yellow] text-xs">{selectedRecord.team_name}</span>
              </div>
              <button 
                className="text-[--text-muted] hover:text-[--text-primary] text-lg"
                onClick={() => setSelectedRecord(null)}
              >
                ✕
              </button>
            </div>

            <div className="grid grid-cols-2 gap-3 mb-4">
              <div className="bg-[--bg-tertiary] rounded p-2.5">
                <div className="text-[10px] text-[--text-muted] mb-1">Max Profit</div>
                <div className="text-lg font-bold text-[--accent-green] tabular-nums">
                  {selectedRecord.max_profit_margin.toFixed(2)}%
                </div>
              </div>
              <div className="bg-[--bg-tertiary] rounded p-2.5">
                <div className="text-[10px] text-[--text-muted] mb-1">Duration</div>
                <div className="text-lg font-bold text-[--text-primary] tabular-nums">
                  {formatDuration(selectedRecord.duration_seconds)}
                </div>
              </div>
            </div>

            <div className="space-y-1.5 text-xs">
              <div className="flex justify-between">
                <span className="text-[--text-muted]">Start Time</span>
                <span className="text-[--text-primary]">{formatDate(selectedRecord.start_time)} {formatTime(selectedRecord.start_time)}</span>
              </div>
              {selectedRecord.end_time && (
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">End Time</span>
                  <span className="text-[--text-primary]">{formatDate(selectedRecord.end_time)} {formatTime(selectedRecord.end_time)}</span>
                </div>
              )}
              {selectedRecord.max_profit_time && (
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">Peak Time</span>
                  <span className="text-[--accent-green]">{formatTime(selectedRecord.max_profit_time)}</span>
                </div>
              )}
            </div>

            {/* 利润历史图表（简化版） */}
            {selectedRecord.profit_history && selectedRecord.profit_history.length > 0 && (
              <div className="mt-4 pt-3 border-t border-[--border-color]">
                <div className="text-[10px] text-[--text-muted] mb-2">Profit History ({selectedRecord.profit_history.length} points)</div>
                <div className="bg-[--bg-tertiary] rounded p-2 h-24 flex items-end gap-px">
                  {selectedRecord.profit_history.slice(-50).map((entry, i) => {
                    const maxProfit = Math.max(...selectedRecord.profit_history!.map(e => e.profit_margin));
                    const height = (entry.profit_margin / maxProfit) * 100;
                    return (
                      <div
                        key={i}
                        className="flex-1 bg-[--accent-green] min-w-[2px] rounded-t transition-all hover:bg-[--accent-yellow]"
                        style={{ height: `${Math.max(height, 5)}%` }}
                        title={`${entry.profit_margin.toFixed(2)}% at ${formatTime(entry.time)}`}
                      />
                    );
                  })}
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {/* 自动下单详情弹窗 */}
      {selectedAutoTrade && (
        <div 
          className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 backdrop-blur-sm"
          onClick={() => setSelectedAutoTrade(null)}
        >
          <div 
            className="bg-[--bg-secondary] rounded-lg p-4 max-w-lg w-full mx-4 max-h-[80vh] overflow-y-auto border border-[--border-color] shadow-xl"
            onClick={e => e.stopPropagation()}
          >
            <div className="flex justify-between items-start mb-4 pb-2 border-b border-[--border-color]">
              <div>
                <div className="flex items-center gap-2">
                  <span className="text-base">🤖</span>
                  <h3 className="text-base font-semibold text-[--text-primary]">自动下单详情</h3>
                </div>
                <div className="text-xs text-[--text-muted] mt-1">{selectedAutoTrade.event_name}</div>
                <div className="text-[--accent-yellow] text-xs">{selectedAutoTrade.team_name}</div>
              </div>
              <button 
                className="text-[--text-muted] hover:text-[--text-primary] text-lg"
                onClick={() => setSelectedAutoTrade(null)}
              >
                ✕
              </button>
            </div>

            {/* 状态和利润 */}
            <div className="grid grid-cols-2 gap-3 mb-4">
              <div className={`rounded p-2.5 ${getTradeStatus(selectedAutoTrade).bg}`}>
                <div className="text-[10px] text-[--text-muted] mb-1">下单状态</div>
                <div className={`text-lg font-bold ${getTradeStatus(selectedAutoTrade).color}`}>
                  {getTradeStatus(selectedAutoTrade).icon} {getTradeStatus(selectedAutoTrade).text}
                </div>
              </div>
              <div className="bg-[--bg-tertiary] rounded p-2.5">
                <div className="text-[10px] text-[--text-muted] mb-1">利润率</div>
                <div className="text-lg font-bold text-[--accent-green] tabular-nums">
                  {selectedAutoTrade.profit_margin.toFixed(2)}%
                </div>
              </div>
            </div>

            {/* 跳过原因（如果有） */}
            {selectedAutoTrade.status === 'skipped' && selectedAutoTrade.skip_reason && (
              <div className="mb-4 p-3 bg-gray-500/10 border border-gray-500/30 rounded">
                <div className="text-[10px] text-gray-400 mb-1">⚠️ 跳过原因</div>
                <div className="text-sm text-gray-300">{selectedAutoTrade.skip_reason}</div>
              </div>
            )}

            {/* Kalshi 订单详情 */}
            <div className="mb-3 p-2.5 bg-[--bg-tertiary] rounded">
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs font-semibold text-blue-400">Kalshi</span>
                <span className={`text-[10px] px-1.5 py-0.5 rounded ${selectedAutoTrade.kalshi_success ? 'bg-green-500/20 text-green-400' : 'bg-red-500/20 text-red-400'}`}>
                  {selectedAutoTrade.kalshi_success ? '成功' : '失败'}
                </span>
              </div>
              <div className="space-y-1 text-xs">
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">方向</span>
                  <span className="text-[--text-primary]">{selectedAutoTrade.kalshi_side.toUpperCase()}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">合约数量</span>
                  <span className="text-[--text-primary] tabular-nums">{selectedAutoTrade.kalshi_contracts}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">价格</span>
                  <span className="text-[--text-primary] tabular-nums">{(selectedAutoTrade.kalshi_price * 100).toFixed(0)}¢</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">手续费</span>
                  <span className="text-[--text-primary] tabular-nums">${selectedAutoTrade.kalshi_fee.toFixed(2)}</span>
                </div>
                {selectedAutoTrade.kalshi_order_id && (
                  <div className="flex justify-between">
                    <span className="text-[--text-muted]">订单ID</span>
                    <span className="text-[--text-primary] text-[10px] font-mono truncate max-w-[150px]">{selectedAutoTrade.kalshi_order_id}</span>
                  </div>
                )}
                {selectedAutoTrade.kalshi_error && (
                  <div className="text-red-400 text-[10px] mt-1 p-1 bg-red-500/10 rounded">{selectedAutoTrade.kalshi_error}</div>
                )}
              </div>
            </div>

            {/* Polymarket 订单详情 */}
            <div className="mb-3 p-2.5 bg-[--bg-tertiary] rounded">
              <div className="flex items-center justify-between mb-2">
                <span className="text-xs font-semibold text-purple-400">Polymarket</span>
                <span className={`text-[10px] px-1.5 py-0.5 rounded ${selectedAutoTrade.polymarket_success ? 'bg-green-500/20 text-green-400' : 'bg-red-500/20 text-red-400'}`}>
                  {selectedAutoTrade.polymarket_success ? '成功' : '失败'}
                </span>
              </div>
              <div className="space-y-1 text-xs">
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">方向</span>
                  <span className="text-[--text-primary]">{selectedAutoTrade.polymarket_side.toUpperCase()}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">金额</span>
                  <span className="text-[--text-primary] tabular-nums">${selectedAutoTrade.polymarket_amount.toFixed(4)}</span>
                </div>
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">价格</span>
                  <span className="text-[--text-primary] tabular-nums">{(selectedAutoTrade.polymarket_price * 100).toFixed(2)}¢</span>
                </div>
                {selectedAutoTrade.polymarket_order_id && (
                  <div className="flex justify-between">
                    <span className="text-[--text-muted]">订单ID</span>
                    <span className="text-[--text-primary] text-[10px] font-mono truncate max-w-[150px]">{selectedAutoTrade.polymarket_order_id}</span>
                  </div>
                )}
                {selectedAutoTrade.polymarket_error && (
                  <div className="text-red-400 text-[10px] mt-1 p-1 bg-red-500/10 rounded">{selectedAutoTrade.polymarket_error}</div>
                )}
              </div>
            </div>

            {/* 其他信息 */}
            <div className="space-y-1.5 text-xs border-t border-[--border-color] pt-3">
              <div className="flex justify-between">
                <span className="text-[--text-muted]">总金额</span>
                <span className="text-[--text-primary] tabular-nums">${selectedAutoTrade.total_amount.toFixed(2)}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[--text-muted]">机会持续时间</span>
                <span className="text-[--text-primary] tabular-nums">{(selectedAutoTrade.duration_ms / 1000).toFixed(1)}s</span>
              </div>
              {selectedAutoTrade.total_duration_ms > 0 && (
                <>
                  <div className="flex justify-between">
                    <span className="text-[--text-muted]">下单总耗时</span>
                    <span className="text-[--text-primary] tabular-nums">{(selectedAutoTrade.total_duration_ms / 1000).toFixed(1)}s</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-[--text-muted]">API执行时间</span>
                    <span className="text-[--text-primary] tabular-nums">{selectedAutoTrade.total_duration_ms - selectedAutoTrade.duration_ms}ms</span>
                  </div>
                </>
              )}
              <div className="flex justify-between">
                <span className="text-[--text-muted]">执行时间</span>
                <span className="text-[--text-primary]">{formatDate(selectedAutoTrade.created_at)} {formatTime(selectedAutoTrade.created_at)}</span>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
