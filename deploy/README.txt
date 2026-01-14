Polytaoli - 预测市场套利扫描器
================================

部署步骤:
1. 复制 config.example.toml 为 config.toml
2. 编辑 config.toml，填入 Kalshi API 密钥
3. 复制 poly-order-service/config.toml.sample 为 poly-order-service/config.toml
4. 编辑 poly-order-service/config.toml，填入 Polymarket 私钥和钱包地址
5. 运行: ./start.sh

配置说明:
- Rust 后端端口: 8000
- Python 下单服务端口: 8001
- 日志: 保存在 logs/ 目录
- 前端: 访问 http://your-server:8000

服务架构:
- Rust 后端: 处理套利扫描、WebSocket、API
- Python 服务: 处理 Polymarket 下单（使用官方 SDK）

停止程序: Ctrl+C 或 kill 进程
