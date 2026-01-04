"""主服务器入口 - 优化版（不拆分 Polymarket 市场）

启动流程:
1. 加载配置，初始化客户端
2. 一次性获取两个平台的事件和市场
3. 执行两阶段匹配（事件匹配 + 市场匹配 2:1）
4. 启动 WebSocket 监听配对市场的价格
5. 实时计算套利机会并广播到前端
"""
import asyncio
import logging
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from contextlib import asynccontextmanager
from typing import List, Set, Dict
import json
from datetime import datetime

from config import Config
from kalshi_client import KalshiClient
from polymarket_client import PolymarketClient
from matcher import EventMatcher
from calculator import ArbitrageCalculator
from websocket_manager import WebSocketManager
from models import (
    SystemStats, ArbitrageOpportunity, 
    KalshiEvent, KalshiMarket,
    PolymarketEvent, PolymarketMarket,
    MatchedEvent, MatchedMarket
)

# 配置日志
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)

# 全局变量
config: Config = None
kalshi_client: KalshiClient = None
polymarket_client: PolymarketClient = None
matcher: EventMatcher = None
calculator: ArbitrageCalculator = None
ws_manager: WebSocketManager = None

# 数据存储
kalshi_events: List[KalshiEvent] = []
kalshi_markets: List[KalshiMarket] = []
polymarket_events: List[PolymarketEvent] = []
polymarket_markets: List[PolymarketMarket] = []
matched_events: List[MatchedEvent] = []
matched_markets: List[MatchedMarket] = []

# 前端 WebSocket 连接
active_connections: Set[WebSocket] = set()

# 统计信息
stats = SystemStats()

# 最新套利机会
latest_opportunities: List[ArbitrageOpportunity] = []

# WebSocket 后台任务
ws_task = None
broadcast_task = None

# 广播间隔（秒）
BROADCAST_INTERVAL = 1.0


async def initialize_system():
    """初始化系统 - 只执行一次"""
    global config, kalshi_client, polymarket_client, matcher, calculator, ws_manager
    global kalshi_events, kalshi_markets, polymarket_events, polymarket_markets
    global matched_events, matched_markets, stats
    
    logger.info("=" * 60)
    logger.info("🚀 启动预测市场套利扫描器 (优化版 - 不拆分)")
    logger.info("=" * 60)
    
    # 1. 加载配置
    config = Config.from_file("../config.toml")
    config.validate_config()
    logger.info("✅ 配置加载成功")
    
    # 2. 初始化客户端
    kalshi_client = KalshiClient(config.kalshi)
    polymarket_client = PolymarketClient(config.polymarket)
    
    # 3. 登录 Kalshi
    logger.info("🔐 正在连接 Kalshi...")
    success = await kalshi_client.login()
    if not success:
        logger.error("❌ Kalshi 连接失败，继续运行...")
    
    # 4. 初始化匹配器和计算器
    matcher = EventMatcher()
    calculator = ArbitrageCalculator(
        min_profit_margin=config.settings.min_profit_margin,
        default_bet_amount=config.settings.default_bet_amount
    )
    
    # 5. 一次性获取两个平台的数据
    logger.info("=" * 60)
    logger.info("📥 开始获取市场数据（只执行一次）")
    logger.info("=" * 60)
    
    # 获取 Kalshi 数据
    kalshi_events, kalshi_markets = await kalshi_client.get_nba_events_and_markets()
    
    # 获取 Polymarket 数据（不拆分）
    polymarket_events, polymarket_markets = await polymarket_client.get_nba_events_and_markets()
    
    # 更新统计
    stats.total_kalshi_events = len(kalshi_events)
    stats.total_kalshi_markets = len(kalshi_markets)
    stats.total_polymarket_events = len(polymarket_events)
    stats.total_polymarket_markets = len(polymarket_markets)  # 现在等于事件数
    
    # 6. 执行两阶段匹配 (2:1)
    matched_events, matched_markets = matcher.match_events_and_markets(
        kalshi_events, kalshi_markets,
        polymarket_events, polymarket_markets
    )
    
    stats.matched_events = len(matched_events)
    stats.matched_markets = len(matched_markets)  # Kalshi 市场数
    
    # 7. 获取 WebSocket 订阅信息
    kalshi_tickers, polymarket_token_ids, market_lookup = matcher.get_subscription_info(matched_markets)
    
    # 8. 初始化 WebSocket 管理器
    ws_manager = WebSocketManager(
        kalshi_client=kalshi_client,
        polymarket_client=polymarket_client,
        calculator=calculator,
        on_opportunity=on_arbitrage_opportunity,
        on_log=on_ws_log
    )
    ws_manager.set_matched_markets(matched_markets, market_lookup)
    
    # 9. 计算初始套利机会
    global latest_opportunities
    latest_opportunities = ws_manager.calculate_all()
    stats.arbitrage_opportunities = len(latest_opportunities)
    stats.last_update = datetime.now()
    
    logger.info("=" * 60)
    logger.info("📊 初始化完成")
    logger.info(f"   Kalshi: {stats.total_kalshi_events} 事件, {stats.total_kalshi_markets} 市场")
    logger.info(f"   Polymarket: {stats.total_polymarket_events} 事件, {stats.total_polymarket_markets} 市场 (不拆分)")
    logger.info(f"   匹配: {stats.matched_events} 事件, {stats.matched_markets} 市场对 (2:1)")
    logger.info(f"   订阅: Kalshi {len(kalshi_tickers)} 个, Poly {len(polymarket_token_ids)} 个 token")
    logger.info(f"   套利机会: {stats.arbitrage_opportunities} 个")
    logger.info("=" * 60)
    
    return kalshi_tickers, polymarket_token_ids


