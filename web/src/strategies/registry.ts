/**
 * Every strategy page, in nav order. Adding a strategy: a folder under
 * strategies/ with its page (and api.ts), plus one entry here. `id` matches
 * the backend strategy id and its /api/<id> route prefix.
 */
import { ComponentType } from 'react';
import { useWebSocket } from '../shared/useWebSocket';
import { SportsArbPage } from './sports-arb/SportsArbPage';
import { SportsArbHeaderControls } from './sports-arb/SportsArbHeaderControls';
import { MentionPage } from './mention/MentionPage';

/** Live data from the shared WebSocket, handed to every page. */
export type LiveData = ReturnType<typeof useWebSocket>;

export interface StrategyPageProps {
  live: LiveData;
}

export interface StrategyEntry {
  id: string;
  label: string;
  Page: ComponentType<StrategyPageProps>;
  /** Shown in the shared header while this strategy is open. */
  HeaderControls?: ComponentType;
}

export const STRATEGIES: StrategyEntry[] = [
  { id: 'sports_arb', label: 'Arbitrage Scanner', Page: SportsArbPage, HeaderControls: SportsArbHeaderControls },
  { id: 'mention', label: 'Trump Speeches', Page: MentionPage },
];
