import { AutoTradeControl } from './AutoTradeControl';
import { SettingsControl } from './SettingsControl';

/** Auto-trade and calculator settings buttons in the shared header. */
export function SportsArbHeaderControls() {
  return (
    <>
      <AutoTradeControl />
      <SettingsControl />
    </>
  );
}
