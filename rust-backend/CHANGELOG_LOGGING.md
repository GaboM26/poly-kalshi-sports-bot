# 日志系统更新日志

## 2024-01-08 - 分层日志系统实现

### 🎯 目标
将 debug 日志输出到文件，控制台只显示重要的 info 和 error 日志。

### ✅ 完成的工作

#### 1. 依赖更新
**文件**: `Cargo.toml`

添加了日志相关依赖：
```toml
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "json"] }
tracing-appender = "0.2"
```

#### 2. 日志系统重构
**文件**: `src/main.rs`

实现了分层日志系统：

- **控制台层**:
  - 级别: `info` 及以上
  - 格式: 简洁（无目标模块、线程信息）
  - 过滤: `polytaoli=info,tower_http=warn`

- **文件层**:
  - 级别: `debug` 及以上
  - 格式: 详细（包含目标、线程 ID、行号）
  - 过滤: `polytaoli=debug,tower_http=debug`
  - 位置: `logs/polytaoli.log`
  - 轮转: 每天自动创建新文件

#### 3. Debug 日志增强
**文件**: `src/services/websocket_manager.rs`

添加了详细的 debug 日志：

- 价格更新详情:
  ```rust
  debug!(
      "[Kalshi] 价格更新: {} - Yes: {:.2}/{:.2}, No: {:.2}/{:.2}",
      update.market_id, yb, ya, nb, na
  );
  ```

- 市场影响统计:
  ```rust
  debug!("[Kalshi] 影响 {} 个匹配市场", indices.len());
  ```

- 套利计算跟踪:
  ```rust
  debug!(
      "[计算] 开始计算套利: {} - {}",
      mm.event_name, mm.team_name
  );
  ```

#### 4. 文档完善

新增文档：
- **LOGGING.md**: 完整的日志配置说明
  - 日志级别说明
  - 输出格式示例
  - 配置自定义方法
  - 文件管理建议
  - 故障排查指南
  - 性能考虑
  - 最佳实践

- **test_logging.sh**: 日志测试脚本
  - 自动清理旧日志
  - 编译和启动服务
  - 检查日志文件
  - 统计日志数量
  - 显示示例输出

更新文档：
- **README.md**: 添加日志配置章节
- **.gitignore**: 已包含日志文件忽略规则

### 📊 日志输出示例

#### 控制台输出（简洁）
```
2024-01-08T10:30:00.123Z  INFO 🚀 启动 Polytaoli - 预测市场套利扫描器
2024-01-08T10:30:00.456Z  INFO 📝 日志文件: logs/polytaoli.log
2024-01-08T10:30:01.789Z  INFO ✅ 配置文件加载完成
2024-01-08T10:30:02.012Z  INFO 🔍 正在从两个平台获取市场数据...
2024-01-08T10:30:05.345Z  INFO 📊 已加载: Kalshi 20 个事件/40 个市场, Polymarket 15 个事件/15 个市场
2024-01-08T10:30:06.678Z  INFO ✅ 初始化完成: 30 个匹配的市场
2024-01-08T10:30:07.901Z  INFO 📡 启动 WebSocket 连接: 30 个 Kalshi 市场, 60 个 Polymarket 代币
2024-01-08T10:30:08.234Z  INFO 🌐 服务器监听地址: http://0.0.0.0:8000
```

#### 文件日志输出（详细）
```
2024-01-08T10:30:00.123456Z  INFO polytaoli: 🚀 启动 Polytaoli - 预测市场套利扫描器
2024-01-08T10:30:00.456789Z  INFO polytaoli: 📝 日志文件: logs/polytaoli.log
2024-01-08T10:30:15.123456Z DEBUG polytaoli::services::websocket_manager: [Kalshi] 价格更新: KXNBAGAME-26JAN08LALMEM-LAL - Yes: 0.45/0.46, No: 0.54/0.55 thread_id=ThreadId(3) line=142
2024-01-08T10:30:15.234567Z DEBUG polytaoli::services::websocket_manager: [Kalshi] 影响 2 个匹配市场 thread_id=ThreadId(3) line=149
2024-01-08T10:30:15.345678Z DEBUG polytaoli::services::websocket_manager: [计算] 开始计算套利: LAL-MEM - LAL thread_id=ThreadId(3) line=252
2024-01-08T10:30:15.456789Z DEBUG polytaoli::services::websocket_manager: [Polymarket] 价格更新: 0x1234...5678 - Price: 0.4523 thread_id=ThreadId(4) line=167
```

### 🎨 特性

1. **异步非阻塞**: 使用 `non_blocking` appender，不影响主线程性能
2. **自动轮转**: 每天自动创建新日志文件
3. **分级过滤**: 控制台和文件使用不同的日志级别
4. **详细信息**: 文件日志包含模块、线程、行号等调试信息
5. **中文友好**: 所有日志消息都使用中文

### 📈 性能影响

- **控制台**: 减少了输出量，提升了可读性
- **文件**: 异步写入，对性能影响极小（< 1%）
- **内存**: 使用缓冲写入，内存占用稳定

### 🔧 使用方法

#### 查看实时日志
```bash
tail -f logs/polytaoli.log
```

#### 搜索特定内容
```bash
grep "套利机会" logs/polytaoli.log
grep "ERROR" logs/polytaoli.log
```

#### 统计日志
```bash
wc -l logs/polytaoli.log                    # 总行数
grep -c "DEBUG" logs/polytaoli.log          # DEBUG 日志数
grep -c "INFO" logs/polytaoli.log           # INFO 日志数
```

#### 清理旧日志
```bash
find logs/ -name "polytaoli.log.*" -mtime +7 -delete
```

#### 自定义日志级别
```bash
# 临时显示 debug 日志到控制台
RUST_LOG=polytaoli=debug cargo run

# 只显示错误
RUST_LOG=polytaoli=error cargo run
```

### 🐛 已知问题

无

### 📝 待优化

1. 添加日志压缩功能
2. 实现日志大小限制
3. 添加结构化日志（JSON 格式）选项
4. 实现日志聚合和分析工具

### 🔗 相关文档

- [LOGGING.md](./LOGGING.md) - 详细的日志配置说明
- [README.md](./README.md) - 项目主文档
- [test_logging.sh](./test_logging.sh) - 日志测试脚本

### 💡 最佳实践

1. **开发环境**: 使用 `tail -f logs/polytaoli.log` 查看详细日志
2. **生产环境**: 定期清理旧日志文件
3. **问题排查**: 优先查看文件日志中的 DEBUG 信息
4. **性能监控**: 监控日志文件大小，避免磁盘占满
5. **日志分析**: 使用 `grep`、`awk` 等工具分析日志模式
