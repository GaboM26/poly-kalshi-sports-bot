import { useEffect, useState, useCallback } from 'react';

interface ProfitHistoryEntry {
  time: string;
  profit_margin: number;
  kalshi_price: number;
  polymarket_price: number;
}

interface HistoryRecord {
  id: number;
  event_name: string;
  team_name: string;
  kalshi_market_id: string;
  polymarket_market_id: string;
  kalshi_side: string;  // yes 或 no
  polymarket_side: string;  // yes 或 no
  start_time: string;
  end_time?: string;
  duration_seconds?: number;
  duration_ms?: number;  // 毫秒级持续时间
  max_profit_margin: number;
  max_profit_time?: string;
  profit_history?: ProfitHistoryEntry[];
  // 深度信息
  poly_ask_depth?: number;  // Polymarket ask 深度 (USD)
  kalshi_ask_depth?: number;  // Kalshi ask 深度 (contracts)
  // 价格信息
  kalshi_ask_price?: number;  // Kalshi ask 价格
  polymarket_ask_price?: number;  // Polymarket ask 价格
}

interface SearchResult {
  records: HistoryRecord[];
  total: number;
  limit: number;
  offset: number;
  has_more: boolean;
}

interface Statistics {
  total_records: number;
  avg_profit: number;
  max_profit: number;
  min_profit: number;
  avg_duration: number;
  avg_duration_ms: number;
  max_duration: number;
  max_duration_ms: number;
  min_duration: number;
  min_duration_ms: number;
  total_duration: number;
  duration_percentiles: {
    p50: number;
    p75: number;
    p90: number;
    p95: number;
    p99: number;
  };
  top_events: { event_name: string; count: number; avg_profit: number }[];
  top_teams: { team_name: string; count: number; avg_profit: number }[];
  profit_distribution: { range: string; count: number }[];
  duration_distribution: { range: string; count: number; avg_profit: number }[];
}

interface HistoryExplorerProps {
  apiBaseUrl: string;
  onClose: () => void;
}

