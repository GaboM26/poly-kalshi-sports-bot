"""主应用入口 - 模块化版本"""
import asyncio
import logging
from contextlib import asynccontextmanager
from typing import List
from datetime import datetime

from fastapi import FastAPI, WebSocket
from fastapi.middleware.cors import CORSMiddleware

from app.core.config import Config
from app.core.models import ArbitrageOpportunity
from app.services.arbitrage import ArbitrageService
from app.services.websocket_manager import WebSocketManager
from app.services.storage import ArbitrageStorage
from app.api import routes
from app.api.websocket import handle_websocket, broadcast_message, broadcast_log
from app.utils.logger import setup_logger

# 配置日志
logger = setup_logger("arbitrage_scanner", logging.INFO)

# 全局变量
config: Config = None
arbitrage_service: ArbitrageService = None
ws_manager: WebSocketManager = None
storage: ArbitrageStorage = None
latest_opportunities: List[ArbitrageOpportunity] = []

# 后台任务
ws_task = None
broadcast_task = None
scan_task = None
storage_task = None

# 广播间隔（秒）
BROADCAST_INTERVAL = 1.0

# 市场扫描间隔（秒）- 5分钟
SCAN_INTERVAL = 300

# 计数器
_opportunity_broadcast_count = 0


async def initialize_system():
    """初始化系统"""
    global config, arbitrage_service, ws_manager, storage, latest_opportunities
    
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
    
    # 6. 初始化 SQLite 存储服务（异步队列，不阻塞业务）
    storage = ArbitrageStorage(db_path="arbitrage_history.db")
    logger.info("✅ SQLite 存储服务初始化完成")
    
    # 7. 初始化 WebSocket 管理器（使用存储服务）
    ws_manager = WebSocketManager(
        kalshi_client=arbitrage_service.kalshi_client,
        polymarket_client=arbitrage_service.polymarket_client,
        calculator=arbitrage_service.calculator,
        on_opportunity=on_arbitrage_opportunity,
        on_log=on_ws_log,
        storage=storage
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
    """处理新的套利机会 - 只更新缓存，不立即广播"""
    global latest_opportunities, _opportunity_broadcast_count
    
    _opportunity_broadcast_count += 1
    
    # 记录前几次调用和每 100 次调用（减少日志频率）
    if _opportunity_broadcast_count <= 3 or _opportunity_broadcast_count % 100 == 0:
        logger.info(f"🔔 套利机会 #{_opportunity_broadcast_count}: {opportunity.event_name} {opportunity.team_name}")
    
    # 更新或添加机会到缓存
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
    # 不再立即广播，由定时任务统一推送


def on_ws_log(message: str):
    """处理 WebSocket 日志"""
    asyncio.create_task(broadcast_log(message))


async def broadcast_all_opportunities():
    """定期广播完整的套利机会列表和所有匹配市场"""
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
            
            # 导入转换函数
            from app.api.websocket import convert_matched_market_to_frontend
            
            # 构建套利机会的 lookup，用于关联匹配市场
            opp_lookup = {}
            for opp in latest_opportunities:
                key = f"{opp.event_name}_{opp.team_name}"
                opp_lookup[key] = opp
            
            # 构建所有匹配市场的数据（按 event_name 排序）
            matched_markets_data = []
            for mm in ws_manager.matched_markets:
                key = f"{mm.event_name}_{mm.team_name}"
                opp = opp_lookup.get(key)
                market_data = convert_matched_market_to_frontend(
                    mm,
                    ws_manager.kalshi_prices,
                    ws_manager.poly_token_prices,
                    opp
                )
                matched_markets_data.append(market_data)
            
            # 按 event_name 排序（稳定排序，不随利润变动）
            matched_markets_data.sort(key=lambda x: (x["event_name"], x["team_name"]))
            
            timestamp = None
            if arbitrage_service and arbitrage_service.stats.last_update:
                timestamp = arbitrage_service.stats.last_update.isoformat()
            
            # 只广播匹配市场数据（包含套利信息），减少消息量
            markets_message = {
                "type": "matched_markets_list",
                "data": matched_markets_data,
                "count": len(matched_markets_data),
                "opportunities_count": len(latest_opportunities),
                "timestamp": timestamp
            }
            await broadcast_message(markets_message)
            
        except asyncio.CancelledError:
            logger.info("🛑 广播任务被取消")
            break
        except Exception as e:
            logger.error(f"❌ 广播任务异常: {e}")


async def periodic_market_scan():
    """定期扫描新市场并热订阅
    
    每 SCAN_INTERVAL 秒扫描一次两个平台的事件，
    如果发现新的匹配市场，通过热订阅添加到现有 WebSocket 连接。
    """
    global arbitrage_service, ws_manager
    
    scan_count = 0  # 扫描次数计数器
    
    # 等待初始 WebSocket 连接就绪
    log_msg = f"⏳ 市场扫描任务启动，等待 WebSocket 就绪..."
    logger.info(log_msg)
    await broadcast_log(log_msg)
    
    while ws_manager and not ws_manager.is_ready():
        await asyncio.sleep(1.0)
    
    log_msg = f"🔍 市场扫描任务就绪，间隔 {SCAN_INTERVAL} 秒 ({SCAN_INTERVAL // 60} 分钟)"
    logger.info(log_msg)
    await broadcast_log(log_msg)
    
    while True:
        try:
            # 等待扫描间隔
            await asyncio.sleep(SCAN_INTERVAL)
            
            scan_count += 1
            log_msg = f"🔄 开始第 {scan_count} 次定期市场扫描..."
            logger.info(log_msg)
            await broadcast_log(log_msg)
            
            # 扫描新市场
            new_markets, new_tickers, new_tokens, new_lookup = \
                await arbitrage_service.scan_for_new_markets()
            
            if new_markets:
                log_msg = f"🆕 发现 {len(new_markets)} 个新匹配市场，开始热订阅..."
                logger.info(log_msg)
                await broadcast_log(log_msg)
                
                # 列出新市场详情
                for mm in new_markets[:5]:  # 最多显示前5个
                    detail_msg = f"   - {mm.event_name} ({mm.team_name})"
                    logger.info(detail_msg)
                    await broadcast_log(detail_msg)
                
                if len(new_markets) > 5:
                    more_msg = f"   ... 还有 {len(new_markets) - 5} 个新市场"
                    logger.info(more_msg)
                    await broadcast_log(more_msg)
                
                # 热订阅新市场
                success = await ws_manager.add_subscriptions(
                    new_markets,
                    new_tickers,
                    new_tokens,
                    new_lookup
                )
                
                if success:
                    log_msg = f"✅ 热订阅成功，当前共 {len(ws_manager.matched_markets)} 个配对市场"
                    logger.info(log_msg)
                    await broadcast_log(log_msg)
                    
                    # 推送更新后的统计信息
                    stats = arbitrage_service.get_stats()
                    await broadcast_message({
                        "type": "scan_stats",
                        "scan_count": scan_count,
                        "new_markets_found": len(new_markets),
                        "total_matched_markets": len(ws_manager.matched_markets),
                        "kalshi_markets": stats.total_kalshi_markets,
                        "polymarket_markets": stats.total_polymarket_markets,
                        "timestamp": datetime.now().isoformat()
                    })
                else:
                    log_msg = "⚠️ 热订阅部分失败，请检查日志"
                    logger.warning(log_msg)
                    await broadcast_log(log_msg)
            else:
                log_msg = f"✅ 第 {scan_count} 次扫描完成，没有发现新市场"
                logger.info(log_msg)
                await broadcast_log(log_msg)
                
                # 推送扫描统计信息
                stats = arbitrage_service.get_stats()
                await broadcast_message({
                    "type": "scan_stats",
                    "scan_count": scan_count,
                    "new_markets_found": 0,
                    "total_matched_markets": len(ws_manager.matched_markets),
                    "kalshi_markets": stats.total_kalshi_markets,
                    "polymarket_markets": stats.total_polymarket_markets,
                    "timestamp": datetime.now().isoformat()
                })
                
        except asyncio.CancelledError:
            log_msg = "🛑 市场扫描任务被取消"
            logger.info(log_msg)
            await broadcast_log(log_msg)
            break
        except Exception as e:
            log_msg = f"❌ 市场扫描任务异常: {e}"
            logger.error(log_msg)
            await broadcast_log(log_msg)
            import traceback
            traceback.print_exc()


@asynccontextmanager
async def lifespan(app: FastAPI):
    """应用生命周期管理"""
    global ws_task, broadcast_task, scan_task, storage_task
    
    # 启动时初始化
    result = await initialize_system()
    if result is None:
        logger.error("❌ 系统初始化失败")
        yield
        return
    
    kalshi_tickers, polymarket_token_ids = result
    
    # 启动存储 Worker（异步写入数据库）
    await storage.start()
    logger.info("📦 存储 Worker 已启动")
    
    # 启动 WebSocket 监听
    ws_task = asyncio.create_task(
        ws_manager.start(kalshi_tickers, polymarket_token_ids)
    )
    
    # 启动定期广播任务
    broadcast_task = asyncio.create_task(
        broadcast_all_opportunities()
    )
    log_msg = f"📡 启动定期广播任务，间隔 {BROADCAST_INTERVAL} 秒"
    logger.info(log_msg)
    
    # 启动定期市场扫描任务
    scan_task = asyncio.create_task(
        periodic_market_scan()
    )
    log_msg = f"🔍 启动定期市场扫描任务，间隔 {SCAN_INTERVAL} 秒 ({SCAN_INTERVAL // 60} 分钟)"
    logger.info(log_msg)
    
    yield
    
    # 关闭时清理
    logger.info("🛑 正在关闭服务器...")
    
    # 取消扫描任务
    if scan_task and not scan_task.done():
        scan_task.cancel()
        try:
            await asyncio.wait_for(scan_task, timeout=2.0)
        except (asyncio.CancelledError, asyncio.TimeoutError):
            pass
    
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
    
    # 停止存储服务（flush 剩余数据）
    if storage:
        await storage.stop()
        logger.info("📦 存储服务已停止")
    
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
