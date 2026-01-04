# 项目结构说明

## 📁 目录结构

```
python-backend/
├── app/                          # 主应用目录
│   ├── __init__.py              # 应用包初始化
│   ├── main.py                  # FastAPI 应用入口
│   │
│   ├── api/                     # API 路由层
│   │   ├── __init__.py
│   │   ├── routes.py            # RESTful API 路由定义
│   │   └── websocket.py         # WebSocket 连接处理
│   │
│   ├── core/                    # 核心业务逻辑
│   │   ├── __init__.py
│   │   ├── config.py            # 配置管理（读取 config.toml）
│   │   ├── models.py            # 数据模型定义（Pydantic）
│   │   ├── matcher.py           # 事件和市场匹配引擎
│   │   └── calculator.py        # 套利机会计算引擎
│   │
│   ├── clients/                 # 外部 API 客户端
│   │   ├── __init__.py
│   │   ├── base.py              # 基础客户端抽象类
│   │   ├── kalshi.py            # Kalshi API 客户端
│   │   └── polymarket.py        # Polymarket API 客户端
│   │
│   ├── services/                # 服务层（业务逻辑封装）
│   │   ├── __init__.py
│   │   ├── arbitrage.py         # 套利服务（核心业务流程）
│   │   └── websocket_manager.py # WebSocket 连接管理
│   │
│   └── utils/                   # 工具函数
│       ├── __init__.py
│       ├── logger.py            # 日志配置
│       └── helpers.py           # 辅助函数
│
├── tests/                       # 测试目录
│   ├── __init__.py
│   ├── test_matcher.py          # 匹配器测试
│   └── test_calculator.py       # 计算器测试
│
├── scripts/                     # 脚本和工具
│   ├── debug_kalshi.py          # Kalshi 调试脚本
│   ├── debug_matching.py        # 匹配调试脚本
│   ├── demo.py                  # 演示脚本
│   ├── test_server.py           # 服务器测试
│   └── test_websocket.py        # WebSocket 测试
│
├── old_files/                   # 旧版本文件备份
│
├── requirements.txt             # Python 依赖
├── start.sh                     # 启动脚本
└── .gitignore                   # Git 忽略文件
```

## 🔧 模块说明

### 1. 核心模块 (`app/core/`)

#### `config.py` - 配置管理
- 从 `config.toml` 读取配置
- 定义 Kalshi、Polymarket 和应用设置
- 提供配置验证

#### `models.py` - 数据模型
- 定义所有数据结构（使用 Pydantic）
- 包括：事件、市场、匹配结果、套利机会等
- 提供数据验证和序列化

#### `matcher.py` - 匹配引擎
- 两阶段匹配算法
  - 第一阶段：事件匹配（基于队伍名称和日期）
  - 第二阶段：市场匹配（2:1 匹配 Kalshi 到 Polymarket）
- 生成 WebSocket 订阅信息

#### `calculator.py` - 套利计算
- 计算套利机会
- 验证价格有效性
- 计算最优下注金额和预期利润

### 2. 客户端模块 (`app/clients/`)

#### `base.py` - 基础客户端
- 提供通用 HTTP 客户端功能
- 定义客户端接口规范

#### `kalshi.py` - Kalshi 客户端
- Kalshi API 集成
- 认证和市场数据获取
- WebSocket 价格订阅

#### `polymarket.py` - Polymarket 客户端
- Polymarket API 集成
- 市场数据获取
- WebSocket 价格订阅

### 3. 服务层 (`app/services/`)

#### `arbitrage.py` - 套利服务
- 封装核心业务流程
- 协调各个模块
- 管理数据状态

#### `websocket_manager.py` - WebSocket 管理
- 管理双平台 WebSocket 连接
- 实时价格更新处理
- 套利机会追踪

### 4. API 层 (`app/api/`)

#### `routes.py` - RESTful API
- `/` - 服务信息
- `/api/stats` - 统计信息
- `/api/opportunities` - 套利机会列表
- `/api/matched-markets` - 匹配的市场
- `/api/tracking` - 套利追踪

#### `websocket.py` - WebSocket 处理
- `/ws` - WebSocket 连接端点
- 实时推送套利机会
- 广播系统日志

