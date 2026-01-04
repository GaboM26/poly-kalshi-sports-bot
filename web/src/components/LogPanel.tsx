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
    <div className="h-full flex flex-col">
      <div className="px-3 py-2 border-b border-[--border-color] flex-shrink-0">
        <span className="text-xs text-[--text-secondary] uppercase tracking-wider font-medium">
          Logs
        </span>
        <span className="text-xs text-[--text-muted] ml-2">({logs.length})</span>
      </div>
      
      <div className="flex-1 overflow-y-auto p-2 font-mono">
        {logs.length === 0 ? (
          <div className="text-[--text-muted] text-xs">
            Waiting...
          </div>
        ) : (
          <div className="space-y-0.5">
            {logs.map((log, index) => (
              <div key={index} className="text-xs leading-relaxed">
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
