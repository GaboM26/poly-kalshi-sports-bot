import { useState, useEffect } from 'react';
import { TrackingStats, TrackingRecord, ActiveTracking } from '../types';

interface TrackingPanelProps {
  apiBaseUrl: string;
}

export function TrackingPanel({ apiBaseUrl }: TrackingPanelProps) {
  const [trackingStats, setTrackingStats] = useState<TrackingStats | null>(null);
  const [selectedRecord, setSelectedRecord] = useState<TrackingRecord | null>(null);
  const [activeTab, setActiveTab] = useState<'active' | 'completed'>('active');

  // 定期获取追踪数据
  useEffect(() => {
    const fetchTracking = async () => {
      try {
        const response = await fetch(`${apiBaseUrl}/api/tracking`);
        if (response.ok) {
          const data = await response.json();
          setTrackingStats(data);
        }
      } catch (error) {
        console.error('获取追踪数据失败:', error);
      }
    };

    fetchTracking();
    const interval = setInterval(fetchTracking, 2000); // 每2秒刷新
    return () => clearInterval(interval);
  }, [apiBaseUrl]);

  if (!trackingStats) {
    return (
      <div className="p-4 text-center text-[--text-muted]">
        加载追踪数据...
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      {/* 标签页切换 */}
      <div className="flex border-b border-[--border-color]">
        <button
          className={`flex-1 px-4 py-2 text-sm font-medium transition-colors ${
            activeTab === 'active'
              ? 'text-[--accent-green] border-b-2 border-[--accent-green]'
              : 'text-[--text-muted] hover:text-[--text-secondary]'
          }`}
          onClick={() => setActiveTab('active')}
        >
          🎯 追踪中 ({trackingStats.active_count})
        </button>
        <button
          className={`flex-1 px-4 py-2 text-sm font-medium transition-colors ${
            activeTab === 'completed'
              ? 'text-[--accent-blue] border-b-2 border-[--accent-blue]'
              : 'text-[--text-muted] hover:text-[--text-secondary]'
          }`}
          onClick={() => setActiveTab('completed')}
        >
          📊 历史 ({trackingStats.completed_count})
        </button>
      </div>

      {/* 内容区域 */}
      <div className="flex-1 overflow-y-auto p-3">
        {activeTab === 'active' ? (
          <ActiveTrackingList 
            items={trackingStats.active} 
          />
        ) : (
          <CompletedTrackingList 
            items={trackingStats.recent_completed}
            onSelect={setSelectedRecord}
            selectedId={selectedRecord ? `${selectedRecord.event_name}_${selectedRecord.team_name}_${selectedRecord.start_time}` : null}
          />
        )}
      </div>

      {/* 选中的记录详情 */}
      {selectedRecord && activeTab === 'completed' && (
        <RecordDetail 
          record={selectedRecord} 
          onClose={() => setSelectedRecord(null)}
        />
      )}
    </div>
  );
}

// 追踪中列表
function ActiveTrackingList({ items }: { items: ActiveTracking[] }) {
  if (items.length === 0) {
    return (
      <div className="text-center py-8">
        <div className="text-3xl mb-2">😴</div>
        <div className="text-[--text-muted] text-sm">暂无正在追踪的套利机会</div>
        <div className="text-[--text-muted] text-xs mt-1">当利润超过3%时开始追踪</div>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {items.map((item, index) => (
        <div 
          key={`${item.event_name}_${item.team_name}_${index}`}
          className="bg-[--bg-tertiary] rounded-lg p-3 border-l-4 border-[--accent-green] animate-pulse"
        >
          <div className="flex justify-between items-start mb-2">
            <div>
              <div className="text-sm font-medium text-[--text-primary]">
                {item.event_name}
              </div>
              <div className="text-xs text-[--accent-yellow]">{item.team_name}</div>
            </div>
            <div className="text-right">
              <div className="text-lg font-bold text-[--accent-green]">
                {item.max_profit_margin.toFixed(2)}%
              </div>
              <div className="text-xs text-[--text-muted]">最高利润</div>
            </div>
          </div>
          <div className="flex justify-between text-xs text-[--text-muted]">
            <span>⏱ 已持续 {formatDuration(item.duration_seconds)}</span>
            <span>开始: {formatTime(item.start_time)}</span>
          </div>
        </div>
      ))}
    </div>
  );
}

// 已完成列表
function CompletedTrackingList({ 
  items, 
  onSelect,
  selectedId 
}: { 
  items: TrackingRecord[];
  onSelect: (record: TrackingRecord) => void;
  selectedId: string | null;
}) {
  if (items.length === 0) {
    return (
      <div className="text-center py-8">
        <div className="text-3xl mb-2">📭</div>
        <div className="text-[--text-muted] text-sm">暂无历史追踪记录</div>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {items.slice().reverse().map((item) => {
        const itemId = `${item.event_name}_${item.team_name}_${item.start_time}`;
        const isSelected = selectedId === itemId;
        
        return (
          <div 
            key={itemId}
            className={`bg-[--bg-tertiary] rounded-lg p-3 cursor-pointer transition-all hover:bg-[--bg-secondary] ${
              isSelected ? 'ring-2 ring-[--accent-blue]' : ''
            }`}
            onClick={() => onSelect(item)}
          >
            <div className="flex justify-between items-start mb-2">
              <div>
                <div className="text-sm font-medium text-[--text-primary]">
                  {item.event_name}
                </div>
                <div className="text-xs text-[--accent-yellow]">{item.team_name}</div>
              </div>
              <div className="text-right">
                <div className="text-lg font-bold text-[--accent-blue]">
                  {item.max_profit_margin.toFixed(2)}%
                </div>
                <div className="text-xs text-[--text-muted]">最高利润</div>
              </div>
            </div>
            <div className="flex justify-between text-xs text-[--text-muted]">
              <span>⏱ 持续 {item.duration_seconds ? formatDuration(item.duration_seconds) : '-'}</span>
              <span>{formatTime(item.start_time)}</span>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// 记录详情弹窗
function RecordDetail({ record, onClose }: { record: TrackingRecord; onClose: () => void }) {
  return (
    <div className="border-t border-[--border-color] bg-[--bg-primary] p-3 max-h-64 overflow-y-auto">
      <div className="flex justify-between items-center mb-3">
        <h4 className="text-sm font-semibold text-[--text-primary]">
          📊 {record.event_name} - {record.team_name}
        </h4>
        <button 
          onClick={onClose}
          className="text-[--text-muted] hover:text-[--text-primary]"
        >
          ✕
        </button>
      </div>
      
      <div className="grid grid-cols-2 gap-3 text-xs mb-3">
        <div>
          <div className="text-[--text-muted]">开始时间</div>
          <div className="text-[--text-primary]">{formatTime(record.start_time)}</div>
        </div>
        <div>
          <div className="text-[--text-muted]">结束时间</div>
          <div className="text-[--text-primary]">{record.end_time ? formatTime(record.end_time) : '-'}</div>
        </div>
        <div>
          <div className="text-[--text-muted]">持续时间</div>
          <div className="text-[--text-primary]">{record.duration_seconds ? formatDuration(record.duration_seconds) : '-'}</div>
        </div>
        <div>
          <div className="text-[--text-muted]">最高利润</div>
          <div className="text-[--accent-green] font-bold">{record.max_profit_margin.toFixed(2)}%</div>
        </div>
      </div>

      {/* 利润历史图表（简化版） */}
      {record.profit_history.length > 0 && (
        <div>
          <div className="text-xs text-[--text-muted] mb-2">利润变化 ({record.profit_history.length} 条记录)</div>
          <div className="bg-[--bg-tertiary] rounded p-2 h-20 flex items-end gap-px">
            {record.profit_history.slice(-50).map((entry, index) => {
              const height = Math.max(5, (entry.profit_margin / record.max_profit_margin) * 100);
              return (
                <div
                  key={index}
                  className="flex-1 bg-[--accent-blue] rounded-t transition-all hover:bg-[--accent-green]"
                  style={{ height: `${height}%` }}
                  title={`${entry.profit_margin.toFixed(2)}% @ ${formatTime(entry.time)}`}
                />
              );
            })}
          </div>
          <div className="flex justify-between text-xs text-[--text-muted] mt-1">
            <span>开始</span>
            <span>结束</span>
          </div>
        </div>
      )}
    </div>
  );
}

// 格式化持续时间
function formatDuration(seconds: number): string {
  if (seconds < 60) {
    return `${Math.round(seconds)}秒`;
  } else if (seconds < 3600) {
    const mins = Math.floor(seconds / 60);
    const secs = Math.round(seconds % 60);
    return `${mins}分${secs}秒`;
  } else {
    const hours = Math.floor(seconds / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    return `${hours}小时${mins}分`;
  }
}

// 格式化时间
function formatTime(isoString: string): string {
  const date = new Date(isoString);
  return date.toLocaleTimeString('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false
  });
}
