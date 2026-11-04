import { MentionSignalsPanel } from './MentionSignalsPanel';
import { OrderPanel } from '../../shared/components/OrderPanel';
import type { StrategyPageProps } from '../registry';

/** Trump Speeches: mention signals on top, the shared positions panel below. */
export function MentionPage({ live }: StrategyPageProps) {
  return (
    <main className="flex-1 flex flex-col overflow-hidden gap-1 p-1 bg-[--bg-primary]">
      <div className="flex-1 min-h-0 overflow-y-auto bg-[--bg-secondary] rounded border border-[--border-color] shadow-lg" style={{flexBasis: '55%'}}>
        <MentionSignalsPanel signals={live.mentionSignals} status={live.mentionStatus} />
      </div>
      <div className="flex-1 min-h-0 overflow-hidden flex flex-col bg-[--bg-secondary] rounded border border-[--border-color] shadow-lg" style={{flexBasis: '45%'}}>
        <OrderPanel />
      </div>
    </main>
  );
}
