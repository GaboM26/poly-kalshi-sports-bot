import { useEffect, useState } from 'react';
import { Header } from './shared/components/Header';
import { Login } from './shared/components/Login';
import { StrategyNav } from './shared/components/StrategyNav';
import { useWebSocket } from './shared/useWebSocket';
import { wsUrl } from './shared/api';
import { STRATEGIES } from './strategies/registry';

const STRATEGY_STORAGE_KEY = 'strategy';

function App() {
  // Authentication state
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [currentUsername, setCurrentUsername] = useState<string | null>(null);

  // Check localStorage for a token on initialization.
  useEffect(() => {
    const token = localStorage.getItem('auth_token');
    const username = localStorage.getItem('username');
    if (token && username) {
      setIsAuthenticated(true);
      setCurrentUsername(username);
    }
  }, []);

  // Handle successful login.
  const handleLoginSuccess = (token: string, username: string) => {
    localStorage.setItem('auth_token', token);
    localStorage.setItem('username', username);
    setIsAuthenticated(true);
    setCurrentUsername(username);
  };

  // Log out.
  const handleLogout = () => {
    localStorage.removeItem('auth_token');
    localStorage.removeItem('username');
    setIsAuthenticated(false);
    setCurrentUsername(null);
  };

  const live = useWebSocket(wsUrl());
  const { matchedMarkets, isConnected, stats, lastUpdateTime, updateCount, dataCoverage, metrics } = live;
  // Calculate total profit from matched markets with arbitrage opportunities.
  const totalProfit = matchedMarkets
    .filter(m => m.has_opportunity)
    .reduce((sum, m) => sum + m.expected_profit, 0);

  const [strategyId, setStrategyId] = useState<string>(() => {
    try {
      return localStorage.getItem(STRATEGY_STORAGE_KEY) ?? STRATEGIES[0].id;
    } catch {
      return STRATEGIES[0].id;
    }
  });
  const selectStrategy = (id: string) => {
    setStrategyId(id);
    try {
      localStorage.setItem(STRATEGY_STORAGE_KEY, id);
    } catch {
      // Remembering the page is a convenience only.
    }
  };
  const active = STRATEGIES.find((s) => s.id === strategyId) ?? STRATEGIES[0];

  // Show the login page when unauthenticated.
  if (!isAuthenticated) {
    return <Login onLoginSuccess={handleLoginSuccess} />;
  }

  return (
    <div className="h-screen flex flex-col overflow-hidden">
      <Header 
        isConnected={isConnected} 
        stats={stats} 
        totalProfit={totalProfit}
        lastUpdateTime={lastUpdateTime}
        updateCount={updateCount}
        dataCoverage={dataCoverage}
        metrics={metrics}
        onLogout={handleLogout}
        username={currentUsername}
        controls={active.HeaderControls ? <active.HeaderControls /> : null}
      />
      <StrategyNav strategies={STRATEGIES} active={active.id} onChange={selectStrategy} />
      <active.Page live={live} />
    </div>
  );
}

export default App;
