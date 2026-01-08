# 从 Python 后端迁移到 Rust 后端指南

## 概述

本指南说明如何将前端从 Python 后端（端口 3000）切换到 Rust 后端（端口 8000）。

## 已完成的修改

### 1. 前端配置修改

#### `web/src/App.tsx`
- ✅ WebSocket URL 从 `ws://localhost:3000/ws` 改为 `ws://localhost:8000/ws`
- ✅ API Base URL 从 `http://localhost:3000` 改为 `http://localhost:8000`
- ✅ 禁用登录功能（Rust 后端暂时不需要认证）

#### `web/src/hooks/useWebSocket.ts`
- ✅ 适配 Rust 后端的 WebSocket 消息格式
- ✅ 支持 `opportunity`, `opportunities`, `stats` 消息类型
- ✅ 保留对旧格式的兼容性

### 2. 新增文件

- ✅ `web/README_RUST_BACKEND.md` - Rust 后端使用指南
- ✅ `start_rust_stack.sh` - 一键启动脚本
- ✅ `MIGRATION_TO_RUST.md` - 本迁移指南

## 快速开始

### 方法 1: 使用启动脚本（推荐）

```bash
# 在项目根目录
./start_rust_stack.sh
```

### 方法 2: 手动启动

#### 启动 Rust 后端
```bash
cd rust-backend
cargo run --release
```

#### 启动前端
```bash
cd web
npm install  # 首次运行
npm run dev
```

## 功能对比

| 功能 | Python 后端 | Rust 后端 | 状态 |
|------|------------|----------|------|
| 市场数据获取 | ✅ | ✅ | 完成 |
| 市场匹配 | ✅ | ✅ | 完成 |
| WebSocket 实时更新 | ✅ | ✅ | 完成 |
| 套利计算 | ✅ | ✅ | 完成 |
| 用户认证 | ✅ | ⏳ | 待实现 |
| 订单管理 | ✅ | ⏳ | 部分实现 |
| 历史记录 | ✅ | ✅ | 完成 |
| 账户余额 | ✅ | ✅ | 完成 |

## API 端点映射

### 已实现的端点

| 功能 | Python | Rust | 状态 |
|------|--------|------|------|
| 健康检查 | `/api/health` | `/api/health` | ✅ |
| 系统统计 | `/api/stats` | `/api/stats` | ✅ |
| 数据覆盖率 | `/api/data-coverage` | `/api/data-coverage` | ✅ |
| 套利机会 | `/api/opportunities` | `/api/opportunities` | ✅ |
| 匹配市场 | `/api/matched-markets` | `/api/matched-markets` | ✅ |
| 套利历史 | `/api/arbitrage-history` | `/api/arbitrage-history` | ✅ |
| Kalshi 余额 | `/api/balance/kalshi` | `/api/balance/kalshi` | ✅ |
| Polymarket 余额 | `/api/balance/polymarket` | `/api/balance/polymarket` | ✅ |
| Kalshi 下单 | `/api/order/kalshi` | `/api/order/kalshi` | ✅ |
| Polymarket 下单 | `/api/order/polymarket` | `/api/order/polymarket` | ✅ |
| 套利执行 | `/api/arbitrage/execute` | `/api/arbitrage/execute` | ✅ |

### 待实现的端点

| 功能 | Python | Rust | 状态 |
|------|--------|------|------|
| 用户登录 | `/api/auth/login` | - | ⏳ |
| 账户余额（统一） | `/api/account-balance` | - | ⏳ |
| 订单列表 | `/api/orders/*` | - | ⏳ |
| 持仓列表 | `/api/positions/*` | - | ⏳ |
| 历史搜索 | `/api/history/search` | - | ⏳ |
| 历史统计 | `/api/history/statistics` | - | ⏳ |
| 追踪信息 | `/api/tracking` | - | ⏳ |

## WebSocket 消息格式变化

### Python 后端
```json
{
  "type": "matched_markets_list",
  "data": [...],
  "count": 10,
  "opportunities_count": 3
}
```

### Rust 后端
```json
// 单个套利机会
{
  "type": "opportunity",
  "data": { ... }
}

// 套利机会列表
{
  "type": "opportunities",
  "data": [ ... ]
}

// 系统统计
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

## 注意事项

### 1. 认证功能
- **Python**: 需要登录才能访问
- **Rust**: 暂时不需要认证，前端已禁用登录页面
- **影响**: 用户可以直接访问系统，无需登录

### 2. 订单管理
- **Python**: 完整的订单管理功能
- **Rust**: 基本的下单功能已实现，订单列表和取消功能待实现
- **影响**: 前端的订单面板可能无法正常工作

### 3. 历史记录
- **Python**: 使用 JSON 文件存储
- **Rust**: 使用 SQLite 数据库存储
- **影响**: 数据格式和查询方式有所不同

### 4. 性能差异
- **Rust**: 更快的启动速度和更低的内存占用
- **Rust**: 更好的并发处理能力
- **Rust**: 更稳定的 WebSocket 连接

## 故障排除

### 问题 1: 前端无法连接到后端
**症状**: WebSocket 连接失败，显示 "WebSocket 连接断开"

**解决方案**:
1. 确认 Rust 后端正在运行: `curl http://localhost:8000/api/health`
2. 检查端口 8000 是否被占用: `lsof -i :8000`
3. 查看 Rust 后端日志，确认是否有错误

### 问题 2: 数据不更新
**症状**: 前端显示 "等待实时价格..."

**解决方案**:
1. 检查 Rust 后端日志，确认 WebSocket 连接状态
2. 确认 Kalshi 和 Polymarket 的 API 配置正确
3. 查看浏览器控制台，确认是否有 WebSocket 错误

### 问题 3: 某些功能不可用
**症状**: 订单管理、历史搜索等功能报错

**解决方案**:
- 这些功能在 Rust 后端中尚未完全实现
- 可以继续使用 Python 后端，或等待 Rust 后端功能完善

## 回滚到 Python 后端

如果需要回滚到 Python 后端，只需修改 `web/src/App.tsx`:

```typescript
// 将端口改回 3000
const wsUrl = 'ws://localhost:3000/ws';
const apiBaseUrl = 'http://localhost:3000';

// 恢复登录功能
const [isAuthenticated, setIsAuthenticated] = useState(false);
```

## 开发路线图

### 短期目标
- [ ] 实现用户认证功能
- [ ] 完善订单管理 API
- [ ] 实现持仓查询功能

### 中期目标
- [ ] 实现历史记录搜索和统计
- [ ] 添加追踪功能 API
- [ ] 优化 WebSocket 性能

### 长期目标
- [ ] 添加更多交易平台支持
- [ ] 实现自动交易功能
- [ ] 添加风险管理功能

## 反馈和支持

如果遇到问题或有建议，请：
1. 查看 Rust 后端日志
2. 查看浏览器控制台
3. 查看 `web/README_RUST_BACKEND.md` 了解更多信息
