import { useEffect, useRef, useState } from 'react';
import { WsMessage, LogEntry, DataCoverage, MatchedMarketData, MetricsReport } from '../types';

export function useWebSocket(url: string) {
  const [matchedMarkets, setMatchedMarkets] = useState<MatchedMarketData[]>([]); // 所有匹配市场（包含套利信息）
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isConnected, setIsConnected] = useState(false);
  const [isReceivingData, setIsReceivingData] = useState(false); // 是否正在接收实时数据
  const [lastUpdateTime, setLastUpdateTime] = useState<Date | null>(null); // 最后更新时间
  const [updateCount, setUpdateCount] = useState(0); // 更新计数，用于触发动画
  const [stats, setStats] = useState({
    kalshiCount: 0,
    polymarketCount: 0,
    matchedCount: 0,
    opportunitiesCount: 0,
  });
  const [dataCoverage, setDataCoverage] = useState<DataCoverage>({
    total_markets: 0,
    kalshi_ready: 0,
    polymarket_ready: 0,
    both_ready: 0,
    kalshi_coverage: '0/0',
    polymarket_coverage: '0/0',
    full_coverage: '0/0',
    kalshi_connected: false,
    polymarket_connected: false,
  });
  const [metrics, setMetrics] = useState<MetricsReport | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  // 跟踪价格变化用于高亮动画
  const prevPricesRef = useRef<Map<string, { k_yes: number; k_no: number; p_yes: number; p_no: number }>>(new Map());

  useEffect(() => {
    // 将 addLog 移到 useEffect 内部，避免闭包陷阱
    const addLog = (level: LogEntry['level'], message: string) => {
      const entry: LogEntry = {
        time: new Date().toISOString(),
        level,
        message,
      };
      setLogs((prev) => {
        const newLogs = [entry, ...prev];
        return newLogs.slice(0, 100); // 保留最近100条
      });
    };

    // 将 handleMessage 移到 useEffect 内部，解决闭包陷阱
    const handleMessage = (message: WsMessage) => {
      switch (message.type) {
        case 'opportunity':
          // Rust 后端发送单个套利机会
          if (message.data) {
            const opp = message.data as any;
            addLog('success', `发现套利机会: ${opp.event_name} ${opp.team_name} - ${opp.profit_margin.toFixed(2)}%`);
          }
          break;

        case 'opportunities':
          // Rust 后端发送套利机会列表
          if (message.data && Array.isArray(message.data)) {
            setIsReceivingData(true);
            const opportunities = message.data as any[];
            
            // 更新统计
            setStats((prev) => ({
              ...prev,
              opportunitiesCount: opportunities.length,
            }));
            
            setLastUpdateTime(new Date());
            setUpdateCount((prev) => prev + 1);
          }
          break;

        case 'stats':
          // Rust 后端发送系统统计
          if (message.data) {
            const statsData = message.data as any;
            setStats({
              kalshiCount: statsData.total_kalshi_markets || 0,
              polymarketCount: statsData.total_polymarket_markets || 0,
              matchedCount: statsData.matched_markets || 0,
              opportunitiesCount: statsData.arbitrage_opportunities || 0,
            });
          }
          break;

        case 'log':
          if (message.level && message.message) {
            addLog(message.level as any, message.message);
          }
          break;

        case 'matched_markets_list':
          // 兼容旧的 Python 后端消息格式（如果有）
          if (message.data && Array.isArray(message.data)) {
            setIsReceivingData(true);
            const marketsData = message.data as MatchedMarketData[];
            
            // 检测价格变化，用于高亮动画
            // 使用更唯一的 key：kalshi_market_id + polymarket_market_id
            const newPricesMap = new Map<string, { k_yes: number; k_no: number; p_yes: number; p_no: number; changed: boolean }>();
            marketsData.forEach((m) => {
              const key = `${m.kalshi_market_id}_${m.polymarket_market_id}`;
              const prev = prevPricesRef.current.get(key);
              const changed = prev ? (
                prev.k_yes !== m.kalshi_yes_price ||
                prev.k_no !== m.kalshi_no_price ||
                prev.p_yes !== m.poly_yes_price ||
                prev.p_no !== m.poly_no_price
              ) : false;
              newPricesMap.set(key, {
                k_yes: m.kalshi_yes_price,
                k_no: m.kalshi_no_price,
                p_yes: m.poly_yes_price,
                p_no: m.poly_no_price,
                changed
              });
            });
            prevPricesRef.current = new Map(
              Array.from(newPricesMap.entries()).map(([k, v]) => [k, { k_yes: v.k_yes, k_no: v.k_no, p_yes: v.p_yes, p_no: v.p_no }])
            );
            
            setMatchedMarkets(marketsData);
            setLastUpdateTime(new Date());
            setUpdateCount((prev) => prev + 1);
            
            // 更新统计
            if (message.count !== undefined) {
              setStats((prev) => ({
                ...prev,
                matchedCount: message.count!,
                opportunitiesCount: message.opportunities_count || prev.opportunitiesCount,
              }));
            }
          }
          break;

        case 'metrics':
          // 处理性能指标消息
          if (message.data && typeof message.data === 'object' && 'api_latency' in message.data) {
            setMetrics(message.data as MetricsReport);
          }
          break;

        default:
          break;
      }
    };

    const connect = () => {
      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        console.log('WebSocket 连接已建立');
        setIsConnected(true);
        addLog('success', 'WebSocket 连接成功');
      };

      ws.onmessage = (event) => {
        try {
          const message: WsMessage = JSON.parse(event.data);
          console.log('收到 WebSocket 消息:', message.type, message);
          handleMessage(message);
        } catch (error) {
          console.error('解析 WebSocket 消息失败:', error);
        }
      };

      ws.onerror = (error) => {
        console.error('WebSocket 错误:', error);
        addLog('error', 'WebSocket 连接错误');
      };

      ws.onclose = () => {
        console.log('WebSocket 连接已关闭');
        setIsConnected(false);
        addLog('warning', 'WebSocket 连接断开，5秒后重连...');
        
        // 5秒后重连
        setTimeout(() => {
          connect();
        }, 5000);
      };
    };

    connect();

    return () => {
      if (wsRef.current) {
        wsRef.current.close();
      }
    };
  }, [url]);

  // 定期获取数据覆盖率
  useEffect(() => {
    const fetchCoverage = async () => {
      try {
        // 从 ws url 推断 api url
        const apiUrl = url.replace('ws://', 'http://').replace('wss://', 'https://').replace('/ws', '');
        const response = await fetch(`${apiUrl}/api/data-coverage`);
        if (response.ok) {
          const data = await response.json();
          setDataCoverage(data);
        }
      } catch (error) {
        // 静默失败
      }
    };

    // 初始获取
    fetchCoverage();
    // 每 3 秒更新一次
    const interval = setInterval(fetchCoverage, 3000);
    return () => clearInterval(interval);
  }, [url]);

  return {
    matchedMarkets,
    logs,
    isConnected,
    isReceivingData,
    lastUpdateTime,
    updateCount,
    stats,
    dataCoverage,
    metrics,
  };
}
