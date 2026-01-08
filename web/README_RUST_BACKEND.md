# 前端连接 Rust 后端指南

## 概述

前端已配置为使用 Rust 后端（端口 8000），而不是 Python 后端（端口 3000）。

## 配置变更

### 1. 端口修改

- **WebSocket 端口**: `ws://localhost:8000/ws`
- **API 端口**: `http://localhost:8000`

### 2. API 端点映射

Rust 后端提供以下 API 端点：

#### 健康检查
- `GET /` - 健康检查
- `GET /api/health` - 健康检查

#### 统计和数据
- `GET /api/stats` - 获取系统统计信息
- `GET /api/data-coverage` - 获取数据覆盖率
- `GET /api/opportunities` - 获取当前套利机会
- `GET /api/matched-markets` - 获取匹配的市场
- `GET /api/arbitrage-history?limit=100` - 获取套利历史记录

#### 账户信息
- `GET /api/balance/kalshi` - 获取 Kalshi 余额
- `GET /api/balance/polymarket` - 获取 Polymarket 余额

#### 订单操作
- `POST /api/order/kalshi` - 下 Kalshi 订单
  ```json
  {
    "ticker": "KXNBAGAME-...",
    "side": "yes",
    "outcome": "yes",
    "count": 10,
    "price": 50
  }
  ```

- `POST /api/order/polymarket` - 下 Polymarket 订单
  ```json
  {
    "token_id": "...",
    "side": "buy",
    "amount": 10.0
  }
  ```

- `POST /api/arbitrage/execute` - 执行套利交易
  ```json
  {
    "event_name": "LAL-MEM",
    "team_name": "LAL",
    "kalshi_side": "yes",
    "polymarket_side": "no",
    "amount": 100.0
  }
  ```

#### WebSocket
- `GET /ws` - WebSocket 连接，用于实时更新

## WebSocket 消息格式

Rust 后端发送以下类型的 WebSocket 消息：

### 1. 套利机会（单个）
```json
{
  "type": "opportunity",
  "data": {
    "event_name": "LAL-MEM",
    "team_name": "LAL",
    "profit_margin": 5.2,
    "expected_profit": 10.5,
    ...
  }
}
```

### 2. 套利机会列表
```json
{
  "type": "opportunities",
  "data": [...]
}
```

### 3. 系统统计
```json
{
  "type": "stats",
  "data": {
    "total_kalshi_markets": 20,
    "total_polymarket_markets": 10,
    "matched_markets": 15,
    "arbitrage_opportunities": 3,
    "kalshi_ws_connected": true,
    "polymarket_ws_connected": true
  }
}
```

### 4. 日志消息
```json
{
  "type": "log",
  "level": "info",
  "message": "连接成功"
}
```

## 启动步骤

### 1. 启动 Rust 后端

```bash
cd rust-backend
cargo run --release
```

服务器将在 `http://0.0.0.0:8000` 启动。

### 2. 启动前端

```bash
cd web
npm install
npm run dev
```

前端将在 `http://localhost:5173` 启动，并自动连接到 Rust 后端。

## 数据流程

1. **初始化**: Rust 后端启动时自动获取 Kalshi 和 Polymarket 的市场数据
2. **匹配**: 自动匹配两个平台的市场
3. **WebSocket 连接**: 建立与两个平台的 WebSocket 连接，接收实时价格更新
4. **套利计算**: 实时计算套利机会
5. **推送更新**: 通过 WebSocket 将套利机会推送到前端

## 主要区别（vs Python 后端）

| 特性 | Python 后端 | Rust 后端 |
|------|------------|----------|
| 端口 | 3000 | 8000 |
| WebSocket 消息 | `matched_markets_list` | `opportunity`, `opportunities`, `stats` |
| 性能 | 较慢 | 更快 |
| 内存占用 | 较高 | 较低 |
| 并发处理 | 有限 | 优秀 |

## 注意事项

1. **认证**: Rust 后端暂时不需要认证，前端已禁用登录功能，可以直接访问
2. **订单管理**: Rust 后端的订单管理功能正在开发中，部分 API 端点可能尚未实现
3. **历史记录**: Rust 后端使用 SQLite 存储套利历史记录
4. **账户余额**: 前端会尝试获取账户余额，如果 API 不可用会静默失败

## 故障排除

### WebSocket 无法连接
- 确保 Rust 后端正在运行
- 检查端口 8000 是否被占用
- 查看浏览器控制台的错误信息

### API 调用失败
- 确认 API 端点是否正确
- 检查 Rust 后端的日志输出
- 验证请求格式是否符合 Rust 后端的要求

### 数据不更新
- 检查 WebSocket 连接状态
- 确认 Kalshi 和 Polymarket 的 WebSocket 连接是否正常
- 查看 Rust 后端的日志，确认是否接收到价格更新
