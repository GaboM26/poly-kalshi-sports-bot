"""主应用入口 - 模块化版本"""
import asyncio
import logging
from contextlib import asynccontextmanager
from typing import List

from fastapi import FastAPI, WebSocket
from fastapi.middleware.cors import CORSMiddleware

from app.core.config import Config
from app.core.models import ArbitrageOpportunity
from app.services.arbitrage import ArbitrageService
from app.services.websocket_manager import WebSocketManager
from app.api import routes
from app.api.websocket import handle_websocket, broadcast_message, broadcast_opportunity, broadcast_log
from app.utils.logger import setup_logger

# 配置日志
logger = setup_logger("arbitrage_scanner", logging.INFO)

# 全局变量
config: Config = None
arbitrage_service: ArbitrageService = None
ws_manager: WebSocketManager = None
latest_opportunities: List[ArbitrageOpportunity] = []

# 后台任务
ws_task = None
broadcast_task = None

# 广播间隔（秒）
BROADCAST_INTERVAL = 1.0

# 计数器
_opportunity_broadcast_count = 0


async def initialize_system():
    """初始化系统"""
    global config, arbitrage_service, ws_manager, latest_opportunities
    
    logger.info("=" * 60)
    logger.info("🚀 启动预测市场套利扫描器 (模块化版)")
    logger.info("=" * 60)
    
    # 1. 加载配置
    config = Config.from_file("config.toml")
    config.validate_config()
    logger.info("✅ 配置加载成功")
    
    # 2. 初始化套利服务
    arbitrage_service = ArbitrageService(config)
    success = await arbitrage_service.initialize()
    if not success:
        logger.error("❌ 服务初始化失败")
        return None, None
    
    # 3. 获取市场数据
    await arbitrage_service.fetch_market_data()
    
    # 4. 匹配市场
    await arbitrage_service.match_markets()
    
    # 5. 获取订阅信息
    kalshi_tickers, polymarket_token_ids, market_lookup = arbitrage_service.get_subscription_info()
    
    # 6. 初始化 WebSocket 管理器
    ws_manager = WebSocketManager(
        kalshi_client=arbitrage_service.kalshi_client,
        polymarket_client=arbitrage_service.polymarket_client,
        calculator=arbitrage_service.calculator,
        on_opportunity=on_arbitrage_opportunity,
        on_log=on_ws_log
    )
    ws_manager.set_matched_markets(arbitrage_service.matched_markets, market_lookup)
    
    # 7. 不再计算初始套利机会 - 等待 WebSocket 连接成功后再计算
    # 初始套利机会列表为空
    arbitrage_service.stats.arbitrage_opportunities = 0
    
    # 8. 设置 API 路由的服务实例
    routes.set_services(arbitrage_service, ws_manager, latest_opportunities)
    
    stats = arbitrage_service.get_stats()
    logger.info("=" * 60)
    logger.info("📊 初始化完成 (等待 WebSocket 连接后计算套利)")
    logger.info(f"   Kalshi: {stats.total_kalshi_events} 事件, {stats.total_kalshi_markets} 市场")
    logger.info(f"   Polymarket: {stats.total_polymarket_events} 事件, {stats.total_polymarket_markets} 市场")
    logger.info(f"   匹配: {stats.matched_events} 事件, {stats.matched_markets} 市场对")
    logger.info(f"   订阅: Kalshi {len(kalshi_tickers)} 个, Poly {len(polymarket_token_ids)} 个 token")
    logger.info("=" * 60)
    
    return kalshi_tickers, polymarket_token_ids


def on_arbitrage_opportunity(opportunity: ArbitrageOpportunity):
    """处理新的套利机会"""
    global latest_opportunities, _opportunity_broadcast_count
    
    _opportunity_broadcast_count += 1
    
    # 记录前几次调用和每 50 次调用
    if _opportunity_broadcast_count <= 5 or _opportunity_broadcast_count % 50 == 0:
        logger.info(f"🔔 套利机会 #{_opportunity_broadcast_count}: {opportunity.event_name} {opportunity.team_name}")
    
    # 更新或添加机会
    found = False
    for i, opp in enumerate(latest_opportunities):
        if opp.event_name == opportunity.event_name and opp.team_name == opportunity.team_name:
            latest_opportunities[i] = opportunity
            found = True
            break
    
    if not found:
        latest_opportunities.append(opportunity)
    
    # 按利润率排序
    latest_opportunities.sort(key=lambda x: x.profit_margin, reverse=True)
    
    # 广播到前端
    try:
        asyncio.create_task(broadcast_opportunity(opportunity))
    except Exception as e:
        logger.error(f"❌ 广播失败: {e}")


