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
  const [loading, setLoading] = useState(true);
  const [selectedRecord, setSelectedRecord] = useState<ArbitrageRecord | null>(null);

  useEffect(() => {
    const fetchHistory = async () => {
      try {
        const response = await fetch(`${apiBaseUrl}/api/arbitrage-history`);
        if (response.ok) {
          const result = await response.json();
          setData({
            active: result?.active || [],
            completed: result?.completed || []
          });
        }
      } catch (error) {
        console.error('Failed to fetch arbitrage history:', error);
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
      <div className="p-4 text-center text-[--text-muted]">
        <div className="text-2xl mb-2">⏳</div>
        Loading history...
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

  return (
    <div className="p-4 space-y-4">
      {/* 高级搜索按钮 */}
      {onOpenExplorer && (
        <button
          onClick={onOpenExplorer}
          className="w-full py-2 px-4 bg-gradient-to-r from-[--accent-purple]/20 to-[--accent-purple]/10 border border-[--accent-purple]/30 rounded-lg text-sm text-[--accent-purple] hover:from-[--accent-purple]/30 hover:to-[--accent-purple]/20 transition-all flex items-center justify-center gap-2"
        >
          <span>🔍</span>
          <span>高级搜索与统计分析</span>
        </button>
      )}

      {/* 活跃套利 */}
      {data?.active?.length > 0 && (
        <div>
          <h3 className="text-sm font-semibold text-[--accent-green] mb-2 flex items-center gap-2">
            <span className="status-dot status-connected animate-pulse-dot"></span>
            Active Arbitrage ({data.active.length})
          </h3>
          <div className="space-y-2">
            {data.active.map((record, idx) => (
              <div
                key={`active-${idx}`}
                className="bg-[rgba(16,185,129,0.1)] border border-[--accent-green] rounded p-3 cursor-pointer hover:bg-[rgba(16,185,129,0.15)] transition-colors"
                onClick={() => setSelectedRecord(record)}
              >
                <div className="flex justify-between items-start mb-1">
                  <div className="text-sm font-medium text-[--text-primary]">{record.event_name}</div>
                  <div className="text-[--accent-green] font-bold tabular-nums">
                    {record.max_profit_margin.toFixed(2)}%
                  </div>
                </div>
                <div className="flex justify-between items-center text-xs text-[--text-muted]">
                  <span className="text-[--accent-yellow]">{record.team_name}</span>
                  <span>Started: {formatTime(record.start_time)}</span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* 历史套利 */}
      <div>
        <h3 className="text-sm font-semibold text-[--text-secondary] mb-2 flex items-center gap-2">
          📜 History ({data?.completed?.length || 0})
        </h3>
        {(!data?.completed || data.completed.length === 0) ? (
          <div className="text-center text-[--text-muted] py-4">
            <div className="text-2xl mb-1">📭</div>
            <div className="text-xs">No historical records yet</div>
          </div>
        ) : (
          <div className="space-y-2 max-h-96 overflow-y-auto">
            {data.completed.map((record, idx) => (
              <div
                key={`completed-${idx}`}
                className="bg-[--bg-tertiary] rounded p-3 cursor-pointer hover:bg-[--bg-secondary] transition-colors border border-transparent hover:border-[--border-color]"
                onClick={() => setSelectedRecord(record)}
              >
                <div className="flex justify-between items-start mb-1">
                  <div className="text-sm font-medium text-[--text-primary]">{record.event_name}</div>
                  <div className="text-[--accent-yellow] font-bold tabular-nums">
                    Max: {record.max_profit_margin.toFixed(2)}%
                  </div>
                </div>
                <div className="flex justify-between items-center text-xs text-[--text-muted]">
                  <span className="text-[--accent-yellow]">{record.team_name}</span>
                  <span>
                    {formatDate(record.start_time)} {formatTime(record.start_time)} • {formatDuration(record.duration_seconds)}
                  </span>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* 详情弹窗 */}
      {selectedRecord && (
        <div 
          className="fixed inset-0 bg-black/50 flex items-center justify-center z-50"
          onClick={() => setSelectedRecord(null)}
        >
          <div 
            className="bg-[--bg-secondary] rounded-lg p-6 max-w-lg w-full mx-4 max-h-[80vh] overflow-y-auto"
            onClick={e => e.stopPropagation()}
          >
            <div className="flex justify-between items-start mb-4">
              <div>
                <h3 className="text-lg font-semibold text-[--text-primary]">{selectedRecord.event_name}</h3>
                <span className="text-[--accent-yellow] text-sm">{selectedRecord.team_name}</span>
              </div>
              <button 
                className="text-[--text-muted] hover:text-[--text-primary]"
                onClick={() => setSelectedRecord(null)}
              >
                ✕
              </button>
            </div>

            <div className="grid grid-cols-2 gap-4 mb-4">
              <div className="bg-[--bg-tertiary] rounded p-3">
                <div className="text-xs text-[--text-muted] mb-1">Max Profit</div>
                <div className="text-xl font-bold text-[--accent-green] tabular-nums">
                  {selectedRecord.max_profit_margin.toFixed(2)}%
                </div>
              </div>
              <div className="bg-[--bg-tertiary] rounded p-3">
                <div className="text-xs text-[--text-muted] mb-1">Duration</div>
                <div className="text-xl font-bold text-[--text-primary] tabular-nums">
                  {formatDuration(selectedRecord.duration_seconds)}
                </div>
              </div>
            </div>

            <div className="space-y-2 text-sm">
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
              <div className="mt-4">
                <div className="text-xs text-[--text-muted] mb-2">Profit History ({selectedRecord.profit_history.length} points)</div>
                <div className="bg-[--bg-tertiary] rounded p-3 h-32 flex items-end gap-px">
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
    </div>
  );
}
