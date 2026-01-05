import { LogEntry } from '../types';

interface LogPanelProps {
  logs: LogEntry[];
}

export function LogPanel({ logs }: LogPanelProps) {
  const getLogColor = (level: LogEntry['level']) => {
    switch (level) {
      case 'info': return 'text-blue-400';
      case 'success': return 'text-green-400';
      case 'warning': return 'text-yellow-400';
      case 'error': return 'text-red-400';
    }
  };

  const getLogIcon = (level: LogEntry['level']) => {
    switch (level) {
      case 'info': return '●';
      case 'success': return '✓';
      case 'warning': return '⚠';
      case 'error': return '✗';
    }
  };

  return (
    <div className="h-full flex flex-col card overflow-hidden">
      <div className="px-3 py-1.5 border-b border-[--border-color] flex-shrink-0 bg-[--bg-secondary]">
        <span className="text-[10px] text-[--text-secondary] uppercase tracking-wider font-semibold">
          LOGS
        </span>
        <span className="text-[10px] text-[--text-muted] ml-2">({logs.length})</span>
      </div>
      
      <div className="flex-1 overflow-y-auto p-2 font-mono bg-[--bg-primary]">
        {logs.length === 0 ? (
          <div className="text-[--text-muted] text-[10px]">
            Waiting for logs...
          </div>
        ) : (
          <div className="space-y-0.5">
            {logs.map((log, index) => (
              <div key={index} className="text-[10px] leading-relaxed hover:bg-[--bg-secondary] px-1 py-0.5 rounded transition-colors">
                <span className="text-[--text-muted] tabular-nums">
                  {new Date(log.time).toLocaleTimeString('en', { hour12: false })}
                </span>
                <span className={`${getLogColor(log.level)} mx-1`}>
                  {getLogIcon(log.level)}
                </span>
                <span className="text-[--text-secondary]">
                  {log.message}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