### 5. 工具模块 (`app/utils/`)

#### `logger.py` - 日志配置
- 统一的日志配置
- 支持不同日志级别

#### `helpers.py` - 辅助函数
- 通用工具函数
- 数据转换和验证

## 🚀 启动流程

1. **加载配置** (`config.py`)
   - 读取 `config.toml`
   - 验证配置项

2. **初始化服务** (`arbitrage.py`)
   - 创建 API 客户端
   - 登录 Kalshi

3. **获取市场数据** (`clients/`)
   - 并行获取 Kalshi 和 Polymarket 数据
   - 解析事件和市场信息

4. **匹配市场** (`matcher.py`)
   - 事件匹配
   - 市场匹配（2:1）

5. **计算套利** (`calculator.py`)
   - 计算初始套利机会
   - 按利润率排序

6. **启动 WebSocket** (`websocket_manager.py`)
   - 连接双平台 WebSocket
   - 订阅价格更新
   - 实时计算套利

7. **启动 API 服务** (`main.py`)
   - 启动 FastAPI 服务器
   - 提供 RESTful API
   - 处理 WebSocket 连接

## 📊 数据流

```
Kalshi API ──┐
             ├──> ArbitrageService ──> Matcher ──> Calculator ──> WebSocket Manager
Polymarket ──┘                                                           │
                                                                         ├──> Frontend
                                                                         └──> API Routes
```

## 🔄 扩展指南

### 添加新的交易平台

1. 在 `app/clients/` 创建新客户端
   ```python
   from app.clients.base import BaseAPIClient
   
   class NewPlatformClient(BaseAPIClient):
       async def get_nba_events_and_markets(self):
           # 实现数据获取逻辑
           pass
   ```

2. 在 `app/core/models.py` 添加数据模型
   ```python
   class NewPlatformMarket(BaseModel):
       # 定义市场模型
       pass
   ```

3. 在 `app/core/matcher.py` 添加匹配逻辑
   ```python
   def match_with_new_platform(self, ...):
       # 实现匹配逻辑
       pass
   ```

4. 在 `app/services/arbitrage.py` 集成新平台
   ```python
   self.new_platform_client = NewPlatformClient(config.new_platform)
   ```

### 添加新的 API 端点

1. 在 `app/api/routes.py` 添加路由
   ```python
   @router.get("/api/new-endpoint")
   async def new_endpoint():
       # 实现逻辑
       return {"data": "..."}
   ```

### 添加新的业务逻辑

1. 在 `app/services/` 创建新服务
   ```python
   class NewService:
       def __init__(self, config):
           # 初始化
           pass
       
       async def do_something(self):
           # 业务逻辑
           pass
   ```

2. 在 `app/main.py` 集成服务
   ```python
   new_service = NewService(config)
   await new_service.do_something()
   ```

## 🧪 测试

运行测试：
```bash
# 安装测试依赖
pip install pytest pytest-asyncio

# 运行所有测试
pytest tests/

# 运行特定测试
pytest tests/test_matcher.py
```

## 📝 代码规范

- 使用 Python 3.8+
- 遵循 PEP 8 代码风格
- 使用类型提示
- 编写文档字符串
- 单元测试覆盖核心逻辑

## 🔍 调试

1. 修改日志级别（在 `app/main.py`）：
   ```python
   logger = setup_logger("arbitrage_scanner", logging.DEBUG)
   ```

2. 使用调试脚本（在 `scripts/`）：
   ```bash
   python scripts/debug_kalshi.py
   python scripts/debug_matching.py
   ```

## 📦 依赖管理

核心依赖：
- `fastapi` - Web 框架
- `uvicorn` - ASGI 服务器
- `aiohttp` - 异步 HTTP 客户端
- `websockets` - WebSocket 支持
- `pydantic` - 数据验证
- `toml` - 配置文件解析

## 🎯 优势

相比旧版本：
1. ✅ **模块化** - 清晰的职责分离
2. ✅ **可扩展** - 易于添加新功能
3. ✅ **可测试** - 独立的模块便于测试
4. ✅ **可维护** - 代码组织清晰
5. ✅ **可复用** - 模块可独立使用

