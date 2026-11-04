import { useEffect, useState, useCallback } from 'react';
import { getAppSettings, updateAppSettings, AppSettings } from './api';

/** Header button and modal for the sports-arb calculator settings. */
export function SettingsControl() {
  // Application settings state
  const [appSettings, setAppSettings] = useState<AppSettings | null>(null);
  const [showSettingsModal, setShowSettingsModal] = useState(false);
  const [settingsLoading, setSettingsLoading] = useState(false);
  const [settingsMessage, setSettingsMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);
  // Temporary values used for editing
  const [editRefreshInterval, setEditRefreshInterval] = useState('5');
  const [editMinProfit, setEditMinProfit] = useState('1.0');
  const [editDefaultBet, setEditDefaultBet] = useState('10');
  const [editTrackingThreshold, setEditTrackingThreshold] = useState('2.0');

  // Fetch application settings.
  const fetchAppSettings = useCallback(async () => {
    try {
      const data = await getAppSettings();
      setAppSettings(data);
      setEditRefreshInterval(String(data.refresh_interval));
      setEditMinProfit(String(data.min_profit_margin));
      setEditDefaultBet(String(data.default_bet_amount));
      setEditTrackingThreshold(String(data.tracking_threshold));
    } catch (error) {
      console.error('Failed to fetch application settings:', error);
    }
  }, []);

  // Load application settings initially.
  useEffect(() => {
    fetchAppSettings();
  }, [fetchAppSettings]);

  // Show an application settings message.
  const showSettingsMsg = (text: string, type: 'success' | 'error') => {
    setSettingsMessage({ text, type });
    setTimeout(() => setSettingsMessage(null), 3000);
  };

  // Update application settings.
  const handleUpdateSettings = async () => {
    const refreshInterval = parseInt(editRefreshInterval);
    const minProfit = parseFloat(editMinProfit);
    const defaultBet = parseFloat(editDefaultBet);
    const trackingThreshold = parseFloat(editTrackingThreshold);

    if (isNaN(refreshInterval) || refreshInterval < 5) {
      showSettingsMsg('Refresh interval must be at least 5 seconds', 'error');
      return;
    }
    if (isNaN(minProfit) || minProfit < 0) {
      showSettingsMsg('Minimum profit margin cannot be negative', 'error');
      return;
    }
    if (isNaN(defaultBet) || defaultBet <= 0) {
      showSettingsMsg('Default amount must be greater than 0', 'error');
      return;
    }
    if (isNaN(trackingThreshold) || trackingThreshold < 0) {
      showSettingsMsg('Tracking threshold cannot be negative', 'error');
      return;
    }

    setSettingsLoading(true);
    try {
      const result = await updateAppSettings({
        refresh_interval: refreshInterval,
        min_profit_margin: minProfit,
        default_bet_amount: defaultBet,
        tracking_threshold: trackingThreshold,
      });
      if (result.success) {
        showSettingsMsg('Settings saved (fully applied after restart)', 'success');
        await fetchAppSettings();
      } else {
        showSettingsMsg(result.error || 'Failed to save settings', 'error');
      }
    } catch (error) {
      showSettingsMsg('Failed to save settings', 'error');
    }
    setSettingsLoading(false);
  };

  return (
    <>
      {/* Settings button */}
      <button
        onClick={() => setShowSettingsModal(true)}
        className="flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-semibold bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors"
        title="Application Settings"
      >
        <span>⚙️</span>
        <span>Settings</span>
      </button>

      {/* Application settings modal */}
      {showSettingsModal && appSettings && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50" onClick={() => setShowSettingsModal(false)}>
          <div 
            className="bg-[--bg-secondary] border border-[--border-color] rounded-lg shadow-xl w-96 max-w-[90vw]"
            onClick={(e) => e.stopPropagation()}
          >
            {/* Modal header */}
            <div className="flex items-center justify-between px-4 py-3 border-b border-[--border-color]">
              <div className="flex items-center gap-2">
                <span className="text-base">⚙️</span>
                <span className="font-semibold text-[--text-primary] text-sm">Application Settings</span>
              </div>
              <button
                onClick={() => setShowSettingsModal(false)}
                className="text-[--text-muted] hover:text-[--text-primary] text-lg leading-none"
              >
                ×
              </button>
            </div>

            {/* Modal content */}
            <div className="p-4 space-y-4">
              {/* Refresh interval */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm text-[--text-secondary]">Data Refresh Interval</span>
                  <p className="text-[10px] text-[--text-muted]">Interval for scheduled market scans</p>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={editRefreshInterval}
                    onChange={(e) => setEditRefreshInterval(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="1"
                  />
                  <span className="text-xs text-[--text-muted]">seconds</span>
                </div>
              </div>

              {/* Minimum profit margin */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm text-[--text-secondary]">Minimum Profit Margin</span>
                  <p className="text-[10px] text-[--text-muted]">Hide opportunities below this profit margin</p>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={editMinProfit}
                    onChange={(e) => setEditMinProfit(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="0"
                    step="0.1"
                  />
                  <span className="text-xs text-[--text-muted]">%</span>
                </div>
              </div>

              {/* Default bet amount */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm text-[--text-secondary]">Default Bet Amount</span>
                  <p className="text-[10px] text-[--text-muted]">Default amount used for arbitrage calculations</p>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-xs text-[--text-muted]">$</span>
                  <input
                    type="number"
                    value={editDefaultBet}
                    onChange={(e) => setEditDefaultBet(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="1"
                    step="1"
                  />
                </div>
              </div>

              {/* Tracking threshold */}
              <div className="flex items-center justify-between">
                <div>
                  <span className="text-sm text-[--text-secondary]">Tracking Threshold</span>
                  <p className="text-[10px] text-[--text-muted]">Start tracking when this profit margin is exceeded</p>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    value={editTrackingThreshold}
                    onChange={(e) => setEditTrackingThreshold(e.target.value)}
                    className="w-16 text-xs px-2 py-1 rounded bg-[--bg-primary] border border-[--border-color] text-[--text-primary] focus:outline-none focus:border-blue-500 text-right"
                    min="0"
                    step="0.1"
                  />
                  <span className="text-xs text-[--text-muted]">%</span>
                </div>
              </div>

              {/* Save button */}
              <div className="flex justify-end pt-2">
                <button
                  onClick={handleUpdateSettings}
                  disabled={settingsLoading}
                  className="px-4 py-1.5 rounded bg-blue-500/20 text-blue-400 hover:bg-blue-500/30 transition-colors text-xs font-semibold disabled:opacity-50"
                >
                  {settingsLoading ? 'Saving...' : 'Save Settings'}
                </button>
              </div>

              {/* Status message */}
              {settingsMessage && (
                <div className={`text-xs px-3 py-2 rounded ${
                  settingsMessage.type === 'success'
                    ? 'bg-green-500/20 text-green-400'
                    : 'bg-red-500/20 text-red-400'
                }`}>
                  {settingsMessage.text}
                </div>
              )}
            </div>

            {/* Modal footer */}
            <div className="px-4 py-3 border-t border-[--border-color] bg-[--bg-tertiary] rounded-b-lg">
              <p className="text-[10px] text-[--text-muted] leading-relaxed">
                💡 Settings take effect immediately. Some settings, such as the refresh interval, require a service restart to be fully applied.
              </p>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
