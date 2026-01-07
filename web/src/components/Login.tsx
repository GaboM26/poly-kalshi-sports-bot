import { useState, FormEvent } from 'react';

interface LoginProps {
  onLoginSuccess: (token: string, username: string) => void;
  apiBaseUrl: string;
}

export function Login({ onLoginSuccess, apiBaseUrl }: LoginProps) {
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setError('');
    setLoading(true);

    try {
      const response = await fetch(`${apiBaseUrl}/api/auth/login`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ username, password }),
      });

      if (!response.ok) {
        const data = await response.json();
        throw new Error(data.detail || '登录失败');
      }

      const data = await response.json();
      
      // 保存 token 到 localStorage
      localStorage.setItem('auth_token', data.access_token);
      localStorage.setItem('username', data.username);
      
      // 通知父组件登录成功
      onLoginSuccess(data.access_token, data.username);
    } catch (err) {
      setError(err instanceof Error ? err.message : '登录失败，请重试');
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-[--bg-primary]">
      <div className="w-full max-w-md">
        {/* Logo 和标题 */}
        <div className="text-center mb-8">
          <div className="text-5xl mb-4">🎯</div>
          <h1 className="text-2xl font-bold text-[--text-primary]">
            预测市场套利系统
          </h1>
        </div>

        {/* 登录表单 */}
        <div className="card p-8">
          <form onSubmit={handleSubmit} className="space-y-4">
            {/* 用户名 */}
            <div>
              <input
                id="username"
                type="text"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                required
                autoFocus
                disabled={loading}
                className="w-full px-4 py-3 bg-[--bg-secondary] border border-[--border-color] rounded-lg 
                         text-[--text-primary] placeholder-[--text-muted]
                         focus:outline-none focus:ring-2 focus:ring-[--accent-purple] focus:border-transparent
                         disabled:opacity-50 disabled:cursor-not-allowed"
                placeholder="用户名"
              />
            </div>

            {/* 密码 */}
            <div>
              <input
                id="password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                disabled={loading}
                className="w-full px-4 py-3 bg-[--bg-secondary] border border-[--border-color] rounded-lg 
                         text-[--text-primary] placeholder-[--text-muted]
                         focus:outline-none focus:ring-2 focus:ring-[--accent-purple] focus:border-transparent
                         disabled:opacity-50 disabled:cursor-not-allowed"
                placeholder="密码"
              />
            </div>

            {/* 错误提示 */}
            {error && (
              <div className="p-3 bg-red-500/10 border border-red-500/30 rounded-lg">
                <p className="text-sm text-red-400 text-center">{error}</p>
              </div>
            )}

            {/* 登录按钮 */}
            <button
              type="submit"
              disabled={loading || !username || !password}
              className="w-full py-3 px-4 bg-[--accent-purple] text-white font-medium rounded-lg
                       hover:bg-[--accent-purple]/90 focus:outline-none focus:ring-2 focus:ring-[--accent-purple] focus:ring-offset-2
                       disabled:opacity-50 disabled:cursor-not-allowed
                       transition-colors"
            >
              {loading ? '登录中...' : '登录'}
            </button>
          </form>
        </div>
      </div>
    </div>
  );
}
