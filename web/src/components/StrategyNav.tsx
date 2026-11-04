export type Strategy = 'arbitrage' | 'mention';

const STRATEGIES: { id: Strategy; label: string }[] = [
  { id: 'arbitrage', label: 'Arbitrage Scanner' },
  { id: 'mention', label: 'Trump Speeches' },
];

interface StrategyNavProps {
  active: Strategy;
  onChange: (strategy: Strategy) => void;
}

/** Top-level switch between trading strategies; each gets its own page. */
export function StrategyNav({ active, onChange }: StrategyNavProps) {
  return (
    <nav className="flex flex-shrink-0 gap-1 border-b border-[--border-color] bg-[--bg-tertiary] px-2">
      {STRATEGIES.map(({ id, label }) => (
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
