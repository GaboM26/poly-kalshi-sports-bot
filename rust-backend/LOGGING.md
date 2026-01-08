# 日志配置说明

## 概述

Rust 后端使用分层日志系统，将不同级别的日志输出到不同的目标：

- **控制台**: 只显示 `info` 和 `error` 级别的重要日志
- **文件**: 记录所有 `debug` 及以上级别的详细日志

## 日志级别

从低到高：
1. `trace` - 最详细的跟踪信息（未启用）
2. `debug` - 调试信息（仅文件）
3. `info` - 一般信息（控制台 + 文件）
4. `warn` - 警告信息（控制台 + 文件）
5. `error` - 错误信息（控制台 + 文件）

## 日志输出

### 控制台日志
- **级别**: `info` 及以上
- **格式**: 简洁格式，不显示目标模块、线程信息
- **用途**: 显示系统运行状态和重要事件

示例输出：
```
2024-01-08T10:30:00.123Z  INFO 🚀 启动 Polytaoli - 预测市场套利扫描器
2024-01-08T10:30:00.456Z  INFO 📝 日志文件: logs/polytaoli.log
2024-01-08T10:30:01.789Z  INFO ✅ 配置文件加载完成
2024-01-08T10:30:02.012Z  INFO 🔍 正在从两个平台获取市场数据...
```

### 文件日志
- **位置**: `logs/polytaoli.log`
- **级别**: `debug` 及以上
- **格式**: 详细格式，包含时间戳、级别、目标模块、线程 ID、行号
- **轮转**: 每天自动创建新文件（如 `polytaoli.log.2024-01-08`）
- **用途**: 详细的调试信息和问题排查

示例输出：
```
2024-01-08T10:30:00.123456Z  INFO polytaoli: 🚀 启动 Polytaoli - 预测市场套利扫描器
2024-01-08T10:30:00.456789Z  INFO polytaoli: 📝 日志文件: logs/polytaoli.log
2024-01-08T10:30:01.234567Z DEBUG polytaoli::services::websocket_manager: [Kalshi] 价格更新: KXNBAGAME-... - Yes: 0.45/0.46, No: 0.54/0.55 thread_id=ThreadId(3) line=142
2024-01-08T10:30:01.345678Z DEBUG polytaoli::services::websocket_manager: [Kalshi] 影响 2 个匹配市场 thread_id=ThreadId(3) line=149
2024-01-08T10:30:01.456789Z DEBUG polytaoli::services::websocket_manager: [计算] 开始计算套利: LAL-MEM - LAL thread_id=ThreadId(3) line=252
```

## 日志内容

### Info 级别日志（控制台可见）

#### 启动和初始化
- 🚀 系统启动
- 📝 日志文件位置
- ✅ 配置加载完成
- 🔍 开始获取市场数据
- 📊 数据加载统计
- ✅ 初始化完成
- 📡 WebSocket 连接启动
- 🌐 服务器监听地址

#### 运行时事件
- ✅ [Kalshi] 开始接收实时价格数据
- ✅ [Polymarket] 开始接收实时价格数据
- 📈 开始跟踪套利机会
- 📉 跟踪结束
- 📊 定期扫描结果

#### WebSocket 和网络
- 新的 WebSocket 客户端已连接
- WebSocket 客户端已断开连接
- 正在连接 Kalshi WebSocket...
- 已订阅 X 个 Kalshi 市场
- 正在连接 Polymarket WebSocket...
- 已订阅 X 个 Polymarket 代币

#### 错误和警告
- ❌ 各种错误信息
- ⚠️  警告信息

### Debug 级别日志（仅文件）

#### 价格更新
- `[Kalshi] 价格更新: {market_id} - Yes: {bid}/{ask}, No: {bid}/{ask}`
- `[Polymarket] 价格更新: {token_id} - Price: {price}`
- `[Kalshi] 影响 X 个匹配市场`

#### 套利计算
- `[计算] 市场 X 数据未就绪`
- `[计算] 开始计算套利: {event_name} - {team_name}`

#### 市场匹配
- 日期不匹配详情
- 缺少日期警告
- 匹配置信度信息

## 配置自定义

### 环境变量方式

可以通过 `RUST_LOG` 环境变量覆盖默认配置：

```bash
# 控制台显示 debug 日志
RUST_LOG=polytaoli=debug cargo run

# 只显示错误
RUST_LOG=polytaoli=error cargo run

# 显示特定模块的 debug 日志
RUST_LOG=polytaoli::services::websocket_manager=debug cargo run
```

### 代码修改方式

编辑 `src/main.rs` 中的日志配置：

```rust
// 控制台日志层 - 修改过滤级别
let console_layer = fmt::layer()
    .with_target(false)
    .with_thread_ids(false)
    .with_thread_names(false)
    .with_filter(EnvFilter::new("polytaoli=debug,tower_http=warn")); // 改为 debug

// 文件日志层 - 修改过滤级别
let file_layer = fmt::layer()
    .with_writer(non_blocking_file)
    .with_ansi(false)
    .with_target(true)
    .with_thread_ids(true)
    .with_line_number(true)
    .with_filter(EnvFilter::new("polytaoli=trace,tower_http=debug")); // 改为 trace
```

## 日志文件管理

### 自动轮转
- 日志文件每天自动轮转
- 旧文件格式: `polytaoli.log.YYYY-MM-DD`
- 当前文件: `polytaoli.log`

### 手动清理
```bash
# 删除 7 天前的日志
find logs/ -name "polytaoli.log.*" -mtime +7 -delete

# 压缩旧日志
gzip logs/polytaoli.log.2024-01-*
```

### 日志大小监控
```bash
# 查看日志文件大小
du -h logs/

# 查看最新日志
tail -f logs/polytaoli.log

# 搜索特定内容
grep "套利机会" logs/polytaoli.log
```

## 故障排查

### 日志文件未创建
1. 检查 `logs/` 目录是否存在
2. 检查文件写入权限
3. 查看控制台是否有错误信息

### 日志内容过多
1. 调整文件日志级别为 `info`
2. 禁用特定模块的 debug 日志
3. 定期清理旧日志文件

### 日志内容过少
1. 检查日志级别配置
2. 确认 `RUST_LOG` 环境变量未覆盖配置
3. 查看是否有日志被过滤

## 性能考虑

- **异步写入**: 使用 `non_blocking` appender，不会阻塞主线程
- **文件缓冲**: 自动批量写入，减少 I/O 操作
- **控制台过滤**: 只显示重要日志，减少输出开销
- **日志轮转**: 自动管理文件大小，避免单个文件过大

## 最佳实践

1. **开发环境**: 使用 `RUST_LOG=debug` 查看详细信息
2. **生产环境**: 使用默认配置（控制台 info，文件 debug）
3. **问题排查**: 查看文件日志获取详细的调试信息
4. **性能监控**: 定期检查日志文件大小和磁盘使用
5. **日志归档**: 定期备份和清理旧日志文件