_opportunity_broadcast_count = 0

def on_arbitrage_opportunity(opportunity: ArbitrageOpportunity):
    """处理新的套利机会"""
    global latest_opportunities, stats, _opportunity_broadcast_count
    
    _opportunity_broadcast_count += 1
    
    # 记录前几次调用和每 50 次调用
    if _opportunity_broadcast_count <= 5 or _opportunity_broadcast_count % 50 == 0:
        logger.info(f"🔔 on_arbitrage_opportunity 被调用 #{_opportunity_broadcast_count}: {opportunity.event_name} {opportunity.team_name}")
    
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
    stats.arbitrage_opportunities = len(latest_opportunities)
    stats.last_update = datetime.now()
    
    # 广播到前端（仅当有连接时）
    if active_connections:
        if _opportunity_broadcast_count % 100 == 0:
            logger.info(f"📡 广播套利更新 #{_opportunity_broadcast_count}: {opportunity.event_name} {opportunity.team_name} 利润={opportunity.profit_margin:.2f}%")
        try:
            asyncio.create_task(broadcast_opportunity(opportunity))
        except Exception as e:
            logger.error(f"❌ 广播失败: {e}")


def on_ws_log(message: str):
    """处理 WebSocket 日志"""
    asyncio.create_task(broadcast_log(message))


async def broadcast_opportunity(opportunity: ArbitrageOpportunity):
    """广播套利机会到前端"""
    if not active_connections:
        return
    
    message = {
        "type": "opportunity",
        "data": convert_opportunity_to_frontend(opportunity)
    }
    
    await broadcast_message(message)


async def broadcast_log(log_message: str):
    """广播日志到前端"""
    if not active_connections:
        return
    
    message = {
        "type": "log",
        "message": log_message,
        "timestamp": datetime.now().isoformat()
    }
    
    await broadcast_message(message)


async def broadcast_message(message: dict):
    """广播消息到所有前端连接"""
    if not active_connections:
        return
    
    message_str = json.dumps(message, default=str)
    disconnected = set()
    
    for connection in active_connections:
        try:
            await connection.send_text(message_str)
        except:
            disconnected.add(connection)
    
    for conn in disconnected:
        active_connections.discard(conn)


async def broadcast_all_opportunities():
    """广播完整的套利机会列表（定期执行）"""
    global latest_opportunities, ws_manager
    
    while True:
        try:
            await asyncio.sleep(BROADCAST_INTERVAL)
            
            if not active_connections:
                continue
            
            # 重新计算所有套利机会并排序
            if ws_manager:
                latest_opportunities = ws_manager.calculate_all()
            
            if not latest_opportunities:
                continue
            
            # 广播完整列表
            message = {
                "type": "opportunities_list",
                "data": [convert_opportunity_to_frontend(opp) for opp in latest_opportunities[:50]],
                "count": len(latest_opportunities),
                "timestamp": datetime.now().isoformat()
            }
            
            await broadcast_message(message)
            
        except asyncio.CancelledError:
            logger.info("🛑 广播任务被取消")
            break
        except Exception as e:
            logger.error(f"❌ 广播任务异常: {e}")


def convert_opportunity_to_frontend(opp: ArbitrageOpportunity) -> dict:
    """转换套利机会为前端格式"""
    # 计算 Yes/No 价格
    if opp.kalshi_side == "yes":
        kalshi_yes = opp.kalshi_price
        kalshi_no = 1.0 - opp.kalshi_price
    else:
        kalshi_yes = 1.0 - opp.kalshi_price
        kalshi_no = opp.kalshi_price
    
    if opp.polymarket_side == "yes":
        poly_yes = opp.polymarket_price
        poly_no = 1.0 - opp.polymarket_price
    else:
        poly_yes = 1.0 - opp.polymarket_price
        poly_no = opp.polymarket_price
    
    return {
        "kalshi_market": {
            "platform": "Kalshi",
            "event_id": opp.kalshi_market_id,
            "event_name": opp.event_name,
            "yes_price": kalshi_yes,
            "no_price": kalshi_no,
            "volume": 0,  # 前端需要这个字段
            "team_name": opp.team_name,
            "category": "NBA",
            "bet_side": opp.kalshi_side,
            "bet_amount": opp.kalshi_bet,
            "end_time": opp.start_time.isoformat() if opp.start_time else None  # 添加比赛时间
        },
        "polymarket_market": {
            "platform": "Polymarket",
            "event_id": opp.polymarket_market_id,
            "event_name": opp.event_name,
            "yes_price": poly_yes,
            "no_price": poly_no,
            "volume": 0,  # 前端需要这个字段
            "team_name": opp.team_name,
            "category": "NBA",
            "bet_side": opp.polymarket_side,
            "bet_amount": opp.polymarket_bet,
            "end_time": opp.start_time.isoformat() if opp.start_time else None  # 添加比赛时间
        },
        "arbitrage_type": f"Kalshi{opp.kalshi_side.capitalize()}Polymarket{opp.polymarket_side.capitalize()}",
        "profit_margin": opp.profit_margin,
        "expected_profit": opp.expected_profit,
        "optimal_bet": [opp.kalshi_bet, opp.polymarket_bet],  # 前端期望的是数组
        "match_confidence": 0.95,
        "timestamp": opp.timestamp.isoformat()
    }


