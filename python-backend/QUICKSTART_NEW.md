# 快速开始指南 - 模块化版本

## 📦 安装

### 1. 创建虚拟环境

```bash
cd python-backend
python3 -m venv venv
source venv/bin/activate  # Linux/Mac
# 或
venv\Scripts\activate  # Windows
```

### 2. 安装依赖

```bash
pip install -r requirements.txt
```

## ⚙️ 配置

编辑 `../config.toml` 文件：

```toml
[kalshi]
api_key = "your-kalshi-api-key"
api_secret = "your-kalshi-api-secret"
base_url = "https://api.elections.kalshi.com/trade-api/v2"

[polymarket]
api_key = ""
base_url = "https://gamma-api.polymarket.com"
clob_url = "https://clob.polymarket.com"

[settings]
refresh_interval = 5
min_profit_margin = 1.0
default_bet_amount = 100.0
```

## 🚀 启动

### 方式 1: 使用启动脚本（推荐）

```bash
./start.sh
```

### 方式 2: 直接运行

```bash
source venv/bin/activate
python -m app.main
```

服务器将在 `http://localhost:3000` 启动

## 🧪 测试

### 运行测试

```bash
# 安装测试依赖
pip install pytest pytest-asyncio

# 运行所有测试
pytest tests/

# 运行特定测试
pytest tests/test_matcher.py -v
```

### 使用调试脚本

```bash
# 测试 Kalshi 连接
python scripts/debug_kalshi.py

# 测试市场匹配
python scripts/debug_matching.py

# 运行演示
python scripts/demo.py
```

## 📡 API 端点

### RESTful API

- `GET /` - 服务信息
- `GET /api/stats` - 系统统计
- `GET /api/opportunities` - 套利机会列表
- `GET /api/matched-markets` - 匹配的市场
- `GET /api/tracking` - 套利追踪信息

### WebSocket

- `ws://localhost:3000/ws` - 实时数据推送

## 📊 使用示例

### 1. 获取统计信息

```bash
curl http://localhost:3000/api/stats
```

响应：
```json
{
  "total_kalshi_events": 10,
  "total_kalshi_markets": 20,
  "total_polymarket_events": 8,
  "total_polymarket_markets": 8,
  "matched_events": 7,
  "matched_markets": 14,
  "arbitrage_opportunities": 3,
  "last_update": "2026-01-04T12:00:00"
}
```

### 2. 获取套利机会

```bash
curl http://localhost:3000/api/opportunities
```

### 3. WebSocket 连接（JavaScript）

```javascript
const ws = new WebSocket('ws://localhost:3000/ws');

ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  console.log('收到消息:', data);
  
  if (data.type === 'opportunity') {
    console.log('套利机会:', data.data);
  }
};
```

## 🔧 开发

### 项目结构

```
app/
├── main.py              # 应用入口
├── api/                 # API 路由
├── core/                # 核心业务逻辑
├── clients/             # API 客户端
├── services/            # 服务层
└── utils/               # 工具函数
```

### 添加新功能

1. **添加新的 API 端点**
   - 编辑 `app/api/routes.py`
   - 添加路由处理函数

2. **添加新的客户端**
   - 在 `app/clients/` 创建新文件
   - 继承 `BaseAPIClient`

3. **添加新的业务逻辑**
   - 在 `app/services/` 创建新服务
   - 在 `app/main.py` 中集成

### 日志调试

修改 `app/main.py` 中的日志级别：

```python
logger = setup_logger("arbitrage_scanner", logging.DEBUG)
```

## 🐛 故障排查

### 问题 1: 虚拟环境未激活

**错误**: `ModuleNotFoundError: No module named 'fastapi'`

**解决**:
```bash
source venv/bin/activate
```

### 问题 2: 端口被占用

**错误**: `Address already in use`

**解决**:
```bash
# 查找占用端口的进程
lsof -i :3000

# 杀死进程
kill -9 <PID>
```

### 问题 3: Kalshi 连接失败

**错误**: `❌ Kalshi 连接失败`

**解决**:
1. 检查 `config.toml` 中的 API 密钥
2. 确认网络连接正常
3. 运行调试脚本：`python scripts/debug_kalshi.py`

### 问题 4: 导入错误

**错误**: `ImportError: attempted relative import with no known parent package`

**解决**:
使用模块方式运行：
```bash
python -m app.main
```
而不是：
```bash
python app/main.py
```

## 📚 更多信息

- 详细项目结构：查看 `PROJECT_STRUCTURE.md`
- 旧版本文档：查看 `old_files/` 目录
- 前端集成：查看 `FRONTEND_INTEGRATION.txt`

## 🎯 下一步

1. ✅ 启动服务器
2. ✅ 测试 API 端点
3. ✅ 连接前端应用
4. 🔄 添加自定义功能
5. 🔄 部署到生产环境

## 💡 提示

- 使用 `./start.sh` 快速启动
- 查看日志了解系统状态
- 使用 `scripts/` 中的工具调试
- 阅读 `PROJECT_STRUCTURE.md` 了解架构

祝你使用愉快！🚀