def on_ws_log(message: str):
    """处理 WebSocket 日志"""
    asyncio.create_task(broadcast_log(message))


async def broadcast_all_opportunities():
    """定期广播完整的套利机会列表"""
    global latest_opportunities, ws_manager
    
    # 等待 WebSocket 连接成功
    logger.info("⏳ 等待 WebSocket 连接成功后开始广播...")
    while ws_manager and not ws_manager.is_ready():
        await asyncio.sleep(0.5)
    logger.info("✅ WebSocket 已就绪，开始广播套利机会")
    
    while True:
        try:
            await asyncio.sleep(BROADCAST_INTERVAL)
            
            # 只有在 WebSocket 连接就绪后才计算
            if ws_manager and ws_manager.is_ready():
                latest_opportunities = ws_manager.calculate_all()
            
            if not latest_opportunities:
                continue
            
            # 导入转换函数
            from app.api.websocket import convert_opportunity_to_frontend
            
            # 广播完整列表
            timestamp = None
            if arbitrage_service and arbitrage_service.stats.last_update:
                timestamp = arbitrage_service.stats.last_update.isoformat()
            
            message = {
                "type": "opportunities_list",
                "data": [convert_opportunity_to_frontend(opp) for opp in latest_opportunities[:50]],
                "count": len(latest_opportunities),
                "timestamp": timestamp
            }
            
            await broadcast_message(message)
            
        except asyncio.CancelledError:
            logger.info("🛑 广播任务被取消")
            break
        except Exception as e:
            logger.error(f"❌ 广播任务异常: {e}")


@asynccontextmanager
async def lifespan(app: FastAPI):
    """应用生命周期管理"""
    global ws_task, broadcast_task
    
    # 启动时初始化
    result = await initialize_system()
    if result is None:
        logger.error("❌ 系统初始化失败")
        yield
        return
    
    kalshi_tickers, polymarket_token_ids = result
    
    # 启动 WebSocket 监听
    ws_task = asyncio.create_task(
        ws_manager.start(kalshi_tickers, polymarket_token_ids)
    )
    
    # 启动定期广播任务
    broadcast_task = asyncio.create_task(
        broadcast_all_opportunities()
    )
    logger.info(f"📡 启动定期广播任务，间隔 {BROADCAST_INTERVAL} 秒")
    
    yield
    
    # 关闭时清理
    logger.info("🛑 正在关闭服务器...")
    
    # 取消广播任务
    if broadcast_task and not broadcast_task.done():
        broadcast_task.cancel()
        try:
            await asyncio.wait_for(broadcast_task, timeout=2.0)
        except (asyncio.CancelledError, asyncio.TimeoutError):
            pass
    
    # 取消 WebSocket 任务
    if ws_task and not ws_task.done():
        ws_task.cancel()
        try:
            await asyncio.wait_for(ws_task, timeout=5.0)
        except (asyncio.CancelledError, asyncio.TimeoutError):
            pass
    
    # 关闭服务
    if arbitrage_service:
        await arbitrage_service.close()
    
    logger.info("👋 服务器已关闭")


# 创建 FastAPI 应用
app = FastAPI(
    title="预测市场套利扫描器",
    description="扫描 Kalshi 和 Polymarket 之间的套利机会",
    version="2.0.0",
    lifespan=lifespan
)

# 配置 CORS
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# 注册路由
app.include_router(routes.router)


@app.websocket("/ws")
async def websocket_endpoint(websocket: WebSocket):
    """WebSocket 连接端点"""
    await handle_websocket(
        websocket,
        arbitrage_service.stats if arbitrage_service else None,
        latest_opportunities
    )


if __name__ == "__main__":
    import uvicorn
    import signal
    
    def signal_handler(sig, frame):
        logger.info("🛑 接收到中断信号，正在关闭...")
    
    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)
    
    try:
        uvicorn.run(
            "app.main:app",
            host="0.0.0.0",
            port=3000,
            log_level="info"
        )
    except KeyboardInterrupt:
        logger.info("👋 程序已退出")
