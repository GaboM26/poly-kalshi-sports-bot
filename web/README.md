# 预测市场套利扫描器 - React 前端

这是预测市场套利扫描器的 React 前端应用。

## 技术栈

- **React 18** - UI 框架
- **TypeScript** - 类型安全
- **Vite** - 构建工具
- **Tailwind CSS** - 样式框架
- **Lucide React** - 图标库

## 开发

```bash
# 安装依赖
npm install

# 启动开发服务器
npm run dev

# 构建生产版本
npm run build

# 预览生产构建
npm run preview
```

## 项目结构

```
src/
├── components/        # React 组件
│   ├── Header.tsx           # 顶部状态栏
│   ├── MarketStats.tsx      # 市场统计卡片
│   ├── OpportunityCard.tsx  # 套利机会卡片
│   ├── OpportunityList.tsx  # 套利机会列表
│   └── LogPanel.tsx         # 日志面板
├── hooks/            # 自定义 Hooks
│   └── useWebSocket.ts      # WebSocket 连接管理
├── types/            # TypeScript 类型定义
│   └── index.ts
├── utils/            # 工具函数
│   ├── api.ts               # API 调用
│   └── format.ts            # 格式化函数
├── App.tsx           # 主应用组件
├── main.tsx          # 应用入口
└── index.css         # 全局样式
```

## 功能特性

### 实时数据
- WebSocket 连接自动管理
- 实时套利机会推送
- 自动重连机制
- 心跳保活

### 用户界面
- 响应式设计
- 现代化 UI
- 实时状态更新
- 详细的套利信息

### 数据展示
- 市场统计面板
- 套利机会列表
- 详情查看
- 系统日志

## API 集成

前端通过以下方式与后端通信：

### REST API
- `GET /api/health` - 健康检查
- `GET /api/markets/kalshi` - Kalshi 市场
- `GET /api/markets/polymarket` - Polymarket 市场
- `GET /api/opportunities` - 套利机会
- `POST /api/scan` - 触发扫描

### WebSocket
- `WS /ws` - 实时数据推送

## 环境配置

开发环境下，Vite 会自动代理 API 请求到后端：

```typescript
// vite.config.ts
server: {
  proxy: {
    '/api': 'http://localhost:3000',
    '/ws': {
      target: 'ws://localhost:3000',
      ws: true,
    },
  },
}
```

## 构建部署

```bash
# 构建
npm run build

# 输出目录
dist/
```

构建后的文件会被 Rust 后端的静态文件服务器提供。
