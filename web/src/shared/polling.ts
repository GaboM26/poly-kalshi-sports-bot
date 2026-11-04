/**
 * How often the UI re-fetches data from the backend, in milliseconds. One
 * place to tune every poll. Live prices and opportunities do not poll: they
 * arrive over the WebSocket.
 */
export const POLL_MS = {
  /** Wait before reconnecting a dropped WebSocket. */
  wsReconnect: 5000,
  /** Shared: account balances in the header (account endpoints are rate-limited). */
  accountBalance: 60000,
  /** Shared: the positions panel (account endpoints are rate-limited). */
  positions: 60000,
  /** Sports arb: data coverage in the header. */
  dataCoverage: 3000,
  /** Sports arb: auto-trade status in the header. */
  autoTradeStatus: 2000,
  /** Sports arb: arbitrage and auto-trade history. */
  arbitrageHistory: 5000,
  /** Sports arb: order book depth of the selected market. */
  orderbookDepth: 3000,
  /** Sports arb: active tracking panel. */
  tracking: 2000,
  /** Mention: decisions and positions. */
  mentionDetails: 10000,
} as const;
