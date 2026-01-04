import { useEffect, useRef, useState, useCallback } from 'react';
import { WsMessage, ArbitrageOpportunity, LogEntry } from '../types';

export function useWebSocket(url: string) {
  const [opportunities, setOpportunities] = useState<ArbitrageOpportunity[]>([]);
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
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
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

  const handleMessage = (message: WsMessage) => {
    switch (message.type) {
      case 'opportunities_list':
        // 处理完整的套利机会列表（实时更新）
        if (message.data && Array.isArray(message.data)) {
          setIsReceivingData(true);
          setOpportunities(message.data);
          setLastUpdateTime(new Date());
          setUpdateCount((prev) => prev + 1);
          // 更新统计
          if (message.count !== undefined) {
            setStats((prev) => ({
              ...prev,
              opportunitiesCount: message.count!,
            }));
          }
        }
        break;
        
      case 'opportunity':
        if (message.data && !Array.isArray(message.data)) {
          // 标记已开始接收实时数据
          setIsReceivingData(true);
          
          const oppData = message.data as ArbitrageOpportunity;
          console.log('处理套利机会:', oppData.kalshi_market.event_name, 
                      'K价格:', oppData.kalshi_market.yes_price, oppData.kalshi_market.no_price,
                      'P价格:', oppData.polymarket_market.yes_price, oppData.polymarket_market.no_price,
                      '利润:', oppData.profit_margin);
          
          setOpportunities((prev) => {
            // 检查是否已存在（使用更宽松的匹配，只要事件ID相同就更新）
            const existingIndex = prev.findIndex(
              (opp) =>
                opp.kalshi_market.event_id === oppData.kalshi_market.event_id &&
                opp.polymarket_market.event_id === oppData.polymarket_market.event_id
            );

            let newOpps;
            if (existingIndex >= 0) {
              // 更新现有机会（保持原位置）
              console.log('更新现有套利机会，索引:', existingIndex);
              newOpps = [...prev];
              newOpps[existingIndex] = oppData;
            } else {
              // 添加新机会
              console.log('添加新套利机会');
              newOpps = [...prev, oppData];
              addLog('success', `新套利: ${oppData.kalshi_market.event_name}`);
            }
            
            // 按利润率排序
            newOpps.sort((a, b) => b.profit_margin - a.profit_margin);
            // 保留前100个
            return newOpps.slice(0, 100);
          });
        }
        break;

      case 'scan_started':
        addLog('info', '开始市场扫描...');
        break;

      case 'scan_completed':
        setStats({
          kalshiCount: message.kalshi_count || 0,
          polymarketCount: message.polymarket_count || 0,
          matchedCount: message.matched_count || 0,
          opportunitiesCount: message.opportunities_count || 0,
        });
        // 扫描完成后，等待实时数据
        if (message.matched_count && message.matched_count > 0) {
          addLog('info', `市场匹配完成: ${message.matched_count} 对，等待实时价格...`);
        }
        break;

      case 'log':
        if (message.level && message.message) {
          addLog(message.level as any, message.message);
        }
        break;

      case 'ping':
        // 响应心跳
        if (wsRef.current && wsRef.current.readyState === WebSocket.OPEN) {
          wsRef.current.send(JSON.stringify({ type: 'pong' }));
        }
        break;

      default:
        break;
    }
  };

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

  return {
    opportunities,
    logs,
    isConnected,
    isReceivingData,
    lastUpdateTime,
    updateCount,
    stats,
  };
}
