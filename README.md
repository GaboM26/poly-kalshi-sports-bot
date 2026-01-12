# Polytaoli - 预测市场套利扫描器

<div align="center">

**高性能预测市场套利机会实时监控系统**

[![Rust](https://img.shields.io/badge/Rust-1.70+-orange.svg)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-18.3-blue.svg)](https://reactjs.org/)
[![TypeScript](https://img.shields.io/badge/TypeScript-5.4-blue.svg)](https://www.typescriptlang.org/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

</div>

## 📖 项目简介

Polytaoli 是一个专为 **Kalshi** 和 **Polymarket** 两大预测市场平台设计的高性能套利扫描器。通过实时监控两个平台的市场价格差异，自动计算套利机会，并提供自动化交易功能。

### 核心特性

- 🚀 **高性能架构**: Rust 后端 + React 前端，WebSocket 实时通信
- 📊 **实时监控**: 同时监控 Kalshi 和 Polymarket 的市场数据
- 💰 **智能套利**: 自动匹配相同事件，计算套利机会和预期收益
- 🤖 **自动交易**: 支持自动下单执行套利策略（可配置）
- 📈 **数据追踪**: 完整的套利历史记录和性能指标
- 🔔 **Telegram 通知**: 自动交易通知推送
- 🎨 **现代化 UI**: 实时更新的交互式界面，支持深色模式
- 🔐 **安全认证**: JWT 身份验证保护 API 访问

## 🏗️ 技术架构

### 后端技术栈

- **语言**: Rust (Edition 2021)
- **Web 框架**: Axum 0.7 (异步 HTTP + WebSocket)
- **异步运行时**: Tokio (全功能异步运行时)
- **HTTP 客户端**: Reqwest (支持 JSON + TLS)
- **数据库**: SQLite (rusqlite)
- **加密签名**: 
  - Alloy (以太坊签名，用于 Polymarket)
  - RSA (Kalshi API 签名)
- **日志**: tracing + tracing-subscriber (结构化日志)

### 前端技术栈

- **框架**: React 18.3 + TypeScript 5.4
- **构建工具**: Vite 5.1
- **样式**: Tailwind CSS 3.4
- **图表**: Recharts 2.12
- **图标**: Lucide React
- **实时通信**: WebSocket API

### 核心模块

```
polytaoli/
├── rust-backend/          # Rust 后端
│   ├── src/
│   │   ├── api/          # HTTP 路由和 WebSocket 服务
│   │   ├── clients/      # Kalshi 和 Polymarket API 客户端
│   │   ├── clob/         # Polymarket CLOB 订单系统
│   │   ├── core/         # 套利计算和市场匹配逻辑
│   │   ├── services/     # 业务服务层
│   │   │   ├── arbitrage.rs      # 套利服务协调器
│   │   │   ├── websocket_manager.rs  # WebSocket 管理
│   │   │   ├── storage.rs        # 数据持久化
│   │   │   ├── metrics.rs        # 性能监控
│   │   │   └── telegram.rs       # Telegram 通知
│   │   ├── models/       # 数据模型
│   │   └── config/       # 配置管理
│   └── config.toml       # 配置文件
│
└── web/                  # React 前端
    ├── src/
    │   ├── components/   # React 组件
    │   │   ├── OpportunityList.tsx    # 套利机会列表
    │   │   ├── TrackingPanel.tsx      # 追踪面板
    │   │   ├── OrderPanel.tsx         # 持仓管理
    │   │   ├── ArbitrageHistory.tsx   # 历史记录
    │   │   └── MetricsPanel.tsx       # 性能监控
    │   ├── hooks/        # 自定义 Hooks
    │   └── types/        # TypeScript 类型定义
    └── package.json
```

## 🚀 快速开始

### 环境要求

- **Rust**: 1.70+ (推荐使用 rustup 安装)
- **Node.js**: 16+ 和 npm
- **操作系统**: macOS / Linux / Windows

### 1. 克隆项目

```bash
git clone <repository-url>
cd polytaoli
```

### 2. 配置后端

```bash
cd rust-backend
cp config.example.toml config.toml
```

编辑 `config.toml`，填入你的 API 凭据：

```toml
[kalshi]
api_key = "your-kalshi-api-key"
api_secret = """-----BEGIN RSA PRIVATE KEY-----
YOUR_PRIVATE_KEY_HERE
-----END RSA PRIVATE KEY-----"""
base_url = "https://api.elections.kalshi.com/trade-api/v2"

[polymarket]
# Magic Link 用户只需配置这两项
private_key = "0xYOUR_PRIVATE_KEY_HERE"
wallet_address = "0xYOUR_WALLET_ADDRESS_HERE"
base_url = "https://gamma-api.polymarket.com"
clob_url = "https://clob.polymarket.com"
signature_type = 1

[auth]
username = "admin"
password = "admin123"
secret_key = "your-secret-key-change-this-in-production-min-32-chars-long"

[settings]
refresh_interval = 5
min_profit_margin = 1.0
default_bet_amount = 10.0
tracking_threshold = 2.0

[auto_trade]
enabled = false
max_amount = 10.0
max_trade_count = 2
min_duration_ms = 500

[telegram]
enabled = false
bot_token = "YOUR_BOT_TOKEN"
chat_id = "YOUR_CHAT_ID"
```

### 3. 一键启动（推荐）

使用提供的启动脚本：

```bash
./start_rust_stack.sh
```

该脚本会自动：
- 启动 Rust 后端（端口 8000）
- 安装前端依赖（如需要）
- 启动前端开发服务器（端口 5173）

### 4. 手动启动

**启动后端：**

```bash
cd rust-backend
cargo run --release
```

**启动前端：**

```bash
cd web
npm install
npm run dev
```

### 5. 访问应用

- **前端界面**: http://localhost:5173
- **后端 API**: http://localhost:8000
- **健康检查**: http://localhost:8000/api/health

默认登录凭据：
- 用户名: `admin`
- 密码: `admin123`

## 📦 生产部署

### Linux 服务器部署

使用构建脚本生成部署包：

```bash
./build_linux.sh
```

这会生成一个包含所有必要文件的 tar.gz 包。

**部署步骤：**

```bash
# 1. 上传到服务器
scp polytaoli-linux-x86_64-*.tar.gz user@server:/path/

# 2. 解压
tar -xzf polytaoli-linux-x86_64-*.tar.gz
cd deploy

# 3. 配置
cp config.example.toml config.toml
nano config.toml  # 编辑配置

# 4. 启动
./start.sh
```

### Windows 部署

```bash
./build_windows.sh
```

生成的 Windows 可执行文件位于 `deploy/` 目录。

### Docker 部署（可选）

```dockerfile
FROM rust:1.70 as builder
WORKDIR /app
COPY rust-backend ./
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/polytaoli .
COPY rust-backend/config.toml .
EXPOSE 8000
CMD ["./polytaoli"]
```

## 🎯 核心功能

### 1. 实时套利监控

- 同时监控 Kalshi 和 Polymarket 的市场数据
- 智能匹配相同事件的不同市场
- 实时计算套利机会和预期收益
- 支持 Yes/No 双向套利策略

### 2. 套利计算

系统自动计算以下指标：

- **利润率**: 考虑手续费后的净利润百分比
- **预期收益**: 基于默认下注金额的预期利润
- **最优策略**: 自动选择 Yes-Yes、Yes-No、No-Yes、No-No 策略
- **订单簿深度**: 实时显示可用流动性

### 3. 自动交易

- 可配置的自动下单功能
- 支持设置最大金额和执行次数
- 持续时间阈值过滤（避免瞬时价格波动）
- Telegram 通知推送

### 4. 数据追踪

- **套利历史**: 完整的套利机会历史记录
- **持仓管理**: 实时查看 Kalshi 和 Polymarket 持仓
- **性能指标**: API 延迟、数据覆盖率、更新频率
- **历史探索**: 可视化历史数据分析

### 5. 市场匹配逻辑

系统使用多层匹配策略：

1. **精确匹配**: 基于事件名称和问题描述
2. **模糊匹配**: 使用关键词和时间范围
3. **NBA 特殊处理**: 针对 NBA 比赛的智能匹配
4. **手动映射**: 支持自定义市场映射

## 🔧 配置说明

### 数据刷新设置

```toml
[settings]
refresh_interval = 5          # 数据刷新间隔（秒）
min_profit_margin = 1.0       # 最小利润率阈值（%）
default_bet_amount = 10.0     # 默认下注金额（美元）
tracking_threshold = 2.0      # 追踪记录阈值（%）
```

### 自动交易设置

```toml
[auto_trade]
enabled = false               # 是否启用自动交易
max_amount = 10.0            # 单次最大金额
max_trade_count = 2          # 最大执行次数
min_duration_ms = 500        # 最小持续时间（毫秒）
```

### Telegram 通知

```toml
[telegram]
enabled = false
bot_token = "YOUR_BOT_TOKEN"
chat_id = "YOUR_CHAT_ID"
```

获取 Telegram Bot Token：
1. 与 @BotFather 对话创建 bot
2. 获取 bot token
3. 将 bot 添加到群组
4. 访问 `https://api.telegram.org/bot<TOKEN>/getUpdates` 获取 chat_id

## 📊 API 文档

### WebSocket 连接

```javascript
const ws = new WebSocket('ws://localhost:8000/ws');

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  // data.matched_markets: 匹配的市场列表
  // data.stats: 统计信息
  // data.metrics: 性能指标
};
```

### REST API 端点

| 端点 | 方法 | 描述 |
|------|------|------|
| `/api/health` | GET | 健康检查 |
| `/api/login` | POST | 用户登录 |
| `/api/settings` | GET/PUT | 获取/更新设置 |
| `/api/auto-trade` | GET/PUT | 自动交易配置 |
| `/api/positions/kalshi` | GET | Kalshi 持仓 |
| `/api/positions/polymarket` | GET | Polymarket 持仓 |
| `/api/arbitrage/history` | GET | 套利历史 |
| `/api/orderbook/depth` | GET | 订单簿深度 |
| `/api/order/kalshi` | POST | Kalshi 下单 |
| `/api/order/polymarket` | POST | Polymarket 下单 |

## 🔐 安全建议

1. **修改默认密码**: 部署前务必修改 `config.toml` 中的用户名和密码
2. **保护 API 密钥**: 不要将包含真实密钥的配置文件提交到版本控制
3. **使用 HTTPS**: 生产环境建议使用反向代理（如 Nginx）配置 HTTPS
4. **JWT 密钥**: 使用强随机字符串作为 JWT 密钥（至少 32 字符）
5. **防火墙**: 限制 8000 端口的访问来源

## 📝 日志管理

日志文件位于 `rust-backend/logs/` 目录：

- **文件名**: `polytaoli.log.YYYY-MM-DD`
- **轮转**: 每天自动轮转
- **级别**: INFO 及以上级别
- **内容**: 包含线程 ID、行号、时间戳

查看实时日志：

```bash
tail -f rust-backend/logs/polytaoli.log
```

## 🐛 故障排查

### 后端无法启动

1. 检查配置文件是否存在且格式正确
2. 验证 API 密钥是否有效
3. 确认端口 8000 未被占用

### WebSocket 连接失败

1. 确认后端正在运行
2. 检查防火墙设置
3. 验证前端配置的 WebSocket URL

### 市场数据不更新

1. 检查网络连接
2. 验证 API 密钥权限
3. 查看日志文件中的错误信息

### 自动交易未执行

1. 确认 `auto_trade.enabled = true`
2. 检查利润率是否达到阈值
3. 验证账户余额是否充足
4. 查看 Telegram 通知（如已启用）

## 🤝 贡献指南

欢迎提交 Issue 和 Pull Request！

## 📄 许可证

本项目采用 MIT 许可证。

## 🙏 致谢

- [Kalshi](https://kalshi.com/) - 预测市场平台
- [Polymarket](https://polymarket.com/) - 去中心化预测市场
- Rust 和 React 社区的优秀工具和库

## 📧 联系方式

如有问题或建议，请通过 Issue 联系。

---

**免责声明**: 本软件仅供学习和研究使用。使用本软件进行交易的风险由用户自行承担。请遵守相关平台的服务条款和当地法律法规。
