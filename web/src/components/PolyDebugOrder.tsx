import { useState } from 'react';
import { createPolymarketOrder } from '../utils/api';

interface PolyDebugOrderProps {
  apiBaseUrl: string;
  onClose: () => void;
}

interface OrderLog {
  timestamp: string;
  type: 'request' | 'response' | 'error';
  message: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  data?: any;
}

export function PolyDebugOrder({ apiBaseUrl, onClose }: PolyDebugOrderProps) {
  const [tokenId, setTokenId] = useState('');
  const [side, setSide] = useState<'buy' | 'sell'>('buy');
  const [amount, setAmount] = useState(1);
  const [loading, setLoading] = useState(false);
  const [logs, setLogs] = useState<OrderLog[]>([]);

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const addLog = (type: OrderLog['type'], message: string, data?: any) => {
    const timestamp = new Date().toISOString().split('T')[1].slice(0, 12);
    setLogs(prev => [...prev, { timestamp, type, message, data }]);
  };

  const clearLogs = () => setLogs([]);

  const handleOrder = async () => {
    if (!tokenId.trim()) {
      addLog('error', 'Token ID 不能为空');
      return;
    }

    setLoading(true);
    clearLogs();

    const request = {
      token_id: tokenId.trim(),
      side,
      amount,
    };

    addLog('request', '发送下单请求', request);

    try {
      const startTime = Date.now();
      const response = await createPolymarketOrder(apiBaseUrl, request);
      const elapsed = Date.now() - startTime;

      addLog('response', `收到响应 (${elapsed}ms)`, response);

      if (response.success) {
        addLog('response', `✅ 下单成功! order_id=${response.order_id || 'N/A'}`);
      } else {
        addLog('error', `❌ 下单失败: ${response.error || '未知错误'}`);
      }
    } catch (e) {
      addLog('error', `❌ 请求异常: ${e instanceof Error ? e.message : '未知错误'}`);
    } finally {
      setLoading(false);
    }
  };

  // 预设的测试 token (CHI-MIA 比赛)
  const presetTokens = [
    { 
      name: 'CHI-MIA (CHI Yes)', 
      token: '94515776290373751754638142228993059501097351216445649452643423016914071837398',
      description: 'Chicago Bulls 胜利'
    },
    { 
      name: 'PHX-WAS (WAS Yes)', 
      token: '113640777070257914779167991695197859988168871541269340805216299248113189823953',
      description: 'Washington Wizards 胜利'
    },
  ];

  return (
    <div className="fixed inset-0 bg-black/80 flex items-center justify-center z-50 p-4">
      <div className="bg-[--bg-secondary] rounded-lg border border-[--border-color] w-full max-w-2xl max-h-[90vh] flex flex-col">
        {/* 标题栏 */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-[--border-color]">
          <div className="flex items-center gap-2">
            <span className="text-lg">🔧</span>
            <h2 className="text-sm font-medium text-[--text-primary]">Polymarket 手动下单调试</h2>
          </div>
          <button
            onClick={onClose}
            className="text-[--text-muted] hover:text-[--text-secondary] text-xl"
          >
            ×
          </button>
        </div>

        {/* 内容区 */}
        <div className="flex-1 overflow-y-auto p-4 space-y-4">
          {/* 预设 Token */}
          <div className="bg-[--bg-tertiary] rounded p-3">
            <div className="text-xs text-[--text-muted] mb-2">快速选择 Token</div>
            <div className="flex flex-wrap gap-2">
              {presetTokens.map((preset) => (
                <button
                  key={preset.token}
                  onClick={() => setTokenId(preset.token)}
                  className={`px-2 py-1 text-xs rounded border ${
                    tokenId === preset.token
                      ? 'border-purple-500 bg-purple-500/20 text-purple-400'
                      : 'border-[--border-color] bg-[--bg-secondary] text-[--text-secondary] hover:bg-[--bg-primary]'
                  }`}
                  title={preset.description}
                >
                  {preset.name}
                </button>
              ))}
            </div>
          </div>

          {/* Token ID 输入 */}
          <div className="space-y-2">
            <label className="text-xs text-[--text-muted]">Token ID</label>
            <input
              type="text"
              value={tokenId}
              onChange={(e) => setTokenId(e.target.value)}
              placeholder="输入 Polymarket Token ID..."
              className="w-full px-3 py-2 text-xs bg-[--bg-tertiary] border border-[--border-color] rounded text-[--text-primary] placeholder:text-[--text-muted] font-mono"
            />
            {tokenId && (
              <div className="text-[10px] text-[--text-muted] font-mono break-all">
                {tokenId.slice(0, 30)}...{tokenId.slice(-10)}
              </div>
            )}
          </div>

          {/* 下单参数 */}
          <div className="grid grid-cols-2 gap-4">
            {/* Side */}
            <div className="space-y-2">
              <label className="text-xs text-[--text-muted]">Side</label>
              <div className="flex gap-2">
                <button
                  onClick={() => setSide('buy')}
                  className={`flex-1 py-2 text-xs rounded ${
                    side === 'buy'
                      ? 'bg-green-500/20 text-green-400 border border-green-500'
                      : 'bg-[--bg-tertiary] text-[--text-secondary] border border-[--border-color]'
                  }`}
                >
                  BUY
                </button>
                <button
                  onClick={() => setSide('sell')}
                  className={`flex-1 py-2 text-xs rounded ${
                    side === 'sell'
                      ? 'bg-red-500/20 text-red-400 border border-red-500'
                      : 'bg-[--bg-tertiary] text-[--text-secondary] border border-[--border-color]'
                  }`}
                >
                  SELL
                </button>
              </div>
            </div>

            {/* Amount */}
            <div className="space-y-2">
              <label className="text-xs text-[--text-muted]">Amount (USDC)</label>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => setAmount(Math.max(0.1, amount - 1))}
                  className="w-8 h-8 rounded bg-[--bg-tertiary] text-[--text-secondary] hover:bg-[--bg-primary] border border-[--border-color]"
                >
                  -
                </button>
                <input
                  type="number"
                  min={0.1}
                  step={0.1}
                  value={amount}
                  onChange={(e) => setAmount(Math.max(0.1, parseFloat(e.target.value) || 0.1))}
                  className="flex-1 h-8 px-2 text-center text-xs bg-[--bg-tertiary] border border-[--border-color] rounded text-[--text-primary]"
                />
                <button
                  onClick={() => setAmount(amount + 1)}
                  className="w-8 h-8 rounded bg-[--bg-tertiary] text-[--text-secondary] hover:bg-[--bg-primary] border border-[--border-color]"
                >
                  +
                </button>
              </div>
              <div className="flex gap-1">
                {[0.5, 1, 2, 5, 10].map((v) => (
                  <button
                    key={v}
                    onClick={() => setAmount(v)}
                    className={`flex-1 py-1 text-[10px] rounded ${
                      amount === v
                        ? 'bg-purple-500/20 text-purple-400'
                        : 'bg-[--bg-tertiary] text-[--text-muted] hover:text-[--text-secondary]'
                    }`}
                  >
                    ${v}
                  </button>
                ))}
              </div>
            </div>
          </div>

          {/* 下单按钮 */}
          <button
            onClick={handleOrder}
            disabled={loading || !tokenId.trim()}
            className="w-full py-3 text-sm font-medium rounded bg-gradient-to-r from-purple-500 to-violet-500 text-white hover:from-purple-600 hover:to-violet-600 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {loading ? '执行中...' : `🚀 执行 ${side.toUpperCase()} $${amount}`}
          </button>

          {/* 日志区域 */}
          <div className="bg-[--bg-tertiary] rounded">
            <div className="flex items-center justify-between px-3 py-2 border-b border-[--border-color]">
              <span className="text-xs text-[--text-muted]">📋 执行日志</span>
              <button
                onClick={clearLogs}
                className="text-[10px] text-[--text-muted] hover:text-[--text-secondary]"
              >
                清空
              </button>
            </div>
            <div className="p-2 max-h-64 overflow-y-auto font-mono text-[10px] space-y-1">
              {logs.length === 0 ? (
                <div className="text-[--text-muted] text-center py-4">等待执行...</div>
              ) : (
                logs.map((log, i) => (
                  <div key={i} className="space-y-1">
                    <div className="flex gap-2">
                      <span className="text-[--text-muted] flex-shrink-0">[{log.timestamp}]</span>
                      <span
                        className={`flex-shrink-0 ${
                          log.type === 'request'
                            ? 'text-blue-400'
                            : log.type === 'response'
                            ? 'text-green-400'
                            : 'text-red-400'
                        }`}
                      >
                        [{log.type}]
                      </span>
                      <span className="text-[--text-secondary]">{log.message}</span>
                    </div>
                    {log.data && (
                      <pre className="text-[--text-muted] ml-4 whitespace-pre-wrap break-all bg-[--bg-secondary] p-1 rounded">
                        {JSON.stringify(log.data, null, 2)}
                      </pre>
                    )}
                  </div>
                ))
              )}
            </div>
          </div>

          {/* 提示信息 */}
          <div className="bg-yellow-500/10 border border-yellow-500/20 rounded p-3 text-xs text-yellow-400">
            <div className="font-medium mb-1">💡 调试提示</div>
            <ul className="list-disc list-inside space-y-0.5 text-[10px] text-yellow-400/80">
              <li>下单后请查看后端日志获取详细的签名和请求信息</li>
              <li>如果报错 "Invalid order payload"，可能是 neg_risk 设置不正确</li>
              <li>CHI-MIA 比赛的 token 可能需要特殊的 neg_risk 设置</li>
              <li>建议先用小金额 ($0.5-$1) 测试</li>
            </ul>
          </div>
        </div>
      </div>
    </div>
  );
}
