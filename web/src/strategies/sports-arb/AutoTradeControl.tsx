import { useEffect, useState, useCallback } from 'react';
import { getAutoTradeStatus, enableAutoTrade, disableAutoTrade, resetAutoTradeCount, updateAutoTradeSettings, AutoTradeStatus } from './api';

/** Header button and modal for the sports-arb auto-trader. */
export function AutoTradeControl() {
  // Automated trading state
  const [autoTradeStatus, setAutoTradeStatus] = useState<AutoTradeStatus | null>(null);
  const [showAutoTradeModal, setShowAutoTradeModal] = useState(false);
  const [autoTradeLoading, setAutoTradeLoading] = useState(false);
  const [durationInput, setDurationInput] = useState('500');
  const [maxTradeCountInput, setMaxTradeCountInput] = useState('2');
  const [maxContractsInput, setMaxContractsInput] = useState('100');
  const [minContractsInput, setMinContractsInput] = useState('10');
  const [neutralizationLossInput, setNeutralizationLossInput] = useState('5');
  const [autoTradeMessage, setAutoTradeMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);

  // Fetch automated trading status. This runs on a 2s poll for the live
  // read-only display (autoTradeStatus.*) - it must not also overwrite the
  // editable input fields below on every tick, or a poll landing mid-edit
  // silently reverts whatever the user is typing before they can submit it.
  // Only seed the inputs from the server on the very first load; after that,
  // an input only changes when the user edits it or a save round-trips.
  const fetchAutoTradeStatus = useCallback(async () => {
    try {
      const data = await getAutoTradeStatus();
      setAutoTradeStatus((prev) => {
        if (prev === null) {
          setDurationInput(String(data.min_duration_ms));
          setMaxTradeCountInput(String(data.max_trade_count));
          setMaxContractsInput(String(data.max_contracts));
          setMinContractsInput(String(data.min_contracts));
          setNeutralizationLossInput(String(data.neutralization_max_loss_cents));
        }
        return data;
      });
    } catch (error) {
      console.error('Failed to fetch automated trading status:', error);
    }
  }, []);

  // Refresh automated trading status periodically.
  useEffect(() => {
    fetchAutoTradeStatus();
    const interval = setInterval(fetchAutoTradeStatus, 2000);
    return () => clearInterval(interval);
  }, [fetchAutoTradeStatus]);

  // Show an automated trading message.
  const showAutoTradeMsg = (text: string, type: 'success' | 'error') => {
    setAutoTradeMessage({ text, type });
    setTimeout(() => setAutoTradeMessage(null), 3000);
  };

  // Toggle automated trading.
  const handleAutoTradeToggle = async () => {
    if (!autoTradeStatus) return;
    setAutoTradeLoading(true);
    try {
      if (autoTradeStatus.enabled) {
        const result = await disableAutoTrade();
        if (result.success) showAutoTradeMsg('Automated trading disabled', 'success');
        else showAutoTradeMsg(result.error || 'Operation failed', 'error');
      } else {
        const result = await enableAutoTrade();
        if (result.success) showAutoTradeMsg('Automated trading enabled', 'success');
        else showAutoTradeMsg(result.error || 'Operation failed', 'error');
      }
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('Operation failed', 'error');
    }
    setAutoTradeLoading(false);
  };

  // Reset the trade count.
  const handleAutoTradeReset = async () => {
    setAutoTradeLoading(true);
    try {
      const result = await resetAutoTradeCount();
      if (result.success) showAutoTradeMsg('Trade count reset', 'success');
      else showAutoTradeMsg(result.error || 'Failed to reset trade count', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('Failed to reset trade count', 'error');
    }
    setAutoTradeLoading(false);
  };

  // Update the duration threshold.
  const handleUpdateDuration = async () => {
    const duration = parseInt(durationInput);
    if (isNaN(duration) || duration < 0) {
      showAutoTradeMsg('Enter a valid duration', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings({ min_duration_ms: duration });
      if (result.success) showAutoTradeMsg('Settings updated', 'success');
      else showAutoTradeMsg(result.error || 'Failed to update settings', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('Failed to update settings', 'error');
    }
    setAutoTradeLoading(false);
  };

  // Update the maximum number of trades.
  const handleUpdateMaxTradeCount = async () => {
    const count = parseInt(maxTradeCountInput);
    if (isNaN(count) || count < 1) {
      showAutoTradeMsg('Enter a valid trade count (≥1)', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings({ max_trade_count: count });
      if (result.success) showAutoTradeMsg('Maximum trade count updated', 'success');
      else showAutoTradeMsg(result.error || 'Failed to update settings', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('Failed to update settings', 'error');
    }
    setAutoTradeLoading(false);
  };

  // Toggle flexible order sizing.
  const handleToggleFlexibleMode = async () => {
    if (!autoTradeStatus) return;
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings({ 
        flexible_mode: !autoTradeStatus.flexible_mode 
      });
      if (result.success) {
        showAutoTradeMsg(
          autoTradeStatus.flexible_mode ? 'Switched to fixed mode' : 'Switched to flexible mode',
          'success'
        );
      } else {
        showAutoTradeMsg(result.error || 'Failed to switch mode', 'error');
      }
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('Failed to switch mode', 'error');
    }
    setAutoTradeLoading(false);
  };

  // Update maximum contracts.
  const handleUpdateMaxContracts = async () => {
    const count = parseInt(maxContractsInput);
    if (isNaN(count) || count < 1) {
      showAutoTradeMsg('Enter a valid contract count (≥1)', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings({ max_contracts: count });
      if (result.success) showAutoTradeMsg('Maximum contracts updated', 'success');
      else showAutoTradeMsg(result.error || 'Failed to update settings', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('Failed to update settings', 'error');
    }
    setAutoTradeLoading(false);
  };

  // Update minimum contracts.
  const handleUpdateMinContracts = async () => {
    const count = parseInt(minContractsInput);
    if (isNaN(count) || count < 1) {
      showAutoTradeMsg('Enter a valid contract count (≥1)', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings({ min_contracts: count });
      if (result.success) showAutoTradeMsg('Minimum contracts updated', 'success');
      else showAutoTradeMsg(result.error || 'Failed to update settings', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('Failed to update settings', 'error');
    }
    setAutoTradeLoading(false);
  };

  const handleUpdateNeutralizationLoss = async () => {
    const cents = parseInt(neutralizationLossInput);
    if (isNaN(cents) || cents < 0 || cents > 99) {
      showAutoTradeMsg('Enter a close-loss bound from 0 to 99 cents', 'error');
      return;
    }
    setAutoTradeLoading(true);
    try {
      const result = await updateAutoTradeSettings({
        neutralization_max_loss_cents: cents,
      });
      if (result.success) showAutoTradeMsg('Close-loss bound updated', 'success');
      else showAutoTradeMsg(result.error || 'Failed to update close-loss bound', 'error');
      await fetchAutoTradeStatus();
    } catch (error) {
      showAutoTradeMsg('Failed to update close-loss bound', 'error');
    }
    setAutoTradeLoading(false);
  };

  return (
    <>
      {/* Automated trading button */}
      {autoTradeStatus && (
        <button
          onClick={() => setShowAutoTradeModal(true)}
          className={`flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-semibold transition-colors ${
            autoTradeStatus.enabled
              ? 'bg-green-500/20 text-green-400 hover:bg-green-500/30'
              : 'bg-gray-500/20 text-gray-400 hover:bg-gray-500/30'
          }`}
          title="Automated Trading Settings"
        >
          <span>🤖</span>
          <span>{autoTradeStatus.enabled ? 'Auto' : 'Manual'}</span>
          <span className="font-mono">{autoTradeStatus.trade_count}/{autoTradeStatus.max_trade_count}</span>
        </button>
      )}

      {/* Automated trading settings modal */}
      {showAutoTradeModal && autoTradeStatus && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowAutoTradeModal(false)}>
          <div 
            className="bg-[--bg-secondary] border border-[--border-color] rounded-lg shadow-xl w-80 max-w-[90vw]"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Modal header */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-[--border-color]">
              <div className="flex items-center gap-2">
                <span className="text-base">🤖</span>
                <span className="font-semibold text-[--text-primary] text-sm">Automated Trading Settings</span>
              </div>
              <button
                onClick={() => setShowAutoTradeModal(false)}
                className="text-[--text-muted] hover:text-[--text-primary] text-lg leading-none"
              >
                ×
              </button>
            </div>

            {/* Modal content */}
            <div className="p-4 space-y-4">
              {/* Toggle state */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">Current Status</span>
                <div className="flex items-center gap-2">
                  <span className={`text-xs px-2 py-1 rounded font-semibold ${
                    autoTradeStatus.enabled
                      ? 'bg-green-500/20 text-green-400'
                      : 'bg-gray-500/20 text-gray-400'
                  }`}>
                    {autoTradeStatus.enabled ? 'Running' : 'Disabled'}
                  </span>
                  <button
                    onClick={handleAutoTradeToggle}
                    disabled={autoTradeLoading}
                    className={`text-xs px-3 py-1 rounded font-semibold transition-colors ${
                      autoTradeStatus.enabled
                        ? 'bg-red-500/20 text-red-400 hover:bg-red-500/30'
                        : 'bg-green-500/20 text-green-400 hover:bg-green-500/30'
                    } ${autoTradeLoading ? 'opacity-50 cursor-not-allowed' : ''}`}
                  >
                    {autoTradeStatus.enabled ? 'Disable' : 'Enable'}
                  </button>
                </div>
              </div>

              {/* Trade count */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">Trades Placed</span>
                <div className="flex items-center gap-2">
                  <span className={`text-sm font-mono font-semibold ${
                    autoTradeStatus.remaining > 0 ? 'text-green-400' : 'text-red-400'
                  }`}>
                    {autoTradeStatus.trade_count}
                  </span>
                  <span className="text-[--text-muted]">/</span>
                  <input
                    type="number"
                    value={maxTradeCountInput}
                    onChange={(e) => setMaxTradeCountInput(e.target.value)}
                    className="w-12 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-center"
                    min="1"
                  />
                  <button
                    onClick={handleUpdateMaxTradeCount}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                  >
                    Update
                  </button>
                  <button
                    onClick={handleAutoTradeReset}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-orange-500/20 text-orange-400 hover:bg-orange-500/30 transition-colors disabled:opacity-50"
                  >
                    Reset
                  </button>
                </div>
              </div>

              {/* Duration threshold */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">Minimum Duration</span>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={durationInput}
                    onChange={(e) => setDurationInput(e.target.value)}
                    className="w-20 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="0"
                    step="100"
                  />
                  <span className="text-xs text-[--text-muted]">ms</span>
                  <button
                    onClick={handleUpdateDuration}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                  >
                    Update
                  </button>
                </div>
              </div>

              {/* Maximum amount per trade */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">Maximum Amount per Trade</span>
                <span className="text-sm font-mono text-[--text-primary]">${autoTradeStatus.max_amount}</span>
              </div>

              {/* Divider */}
              <div className="border-t border-[--border-color] my-2"></div>

              {/* Order mode */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">Order Mode</span>
                <div className="flex items-center gap-2">
                  <span className={`text-xs px-2 py-1 rounded font-semibold ${
                    autoTradeStatus.flexible_mode
                      ? 'bg-purple-500/20 text-purple-400'
                      : 'bg-blue-500/20 text-blue-400'
                  }`}>
                    {autoTradeStatus.flexible_mode ? 'Flexible Mode' : 'Fixed Mode'}
                  </span>
                  <button
                    onClick={handleToggleFlexibleMode}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-gray-500/20 text-gray-300 hover:bg-gray-500/30 transition-colors disabled:opacity-50"
                  >
                    Switch
                  </button>
                </div>
              </div>

              {/* Maximum contracts */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">Maximum Contracts per Trade</span>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={maxContractsInput}
                    onChange={(e) => setMaxContractsInput(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-center"
                    min="1"
                  />
                  <span className="text-xs text-[--text-muted]">contracts</span>
                  <button
                    onClick={handleUpdateMaxContracts}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                  >
                    Update
                  </button>
                </div>
              </div>

              {/* Minimum contracts */}
              <div className="flex items-center justify-between">
                <span className="text-sm text-[--text-secondary]">Minimum Contract Threshold</span>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={minContractsInput}
                    onChange={(e) => setMinContractsInput(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-center"
                    min="1"
                  />
                  <span className="text-xs text-[--text-muted]">contracts</span>
                  <button
                    onClick={handleUpdateMinContracts}
                    disabled={autoTradeLoading}
                    className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                  >
                    Update
                  </button>
                </div>

                {/* Bounded exposure neutralization */}
                <div className="flex items-center justify-between">
                  <div>
                    <span className="text-sm text-[--text-secondary]">Emergency Close Loss Bound</span>
                    <p className="text-[10px] text-[--text-muted]">Maximum adverse price allowed for a partial-fill close</p>
                  </div>
                  <div className="flex items-center gap-2">
                    <input
                      type="number"
                      value={neutralizationLossInput}
                      onChange={(e) => setNeutralizationLossInput(e.target.value)}
                      className="w-12 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-center"
                      min="0"
                      max="99"
                    />
                    <span className="text-xs text-[--text-muted]">¢</span>
                    <button
                      onClick={handleUpdateNeutralizationLoss}
                      disabled={autoTradeLoading}
                      className="text-xs px-2 py-1 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors disabled:opacity-50"
                    >
                      Update
                    </button>
                  </div>
                </div>
              </div>

              {/* Last trade time */}
              {autoTradeStatus.last_trade_time && (
                <div className="flex items-center justify-between">
                  <span className="text-sm text-[--text-secondary]">Last Trade</span>
                  <span className="text-xs text-[--text-muted]">
                    {new Date(autoTradeStatus.last_trade_time).toLocaleString()}
                  </span>
                </div>
              )}

              {/* Status message */}
              {autoTradeMessage && (
                <div className={`text-xs px-3 py-2 rounded ${
                  autoTradeMessage.type === 'success'
                    ? 'bg-green-500/20 text-green-400'
                    : 'bg-red-500/20 text-red-400'
                }`}>
                  {autoTradeMessage.text}
                </div>
              )}
            </div>

            {/* Modal footer */}
            <div className="px-4 py-3 border-t border-[--border-color] bg-[--bg-tertiary] rounded-b-lg">
              <p className="text-[10px] text-[--text-muted] leading-relaxed">
                💡 {autoTradeStatus.flexible_mode 
                  ? `Flexible mode: trade ${autoTradeStatus.min_contracts} contracts at depth 10–20, or half the depth at ≥20 (up to ${autoTradeStatus.max_contracts} contracts)`
                  : `Fixed mode: trade ${autoTradeStatus.min_contracts} contracts each time`
                }. Both platforms must have depth of at least {autoTradeStatus.min_contracts} contracts before trading.
                A partial or one-leg fill is closed only within the configured {autoTradeStatus.neutralization_max_loss_cents}¢ bound; otherwise auto-trading halts.
              </p>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
