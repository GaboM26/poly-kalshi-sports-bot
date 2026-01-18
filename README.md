# Polytaoli - 预测市场套利扫描器

高性能预测市场套利机会实时监控系统，支持 **Kalshi** 和 **Polymarket** 平台。

## 🎯 核心特性

- 🚀 **高性能**: Rust 后端 + React 前端 + WebSocket 实时通信
- 📊 **实时监控**: 双平台市场数据同步，智能事件匹配
- 💰 **自动套利**: 计算利润率、预期收益，支持自动下单
- 📈 **数据追踪**: 套利历史、持仓管理、性能监控
- 🔔 **Telegram 通知**: 自动交易推送

## 🏗️ 系统架构

```
┌─────────────────────────────────────────────────────────────┐
│                      前端 (React + TS)                       │
│                    http://localhost:5173                     │
│  - 实时套利机会展示   - 持仓管理   - 历史数据分析            │
└────────────────────┬────────────────────────────────────────┘
                     │ WebSocket + REST API
┌────────────────────┴────────────────────────────────────────┐
│              Rust 后端 (Axum + Tokio)                        │
│                http://localhost:8000                         │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ 核心服务                                              │   │
│  │  - ArbitrageService: 套利计算与协调                   │   │
│  │  - WebSocketManager: 实时数据推送                     │   │
│  │  - EventMatcher: 市场智能匹配                         │   │
│  │  - ArbitrageCalculator: 利润率计算                    │   │
│  │  - Storage: SQLite 数据持久化                         │   │
│  └──────────────────────────────────────────────────────┘   │
│  ┌──────────────────┐          ┌──────────────────┐         │
│  │ Kalshi Client    │          │ Polymarket Client│         │
│  │ - REST API       │          │ - REST API       │         │
│  │ - WebSocket      │          │ - WebSocket      │         │
│  │ - RSA 签名       │          │ - 以太坊签名     │         │
│  └──────────────────┘          └──────────────────┘         │
└────────────────────┬────────────────────────────────────────┘
                     │ HTTP API
┌────────────────────┴────────────────────────────────────────┐
│         Python 下单服务 (FastAPI)                            │
│              http://localhost:8001                           │
│  - 使用官方 py-clob-client SDK                               │
│  - 处理 Polymarket CLOB 订单签名和提交                       │
└─────────────────────────────────────────────────────────────┘
```

### 技术栈

**后端 (Rust)**
- Axum 0.7 - Web 框架
- Tokio - 异步运行时
- SQLite - 数据存储
- Reqwest - HTTP 客户端
- Alloy/RSA - 加密签名

**前端 (React)**
- React 18 + TypeScript 5
- Vite 5 - 构建工具
- Tailwind CSS - 样式
- Recharts - 数据可视化

**下单服务 (Python)**
- FastAPI - Web 框架
- py-clob-client - Polymarket 官方 SDK

## 🚀 快速启动

### 1. 环境要求

- Rust 1.70+
- Node.js 16+
- Python 3.8+

### 2. 配置

```bash
cd rust-backend
cp config.example.toml config.toml
```

编辑 `config.toml`：

```toml
[kalshi]
api_key = "your-kalshi-api-key"
api_secret = """-----BEGIN RSA PRIVATE KEY-----
YOUR_PRIVATE_KEY_HERE
-----END RSA PRIVATE KEY-----"""

[polymarket]
# 从 https://reveal.magic.link/polymarket 获取
private_key = "0xYOUR_PRIVATE_KEY"
wallet_address = "0xYOUR_WALLET_ADDRESS"
# Python 下单服务地址
order_service_url = "http://127.0.0.1:8001"

[auth]
username = "admin"
password = "admin123"
secret_key = "your-secret-key-min-32-chars"

[settings]
refresh_interval = 5          # 刷新间隔（秒）
min_profit_margin = 1.0       # 最小利润率（%）
default_bet_amount = 10.0     # 默认下注金额
tracking_threshold = 2.0      # 追踪阈值（%）

[auto_trade]
enabled = false               # 自动交易开关
max_amount = 10.0            # 单次最大金额
max_trade_count = 2          # 最大执行次数
min_duration_ms = 500        # 最小持续时间

[telegram]
enabled = false
bot_token = "YOUR_BOT_TOKEN"
chat_id = "YOUR_CHAT_ID"
```

### 3. 一键启动

```bash
./start_rust_stack.sh
```

自动启动：
- Python 下单服务（端口 8001）
- Rust 后端（端口 8000）
- React 前端（端口 5173）

### 4. 访问应用

- **前端**: http://localhost:5173
- **后端 API**: http://localhost:8000
- **健康检查**: http://localhost:8000/api/health

默认登录：`admin` / `admin123`

## 📦 生产部署

### Linux

```bash
./build_linux.sh
scp polytaoli-linux-x86_64-*.tar.gz user@server:/path/
tar -xzf polytaoli-linux-x86_64-*.tar.gz
cd deploy
cp config.example.toml config.toml
# 编辑配置后启动
./start.sh
```

### Windows

```bash
./build_windows.sh
```

## 🔧 核心功能

### 套利计算
- 实时计算利润率（考虑手续费）
- 自动选择最优策略（Yes-Yes/Yes-No/No-Yes/No-No）
- 订单簿深度分析

### 市场匹配
- 精确匹配：事件名称和问题描述
- 模糊匹配：关键词和时间范围
- NBA 特殊处理：智能识别比赛信息

### 自动交易
- 持续时间阈值过滤
- 可配置金额和次数限制
- Telegram 实时通知

### 数据管理
- SQLite 存储套利历史
- 实时性能指标监控
- 持仓查询和管理

## 📊 API 端点

| 端点 | 方法 | 描述 |
|------|------|------|
| `/api/health` | GET | 健康检查 |
| `/api/login` | POST | 用户登录 |
| `/api/settings` | GET/PUT | 设置管理 |
| `/api/auto-trade` | GET/PUT | 自动交易配置 |
| `/api/positions/kalshi` | GET | Kalshi 持仓 |
| `/api/positions/polymarket` | GET | Polymarket 持仓 |
| `/api/arbitrage/history` | GET | 套利历史 |
| `/api/order/kalshi` | POST | Kalshi 下单 |
| `/api/order/polymarket` | POST | Polymarket 下单 |
| `/ws` | WebSocket | 实时数据推送 |

## 🔐 安全建议

1. 修改默认密码和 JWT 密钥
2. 不要提交包含真实密钥的配置文件
3. 生产环境使用 HTTPS（Nginx 反向代理）
4. 限制端口访问来源

## 📝 日志

日志位于 `rust-backend/logs/polytaoli.log.YYYY-MM-DD`，每天自动轮转。

```bash
tail -f rust-backend/logs/polytaoli.log
```

## 🐛 故障排查

- **后端无法启动**: 检查配置文件格式和 API 密钥
- **WebSocket 断连**: 确认后端运行状态和防火墙设置
- **数据不更新**: 验证 API 密钥权限和网络连接
- **自动交易未执行**: 检查利润率阈值和账户余额

---

**免责声明**: 本软件仅供学习研究使用，交易风险自负。