export function HistoryExplorer({ apiBaseUrl, onClose }: HistoryExplorerProps) {
  // 筛选条件
  const [minProfit, setMinProfit] = useState<string>('');
  const [maxProfit, setMaxProfit] = useState<string>('');
  const [minDuration, setMinDuration] = useState<string>('');
  const [maxDuration, setMaxDuration] = useState<string>('');
  const [durationUnit, setDurationUnit] = useState<'s' | 'ms'>('s'); // 秒或毫秒
  const [eventName, setEventName] = useState('');
  const [teamName, setTeamName] = useState('');
  const [sortBy, setSortBy] = useState('start_time');
  const [sortOrder, setSortOrder] = useState('desc');
  
  // 分页
  const [page, setPage] = useState(1);
  const pageSize = 20;
  
  // 数据
  const [result, setResult] = useState<SearchResult | null>(null);
  const [stats, setStats] = useState<Statistics | null>(null);
  const [loading, setLoading] = useState(true);
  const [selectedRecord, setSelectedRecord] = useState<HistoryRecord | null>(null);
  const [activeTab, setActiveTab] = useState<'list' | 'stats'>('list');

  // 搜索函数
  const search = useCallback(async () => {
    setLoading(true);
    try {
      const params = new URLSearchParams();
      if (minProfit) params.set('min_profit', minProfit);
      if (maxProfit) params.set('max_profit', maxProfit);
      
      // 根据单位转换持续时间（API 使用秒）
      if (minDuration) {
        const minDurationSeconds = durationUnit === 'ms' 
          ? parseFloat(minDuration) / 1000 
          : parseFloat(minDuration);
        params.set('min_duration', minDurationSeconds.toString());
      }
      if (maxDuration) {
        const maxDurationSeconds = durationUnit === 'ms' 
          ? parseFloat(maxDuration) / 1000 
          : parseFloat(maxDuration);
        params.set('max_duration', maxDurationSeconds.toString());
      }
      
      if (eventName) params.set('event_name', eventName);
      if (teamName) params.set('team_name', teamName);
      params.set('sort_by', sortBy);
      params.set('sort_order', sortOrder);
      params.set('limit', pageSize.toString());
      params.set('offset', ((page - 1) * pageSize).toString());
      params.set('include_history', 'true');
      
      const response = await fetch(`${apiBaseUrl}/api/history/search?${params}`);
      if (response.ok) {
        const data = await response.json();
        setResult(data);
      }
    } catch (error) {
      console.error('Search failed:', error);
    } finally {
      setLoading(false);
    }
  }, [apiBaseUrl, minProfit, maxProfit, minDuration, maxDuration, durationUnit, eventName, teamName, sortBy, sortOrder, page]);

  // 获取统计信息
  const fetchStats = useCallback(async () => {
    try {
      const response = await fetch(`${apiBaseUrl}/api/history/statistics`);
      if (response.ok) {
        const data = await response.json();
        setStats(data);
      }
    } catch (error) {
      console.error('Failed to fetch stats:', error);
    }
  }, [apiBaseUrl]);

  useEffect(() => {
    search();
    fetchStats();
  }, [search, fetchStats]);

  // 重置筛选
  const resetFilters = () => {
    setMinProfit('');
    setMaxProfit('');
    setMinDuration('');
    setMaxDuration('');
    setDurationUnit('s');
    setEventName('');
    setTeamName('');
    setSortBy('start_time');
    setSortOrder('desc');
    setPage(1);
  };

  const formatDuration = (seconds?: number) => {
    if (!seconds) return '-';
    if (seconds < 60) return `${Math.round(seconds)}s`;
    if (seconds < 3600) return `${Math.round(seconds / 60)}m ${Math.round(seconds % 60)}s`;
    return `${Math.floor(seconds / 3600)}h ${Math.round((seconds % 3600) / 60)}m`;
  };

  const formatDurationMs = (ms?: number) => {
    if (!ms && ms !== 0) return '-';
    if (ms < 1000) return `${Math.round(ms)}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
    if (ms < 3600000) return `${Math.floor(ms / 60000)}m ${Math.round((ms % 60000) / 1000)}s`;
    return `${Math.floor(ms / 3600000)}h ${Math.round((ms % 3600000) / 60000)}m`;
  };

  const formatTime = (timeStr: string) => {
    const date = new Date(timeStr);
    return date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  };

  const formatDate = (timeStr: string) => {
    const date = new Date(timeStr);
    return date.toLocaleDateString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit' });
  };

  const totalPages = result ? Math.ceil(result.total / pageSize) : 0;

  return (
    <div className="fixed inset-0 bg-black/80 flex items-center justify-center z-50 p-4">
      <div className="bg-[--bg-primary] rounded-xl w-full max-w-6xl max-h-[90vh] flex flex-col shadow-2xl border border-[--border-color]">
        {/* 头部 */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-[--border-color]">
          <div className="flex items-center gap-4">
            <h2 className="text-xl font-bold text-[--text-primary]">📊 历史套利探索</h2>
            {stats && (
              <span className="text-sm text-[--text-muted]">
                共 {stats.total_records} 条记录
              </span>
            )}
          </div>
          <button
            onClick={onClose}
            className="text-[--text-muted] hover:text-[--text-primary] text-xl"
          >
            ✕
          </button>
        </div>

        {/* 标签页 */}
        <div className="flex border-b border-[--border-color]">
          <button
            className={`px-6 py-3 text-sm font-medium transition-colors ${
              activeTab === 'list'
                ? 'text-[--accent-purple] border-b-2 border-[--accent-purple] bg-[--bg-secondary]'
                : 'text-[--text-muted] hover:text-[--text-secondary]'
            }`}
            onClick={() => setActiveTab('list')}
          >
            📋 记录列表
          </button>
          <button
            className={`px-6 py-3 text-sm font-medium transition-colors ${
              activeTab === 'stats'
                ? 'text-[--accent-green] border-b-2 border-[--accent-green] bg-[--bg-secondary]'
                : 'text-[--text-muted] hover:text-[--text-secondary]'
            }`}
            onClick={() => setActiveTab('stats')}
          >
            📈 统计分析
          </button>
        </div>

        {activeTab === 'list' ? (
          <>
            {/* 筛选栏 */}
            <div className="px-6 py-4 bg-[--bg-secondary] border-b border-[--border-color]">
              <div className="grid grid-cols-2 md:grid-cols-4 lg:grid-cols-6 gap-3">
                {/* 利润范围 */}
                <div>
                  <label className="block text-xs text-[--text-muted] mb-1">最小利润 %</label>
                  <input
                    type="number"
                    value={minProfit}
                    onChange={e => setMinProfit(e.target.value)}
                    placeholder="0"
                    className="w-full px-3 py-1.5 bg-[--bg-tertiary] border border-[--border-color] rounded text-sm text-[--text-primary] focus:border-[--accent-purple] focus:outline-none"
                  />
                </div>
                <div>
                  <label className="block text-xs text-[--text-muted] mb-1">最大利润 %</label>
                  <input
                    type="number"
                    value={maxProfit}
                    onChange={e => setMaxProfit(e.target.value)}
                    placeholder="100"
                    className="w-full px-3 py-1.5 bg-[--bg-tertiary] border border-[--border-color] rounded text-sm text-[--text-primary] focus:border-[--accent-purple] focus:outline-none"
                  />
                </div>
                
                {/* 持续时间 */}
                <div>
                  <label className="block text-xs text-[--text-muted] mb-1">
                    最小时长
                    <select
                      value={durationUnit}
                      onChange={e => setDurationUnit(e.target.value as 's' | 'ms')}
                      className="ml-1 bg-transparent text-[--accent-purple] cursor-pointer"
                    >
                      <option value="s">(秒)</option>
                      <option value="ms">(毫秒)</option>
                    </select>
                  </label>
                  <input
                    type="number"
                    value={minDuration}
                    onChange={e => setMinDuration(e.target.value)}
                    placeholder="0"
                    className="w-full px-3 py-1.5 bg-[--bg-tertiary] border border-[--border-color] rounded text-sm text-[--text-primary] focus:border-[--accent-purple] focus:outline-none"
                  />
                </div>
                <div>
                  <label className="block text-xs text-[--text-muted] mb-1">
                    最大时长
                    <span className="ml-1 text-[--accent-purple]">({durationUnit === 's' ? '秒' : '毫秒'})</span>
                  </label>
                  <input
                    type="number"
                    value={maxDuration}
                    onChange={e => setMaxDuration(e.target.value)}
                    placeholder="∞"
                    className="w-full px-3 py-1.5 bg-[--bg-tertiary] border border-[--border-color] rounded text-sm text-[--text-primary] focus:border-[--accent-purple] focus:outline-none"
                  />
                </div>
                
                {/* 搜索 */}
                <div>
                  <label className="block text-xs text-[--text-muted] mb-1">事件名称</label>
                  <input
                    type="text"
                    value={eventName}
                    onChange={e => setEventName(e.target.value)}
                    placeholder="搜索事件..."
                    className="w-full px-3 py-1.5 bg-[--bg-tertiary] border border-[--border-color] rounded text-sm text-[--text-primary] focus:border-[--accent-purple] focus:outline-none"
                  />
                </div>
                <div>
                  <label className="block text-xs text-[--text-muted] mb-1">队伍名称</label>
                  <input
                    type="text"
                    value={teamName}
                    onChange={e => setTeamName(e.target.value)}
                    placeholder="搜索队伍..."
                    className="w-full px-3 py-1.5 bg-[--bg-tertiary] border border-[--border-color] rounded text-sm text-[--text-primary] focus:border-[--accent-purple] focus:outline-none"
                  />
                </div>
              </div>
              
              {/* 排序和操作 */}
              <div className="flex items-center justify-between mt-3">
                <div className="flex items-center gap-3">
                  <select
                    value={sortBy}
                    onChange={e => setSortBy(e.target.value)}
                    className="px-3 py-1.5 bg-[--bg-tertiary] border border-[--border-color] rounded text-sm text-[--text-primary] focus:border-[--accent-purple] focus:outline-none"
                  >
                    <option value="start_time">按时间</option>
                    <option value="max_profit_margin">按利润</option>
                    <option value="duration">按时长</option>
                    <option value="event_name">按事件</option>
                  </select>
                  <select
                    value={sortOrder}
                    onChange={e => setSortOrder(e.target.value)}
                    className="px-3 py-1.5 bg-[--bg-tertiary] border border-[--border-color] rounded text-sm text-[--text-primary] focus:border-[--accent-purple] focus:outline-none"
                  >
                    <option value="desc">降序</option>
                    <option value="asc">升序</option>
                  </select>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={resetFilters}
                    className="px-3 py-1.5 text-sm text-[--text-muted] hover:text-[--text-primary] transition-colors"
                  >
                    重置
                  </button>
                  <button
                    onClick={() => { setPage(1); search(); }}
                    className="px-4 py-1.5 bg-[--accent-purple] text-white rounded text-sm font-medium hover:bg-[--accent-purple]/80 transition-colors"
                  >
                    搜索
                  </button>
                </div>
              </div>
            </div>

            {/* 列表内容 */}
            <div className="flex-1 overflow-y-auto">
              {loading ? (
                <div className="flex items-center justify-center py-12">
                  <div className="text-[--text-muted]">加载中...</div>
                </div>
              ) : result?.records.length === 0 ? (
                <div className="flex flex-col items-center justify-center py-12">
                  <div className="text-4xl mb-3">📭</div>
                  <div className="text-[--text-muted]">没有找到匹配的记录</div>
                </div>
              ) : (
                <table className="w-full">
                  <thead className="bg-[--bg-secondary] sticky top-0">
                    <tr className="text-xs text-[--text-muted]">
                      <th className="text-left px-4 py-3 font-medium">事件</th>
                      <th className="text-left px-4 py-3 font-medium">队伍</th>
                      <th className="text-right px-4 py-3 font-medium">最高利润</th>
                      <th className="text-right px-4 py-3 font-medium">持续时间</th>
                      <th className="text-right px-4 py-3 font-medium">Poly深度</th>
                      <th className="text-right px-4 py-3 font-medium">Kalshi深度</th>
                      <th className="text-left px-4 py-3 font-medium">开始时间</th>
                    </tr>
                  </thead>
                  <tbody>
                    {result?.records.map((record, idx) => (
                      <tr
                        key={record.id}
                        className={`border-b border-[--border-color] hover:bg-[--bg-secondary] cursor-pointer transition-colors ${
                          idx % 2 === 0 ? 'bg-[--bg-primary]' : 'bg-[--bg-secondary]/30'
                        }`}
                        onClick={() => setSelectedRecord(record)}
                      >
                        <td className="px-4 py-3">
                          <div className="text-sm text-[--text-primary] font-medium truncate max-w-[200px]">
                            {record.event_name}
                          </div>
                        </td>
                        <td className="px-4 py-3">
                          <span className="text-sm text-[--accent-yellow]">{record.team_name}</span>
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span className={`text-sm font-bold tabular-nums ${
                            record.max_profit_margin >= 5 ? 'text-[--accent-green]' : 'text-[--accent-yellow]'
                          }`}>
                            {record.max_profit_margin.toFixed(2)}%
                          </span>
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span className="text-sm text-[--text-secondary] tabular-nums" title={record.duration_ms ? `${record.duration_ms.toLocaleString()} ms` : ''}>
                            {record.duration_ms ? formatDurationMs(record.duration_ms) : formatDuration(record.duration_seconds)}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span className={`text-sm tabular-nums ${
                            (record.poly_ask_depth || 0) >= 10 ? 'text-[--accent-green]' : 'text-[--accent-red]'
                          }`}>
                            {record.poly_ask_depth ? `$${record.poly_ask_depth.toFixed(1)}` : '-'}
                          </span>
                        </td>
                        <td className="px-4 py-3 text-right">
                          <span className={`text-sm tabular-nums ${
                            (record.kalshi_ask_depth || 0) >= 10 ? 'text-[--accent-green]' : 'text-[--accent-red]'
                          }`}>
                            {record.kalshi_ask_depth ? `${record.kalshi_ask_depth}` : '-'}
                          </span>
                        </td>
                        <td className="px-4 py-3">
                          <span className="text-xs text-[--text-muted]">
                            {formatDate(record.start_time)} {formatTime(record.start_time)}
                          </span>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>

            {/* 分页 */}
            {result && result.total > pageSize && (
              <div className="flex items-center justify-between px-6 py-3 border-t border-[--border-color] bg-[--bg-secondary]">
                <div className="text-sm text-[--text-muted]">
                  显示 {(page - 1) * pageSize + 1} - {Math.min(page * pageSize, result.total)} / {result.total} 条
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => setPage(p => Math.max(1, p - 1))}
                    disabled={page === 1}
                    className="px-3 py-1 text-sm rounded bg-[--bg-tertiary] text-[--text-secondary] disabled:opacity-50 disabled:cursor-not-allowed hover:bg-[--bg-tertiary]/80"
                  >
                    上一页
                  </button>
                  <span className="text-sm text-[--text-muted]">
                    {page} / {totalPages}
                  </span>
                  <button
                    onClick={() => setPage(p => Math.min(totalPages, p + 1))}
                    disabled={page >= totalPages}
                    className="px-3 py-1 text-sm rounded bg-[--bg-tertiary] text-[--text-secondary] disabled:opacity-50 disabled:cursor-not-allowed hover:bg-[--bg-tertiary]/80"
                  >
                    下一页
                  </button>
                </div>
              </div>
            )}
          </>
        ) : (
          /* 统计分析标签页 */
          <div className="flex-1 overflow-y-auto p-6">
            {stats ? (
              <div className="space-y-6">
                {/* 概览卡片 - 利润 */}
                <div>
                  <h3 className="text-sm font-semibold text-[--text-primary] mb-3">📈 利润统计</h3>
                  <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                    <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                      <div className="text-xs text-[--text-muted] mb-1">总记录数</div>
                      <div className="text-2xl font-bold text-[--text-primary]">{stats.total_records}</div>
                    </div>
                    <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                      <div className="text-xs text-[--text-muted] mb-1">平均利润</div>
                      <div className="text-2xl font-bold text-[--accent-green]">{stats.avg_profit}%</div>
                    </div>
                    <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                      <div className="text-xs text-[--text-muted] mb-1">最高利润</div>
                      <div className="text-2xl font-bold text-[--accent-yellow]">{stats.max_profit}%</div>
                    </div>
                    <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                      <div className="text-xs text-[--text-muted] mb-1">最低利润</div>
                      <div className="text-2xl font-bold text-[--text-secondary]">{stats.min_profit}%</div>
                    </div>
                  </div>
                </div>

                {/* 概览卡片 - 持续时间 */}
                <div>
                  <h3 className="text-sm font-semibold text-[--text-primary] mb-3">⏱️ 持续时间统计</h3>
                  <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                    <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                      <div className="text-xs text-[--text-muted] mb-1">平均时长</div>
                      <div className="text-xl font-bold text-[--accent-purple]">{formatDuration(stats.avg_duration)}</div>
                      <div className="text-xs text-[--text-muted] mt-1">{stats.avg_duration_ms?.toLocaleString()} ms</div>
                    </div>
                    <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                      <div className="text-xs text-[--text-muted] mb-1">最长时长</div>
                      <div className="text-xl font-bold text-[--accent-yellow]">{formatDuration(stats.max_duration)}</div>
                      <div className="text-xs text-[--text-muted] mt-1">{stats.max_duration_ms?.toLocaleString()} ms</div>
                    </div>
                    <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                      <div className="text-xs text-[--text-muted] mb-1">最短时长</div>
                      <div className="text-xl font-bold text-[--text-secondary]">{formatDuration(stats.min_duration)}</div>
                      <div className="text-xs text-[--text-muted] mt-1">{stats.min_duration_ms?.toLocaleString()} ms</div>
                    </div>
                    <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                      <div className="text-xs text-[--text-muted] mb-1">总时长</div>
                      <div className="text-xl font-bold text-[--text-primary]">{formatDuration(stats.total_duration)}</div>
                    </div>
                  </div>
                </div>

                {/* 持续时间百分位数 */}
                {stats.duration_percentiles && (
                  <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                    <h3 className="text-sm font-semibold text-[--text-primary] mb-3">📊 持续时间百分位数</h3>
                    <div className="grid grid-cols-5 gap-3">
                      {[
                        { label: 'P50 (中位数)', value: stats.duration_percentiles.p50 },
                        { label: 'P75', value: stats.duration_percentiles.p75 },
                        { label: 'P90', value: stats.duration_percentiles.p90 },
                        { label: 'P95', value: stats.duration_percentiles.p95 },
                        { label: 'P99', value: stats.duration_percentiles.p99 },
                      ].map((p, idx) => (
                        <div key={idx} className="text-center">
                          <div className="text-xs text-[--text-muted] mb-1">{p.label}</div>
                          <div className="text-sm font-bold text-[--accent-purple]">
                            {formatDurationMs(p.value)}
                          </div>
                          <div className="text-[10px] text-[--text-muted]">
                            {p.value?.toLocaleString()} ms
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {/* 利润分布 */}
                <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                  <h3 className="text-sm font-semibold text-[--text-primary] mb-4">💰 利润分布</h3>
                  <div className="flex items-end gap-2 h-32">
                    {stats.profit_distribution.map((item, idx) => {
                      const maxCount = Math.max(...stats.profit_distribution.map(d => d.count));
                      const height = (item.count / maxCount) * 100;
                      return (
                        <div key={idx} className="flex-1 flex flex-col items-center">
                          <div className="w-full flex flex-col items-center">
                            <span className="text-xs text-[--text-muted] mb-1">{item.count}</span>
                            <div
                              className="w-full bg-gradient-to-t from-[--accent-green] to-[--accent-green]/50 rounded-t"
                              style={{ height: `${Math.max(height, 5)}%`, minHeight: '4px' }}
                            />
                          </div>
                          <span className="text-xs text-[--text-muted] mt-2">{item.range}</span>
                        </div>
                      );
                    })}
                  </div>
                </div>

                {/* 持续时间分布 */}
                {stats.duration_distribution && stats.duration_distribution.length > 0 && (
                  <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                    <h3 className="text-sm font-semibold text-[--text-primary] mb-4">⏰ 持续时间分布</h3>
                    <div className="flex items-end gap-2 h-32">
                      {stats.duration_distribution.map((item, idx) => {
                        const maxCount = Math.max(...stats.duration_distribution.map(d => d.count));
                        const height = (item.count / maxCount) * 100;
                        return (
                          <div key={idx} className="flex-1 flex flex-col items-center">
                            <div className="w-full flex flex-col items-center">
                              <span className="text-xs text-[--text-muted] mb-1">{item.count}</span>
                              <div
                                className="w-full bg-gradient-to-t from-[--accent-purple] to-[--accent-purple]/50 rounded-t cursor-pointer hover:from-[--accent-yellow] hover:to-[--accent-yellow]/50 transition-colors"
                                style={{ height: `${Math.max(height, 5)}%`, minHeight: '4px' }}
                                title={`平均利润: ${item.avg_profit?.toFixed(1)}%`}
                              />
                            </div>
                            <span className="text-[10px] text-[--text-muted] mt-2 whitespace-nowrap">{item.range}</span>
                          </div>
                        );
                      })}
                    </div>
                    <div className="text-xs text-[--text-muted] mt-3 text-center">
                      鼠标悬停查看该时长区间的平均利润
                    </div>
                  </div>
                )}

                {/* 热门事件和队伍 */}
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                    <h3 className="text-sm font-semibold text-[--text-primary] mb-3">🏆 热门事件 Top 10</h3>
                    <div className="space-y-2">
                      {stats.top_events.map((event, idx) => (
                        <div key={idx} className="flex items-center justify-between text-sm">
                          <span className="text-[--text-secondary] truncate flex-1">{event.event_name}</span>
                          <div className="flex items-center gap-3 ml-2">
                            <span className="text-[--text-muted]">{event.count}次</span>
                            <span className="text-[--accent-green] tabular-nums">{event.avg_profit?.toFixed(1)}%</span>
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                  <div className="bg-[--bg-secondary] rounded-lg p-4 border border-[--border-color]">
                    <h3 className="text-sm font-semibold text-[--text-primary] mb-3">⭐ 热门队伍 Top 10</h3>
                    <div className="space-y-2">
                      {stats.top_teams.map((team, idx) => (
                        <div key={idx} className="flex items-center justify-between text-sm">
                          <span className="text-[--accent-yellow]">{team.team_name}</span>
                          <div className="flex items-center gap-3">
                            <span className="text-[--text-muted]">{team.count}次</span>
                            <span className="text-[--accent-green] tabular-nums">{team.avg_profit?.toFixed(1)}%</span>
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                </div>
              </div>
            ) : (
              <div className="flex items-center justify-center py-12">
                <div className="text-[--text-muted]">加载统计信息中...</div>
              </div>
            )}
          </div>
        )}
      </div>

      {/* 详情弹窗 */}
      {selectedRecord && (
        <div 
          className="fixed inset-0 bg-black/60 flex items-center justify-center z-[60]"
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
                <div className="text-xs text-[--text-muted] mb-1">最高利润</div>
                <div className="text-xl font-bold text-[--accent-green] tabular-nums">
                  {selectedRecord.max_profit_margin.toFixed(2)}%
                </div>
              </div>
              <div className="bg-[--bg-tertiary] rounded p-3">
                <div className="text-xs text-[--text-muted] mb-1">持续时间</div>
                <div className="text-xl font-bold text-[--text-primary] tabular-nums">
                  {selectedRecord.duration_ms ? formatDurationMs(selectedRecord.duration_ms) : formatDuration(selectedRecord.duration_seconds)}
                </div>
                {selectedRecord.duration_ms && (
                  <div className="text-xs text-[--text-muted] mt-1">
                    {selectedRecord.duration_ms.toLocaleString()} 毫秒
                  </div>
                )}
              </div>
            </div>

            {/* 深度和价格信息 */}
            <div className="grid grid-cols-2 gap-4 mb-4">
              <div className="bg-[--bg-tertiary] rounded p-3">
                <div className="text-xs text-[--text-muted] mb-1">Polymarket 深度</div>
                <div className={`text-lg font-bold tabular-nums ${
                  (selectedRecord.poly_ask_depth || 0) >= 10 ? 'text-[--accent-green]' : 'text-[--accent-red]'
                }`}>
                  {selectedRecord.poly_ask_depth ? `$${selectedRecord.poly_ask_depth.toFixed(2)}` : '-'}
                </div>
                <div className="text-xs text-[--text-muted] mt-1">USD 可用买入深度</div>
              </div>
              <div className="bg-[--bg-tertiary] rounded p-3">
                <div className="text-xs text-[--text-muted] mb-1">Kalshi 深度</div>
                <div className={`text-lg font-bold tabular-nums ${
                  (selectedRecord.kalshi_ask_depth || 0) >= 10 ? 'text-[--accent-green]' : 'text-[--accent-red]'
                }`}>
                  {selectedRecord.kalshi_ask_depth ?? '-'}
                </div>
                <div className="text-xs text-[--text-muted] mt-1">合约数量</div>
              </div>
            </div>

            {/* 价格信息 */}
            <div className="grid grid-cols-2 gap-4 mb-4">
              <div className="bg-[--bg-tertiary] rounded p-3">
                <div className="text-xs text-[--text-muted] mb-1">Polymarket 价格</div>
                <div className="text-lg font-bold text-[--accent-purple] tabular-nums">
                  {selectedRecord.polymarket_ask_price ? `${(selectedRecord.polymarket_ask_price * 100).toFixed(1)}¢` : '-'}
                </div>
                <div className="text-xs text-[--text-muted] mt-1">
                  {selectedRecord.polymarket_side ? `买入 ${selectedRecord.polymarket_side.toUpperCase()}` : 'Ask 价格'}
                </div>
              </div>
              <div className="bg-[--bg-tertiary] rounded p-3">
                <div className="text-xs text-[--text-muted] mb-1">Kalshi 价格</div>
                <div className="text-lg font-bold text-[--accent-blue] tabular-nums">
                  {selectedRecord.kalshi_ask_price ? `${(selectedRecord.kalshi_ask_price * 100).toFixed(1)}¢` : '-'}
                </div>
                <div className="text-xs text-[--text-muted] mt-1">
                  {selectedRecord.kalshi_side ? `买入 ${selectedRecord.kalshi_side.toUpperCase()}` : 'Ask 价格'}
                </div>
              </div>
            </div>

            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-[--text-muted]">开始时间</span>
                <span className="text-[--text-primary]">{formatDate(selectedRecord.start_time)} {formatTime(selectedRecord.start_time)}</span>
              </div>
              {selectedRecord.end_time && (
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">结束时间</span>
                  <span className="text-[--text-primary]">{formatDate(selectedRecord.end_time)} {formatTime(selectedRecord.end_time)}</span>
                </div>
              )}
              {selectedRecord.max_profit_time && (
                <div className="flex justify-between">
                  <span className="text-[--text-muted]">峰值时间</span>
                  <span className="text-[--accent-green]">{formatTime(selectedRecord.max_profit_time)}</span>
                </div>
              )}
            </div>

            {/* 利润历史图表 */}
            {selectedRecord.profit_history && selectedRecord.profit_history.length > 0 && (
              <div className="mt-4">
                <div className="text-xs text-[--text-muted] mb-2">利润历史 ({selectedRecord.profit_history.length} 个点)</div>
                <div className="bg-[--bg-tertiary] rounded p-3 h-32 flex items-end gap-px">
                  {selectedRecord.profit_history.slice(-100).map((entry, i) => {
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
