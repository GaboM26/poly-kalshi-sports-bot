# Polytaoli Rust Backend

高性能预测市场套利扫描系统 - Rust 实现

## 快速开始

### 1. 配置

复制配置模板并填入你的 API 凭据：

```bash
cp config.example.toml config.toml
```

编辑 `config.toml`，填入：
- **Kalshi**: API Key 和 RSA 私钥（从 https://kalshi.com/profile/api 获取）
- **Polymarket**: 私钥和钱包地址（从 https://reveal.magic.link/polymarket 获取）

### 2. 编译

```bash
cargo build --release
```

### 3. 运行

```bash
cargo run --release
```

服务器将在 `http://0.0.0.0:8000` 启动。

### 日志配置

系统使用分层日志系统：

- **控制台**: 只显示 `info` 和 `error` 级别的重要日志
- **文件**: `logs/polytaoli.log` 记录所有 `debug` 及以上级别的详细日志

查看实时日志：
```bash
tail -f logs/polytaoli.log
```

更多日志配置详情，请参考 [LOGGING.md](./LOGGING.md)。

## API 端点

### 健康检查
- `GET /` - 健康检查
- `GET /api/health` - 健康检查

### 数据查询
- `GET /api/stats` - 系统统计信息
- `GET /api/data-coverage` - 数据覆盖率
- `GET /api/opportunities` - 当前套利机会
- `GET /api/matched-markets` - 匹配的市场
- `GET /api/arbitrage-history?limit=100` - 套利历史

### 账户信息
- `GET /api/balance/kalshi` - Kalshi 账户余额
- `GET /api/balance/polymarket` - Polymarket 账户余额

### 交易操作
- `POST /api/order/kalshi` - 在 Kalshi 下单
- `POST /api/order/polymarket` - 在 Polymarket 下单
- `POST /api/arbitrage/execute` - 执行套利交易

### WebSocket
- `WS /ws` - 实时套利机会推送

## 项目结构

```
rust-backend/
├── Cargo.toml              # 项目配置
├── config.example.toml     # 配置模板
├── config.toml            # 实际配置（不提交到 Git）
└── src/
    ├── main.rs            # 程序入口
    ├── config/            # 配置管理
    ├── models/            # 数据模型
    ├── core/              # 核心逻辑（计算器、匹配器）
    ├── clob/              # Polymarket CLOB 客户端
    ├── clients/           # 平台客户端（Kalshi、Polymarket）
    ├── services/          # 服务层（WebSocket 管理、存储）
    └── api/               # HTTP/WebSocket API
```

## 核心特性

### 1. 完整的 Polymarket CLOB 实现
- ✅ EIP-712 签名（L1 认证）
- ✅ HMAC 签名（L2 认证）
- ✅ 订单构建和签名
- ✅ 支持 Magic Link 钱包

### 2. 精确的市场匹配
- ✅ 2:1 匹配（2 个 Kalshi 市场 → 1 个 Polymarket 市场）
- ✅ 正确处理 Yes/No 价格语义
- ✅ 每个市场订阅 3 个数据源

### 3. 实时套利计算
- ✅ WebSocket 实时价格更新
- ✅ 自动计算 Kalshi 交易手续费
- ✅ 高利润机会追踪（>3%）

### 4. 高性能
- ✅ Rust 原生性能
- ✅ 异步 I/O（Tokio）
- ✅ 并发 WebSocket 连接
- ✅ 内存高效的数据结构

## 测试

运行所有测试：

```bash
cargo test
```

运行特定测试：

```bash
cargo test test_kalshi_fee_calculation
```

## 性能对比

相比 Python 版本：
- **计算性能**: 10-50x 提升
- **延迟**: 50-70% 降低
- **吞吐量**: 5-10x 提升
- **内存使用**: 30-50% 降低

## 开发

### 编译检查

```bash
cargo check
```

### 格式化代码

```bash
cargo fmt
```

### 代码检查

```bash
cargo clippy
```

## 许可证

与主项目相同