@asynccontextmanager
async def lifespan(app: FastAPI):
    """应用生命周期管理"""
    global ws_task, broadcast_task
    ws_task = None
    broadcast_task = None
    
    # 启动时初始化
    kalshi_tickers, polymarket_token_ids = await initialize_system()
    
    # 启动 WebSocket 监听（后台任务）
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
        except asyncio.CancelledError:
            logger.info("✅ WebSocket 任务已取消")
        except asyncio.TimeoutError:
            logger.warning("⚠️ WebSocket 任务取消超时")
        except Exception as e:
            logger.error(f"❌ WebSocket 任务取消异常: {e}")
    
    # 关闭 HTTP 会话
    if kalshi_client and kalshi_client.session:
        await kalshi_client.session.close()
    if polymarket_client and polymarket_client.session:
        await polymarket_client.session.close()
    
    logger.info("👋 服务器已关闭")


# 创建 FastAPI 应用
app = FastAPI(title="预测市场套利扫描器 (不拆分版)", lifespan=lifespan)

# 配置 CORS
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)


@app.get("/")
async def root():
    """根路径"""
    return {
        "message": "预测市场套利扫描器 API (不拆分版)",
        "version": "2.1.0",
        "status": "running"
    }


@app.get("/api/stats")
async def get_stats():
    """获取统计信息"""
    ws_stats = ws_manager.get_stats() if ws_manager else {}
    return {
        **stats.model_dump(mode='json'),
        "ws_stats": ws_stats
    }


@app.get("/api/opportunities")
async def get_opportunities():
    """获取套利机会"""
    return [convert_opportunity_to_frontend(opp) for opp in latest_opportunities[:20]]


@app.get("/api/matched-markets")
async def get_matched_markets():
    """获取配对市场"""
    return [
        {
            "event_name": mm.event_name,
            "team_name": mm.team_name,
            "kalshi_market_id": mm.kalshi_market.market_id,
            "kalshi_yes_price": mm.kalshi_market.yes_price,
            "kalshi_no_price": mm.kalshi_market.no_price,
            "polymarket_market_id": mm.polymarket_market.market_id,
            "poly_yes_price": mm.poly_yes_price,
            "poly_no_price": mm.poly_no_price,
            "confidence": mm.confidence
        }
        for mm in matched_markets
    ]


@app.get("/api/tracking")
async def get_tracking():
    """获取套利追踪信息"""
    if ws_manager:
        return ws_manager.get_tracking_stats()
    return {"active_count": 0, "completed_count": 0, "active": [], "recent_completed": []}


@app.websocket("/ws")
async def websocket_endpoint(websocket: WebSocket):
    """WebSocket 连接"""
    await websocket.accept()
    active_connections.add(websocket)
    logger.info(f"✅ WebSocket 连接建立，当前连接数: {len(active_connections)}")
    
    try:
        # 发送初始数据
        await websocket.send_text(json.dumps({
            "type": "connected",
            "message": "已连接到服务器"
        }, default=str))
        
        # 发送当前统计 (使用前端期望的 scan_completed 格式)
        await websocket.send_text(json.dumps({
            "type": "scan_completed",
            "kalshi_count": stats.total_kalshi_markets,
            "polymarket_count": stats.total_polymarket_markets,
            "matched_count": stats.matched_markets,
            "opportunities_count": stats.arbitrage_opportunities
        }, default=str))
        
        # 发送当前套利机会
        for opp in latest_opportunities[:20]:
            await websocket.send_text(json.dumps({
                "type": "opportunity",
                "data": convert_opportunity_to_frontend(opp)
            }, default=str))
        
        # 保持连接
        while True:
            data = await websocket.receive_text()
            
    except WebSocketDisconnect:
        logger.info("WebSocket 连接断开")
    except Exception as e:
        logger.error(f"WebSocket 错误: {e}")
    finally:
        active_connections.discard(websocket)
        logger.info(f"WebSocket 连接关闭，当前连接数: {len(active_connections)}")


if __name__ == "__main__":
    import uvicorn
    import signal
    
    def signal_handler(sig, frame):
        logger.info("🛑 接收到中断信号，正在关闭...")
    
    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)
    
    try:
        uvicorn.run(
            "main:app",
            host="0.0.0.0",
            port=3000,
            log_level="info"
        )
    except KeyboardInterrupt:
        logger.info("👋 程序已退出")
