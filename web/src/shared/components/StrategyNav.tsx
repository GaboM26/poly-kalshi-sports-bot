import type { StrategyEntry } from '../../strategies/registry';

interface StrategyNavProps {
  strategies: StrategyEntry[];
  active: string;
  onChange: (id: string) => void;
}

/** Top-level switch between trading strategies; each gets its own page. */
export function StrategyNav({ strategies, active, onChange }: StrategyNavProps) {
  return (
    <nav className="flex flex-shrink-0 gap-1 border-b border-[--border-color] bg-[--bg-tertiary] px-2">
      {strategies.map(({ id, label }) => (
        <button
          key={id}
          className={`px-4 py-1.5 text-xs font-medium transition-colors border-b-2 ${
            active === id
              ? 'text-[--accent-blue] border-[--accent-blue]'
              : 'text-[--text-muted] hover:text-[--text-secondary] border-transparent'
          }`}
          onClick={() => onChange(id)}
        >
          {label}
        </button>
      ))}
    </nav>
  );
}
