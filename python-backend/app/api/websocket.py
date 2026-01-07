"""WebSocket 处理"""
import json
import logging
from fastapi import WebSocket, WebSocketDisconnect
from typing import Set
from datetime import datetime

from app.core.models import ArbitrageOpportunity

logger = logging.getLogger(__name__)

# 活跃的 WebSocket 连接
active_connections: Set[WebSocket] = set()


async def handle_websocket(websocket: WebSocket, stats, latest_opportunities):
    """处理 WebSocket 连接
    
    Args:
        websocket: WebSocket 连接
        stats: 统计信息对象
        latest_opportunities: 最新套利机会列表
    """
    await websocket.accept()
    active_connections.add(websocket)
    logger.info(f"✅ WebSocket 连接建立，当前连接数: {len(active_connections)}")
    
    try:
        # 发送初始数据
        await websocket.send_text(json.dumps({
            "type": "connected",
            "message": "已连接到服务器"
        }, default=str))
        
        # 发送当前统计
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


async def broadcast_message(message: dict):
    """广播消息到所有连接
    
    Args:
        message: 要广播的消息字典
    """
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


async def broadcast_opportunity(opportunity: ArbitrageOpportunity):
    """广播套利机会
    
    Args:
        opportunity: 套利机会对象
    """
    if not active_connections:
        return
    
    message = {
        "type": "opportunity",
        "data": convert_opportunity_to_frontend(opportunity)
    }
    
    await broadcast_message(message)


async def broadcast_log(log_message: str):
    """广播日志消息
    
    Args:
        log_message: 日志消息
    """
    if not active_connections:
        return
    
    message = {
        "type": "log",
        "message": log_message,
        "timestamp": datetime.now().isoformat()
    }
    
    await broadcast_message(message)


def convert_opportunity_to_frontend(opp: ArbitrageOpportunity) -> dict:
    """转换套利机会为前端格式
    
    Args:
        opp: 套利机会对象
    
    Returns:
        前端格式的字典
    """
    # 使用存储的完整价格（不再用 1 - price 计算）
    kalshi_yes = opp.kalshi_yes_price
    kalshi_no = opp.kalshi_no_price
    poly_yes = opp.polymarket_yes_price
    poly_no = opp.polymarket_no_price
    
    return {
        "kalshi_market": {
            "platform": "Kalshi",
            "event_id": opp.kalshi_market_id,
            "event_name": opp.event_name,
            "yes_price": kalshi_yes,
            "no_price": kalshi_no,
            "volume": 0,
            "team_name": opp.team_name,
            "category": "NBA",
            "bet_side": opp.kalshi_side,
            "bet_amount": opp.kalshi_bet,
            "end_time": opp.start_time.isoformat() if opp.start_time else None
        },
        "polymarket_market": {
            "platform": "Polymarket",
            "event_id": opp.polymarket_market_id,
            "event_name": opp.event_name,
            "yes_price": poly_yes,
            "no_price": poly_no,
            "volume": 0,
            "team_name": opp.team_name,
            "category": "NBA",
            "bet_side": opp.polymarket_side,
            "bet_amount": opp.polymarket_bet,
            "end_time": opp.start_time.isoformat() if opp.start_time else None
        },
        "arbitrage_type": f"Kalshi{opp.kalshi_side.capitalize()}Polymarket{opp.polymarket_side.capitalize()}",
        "profit_margin": opp.profit_margin,
        "expected_profit": opp.expected_profit,
        "optimal_bet": [opp.kalshi_bet, opp.polymarket_bet],
        "match_confidence": 0.95,
        "timestamp": opp.timestamp.isoformat()
    }


def convert_matched_market_to_frontend(mm, kalshi_prices: dict, poly_token_prices: dict, opportunity=None) -> dict:
    """转换匹配市场为前端格式（包含实时价格）
    
    Args:
        mm: MatchedMarket 对象
        kalshi_prices: Kalshi 价格缓存 {market_id: (yes_bid, yes_ask, no_bid, no_ask)}
        poly_token_prices: Polymarket token 价格缓存 {token_id: price}
        opportunity: 可选的套利机会对象
    
    Returns:
        前端格式的字典
    """
    k_id = mm.kalshi_market.market_id
    k_prices = kalshi_prices.get(k_id)
    
    # 获取 Kalshi 实时价格
    if k_prices and len(k_prices) == 4:
        k_yes = k_prices[1]  # yes_ask
        k_no = k_prices[3]   # no_ask
        kalshi_ready = True
    else:
        k_yes = mm.kalshi_market.yes_price
        k_no = mm.kalshi_market.no_price
        kalshi_ready = False
    
    # 获取 Polymarket 实时价格
    p_yes = mm.poly_yes_price
    p_no = mm.poly_no_price
    
    # 检查 Poly 是否 ready
    poly_market = mm.polymarket_market
    own_token = poly_market.get_token_for_team(mm.team_name)
    if mm.team_name.upper() == poly_market.team_a.upper():
        opponent_token = poly_market.token_id_b
    else:
        opponent_token = poly_market.token_id_a
    
    poly_ready = (own_token in poly_token_prices) and (opponent_token in poly_token_prices)
    
    result = {
        "event_name": mm.event_name,
        "team_name": mm.team_name,
        "kalshi_market_id": mm.kalshi_market.market_id,
        "polymarket_market_id": mm.polymarket_market.market_id,
        "kalshi_yes_price": k_yes,
        "kalshi_no_price": k_no,
        "poly_yes_price": p_yes,
        "poly_no_price": p_no,
        "kalshi_ready": kalshi_ready,
        "poly_ready": poly_ready,
        "both_ready": kalshi_ready and poly_ready,
        "confidence": mm.confidence,
        "end_time": mm.kalshi_market.start_time.isoformat() if mm.kalshi_market.start_time else None,
        # 套利相关
        "has_opportunity": opportunity is not None,
        "profit_margin": opportunity.profit_margin if opportunity else 0,
        "expected_profit": opportunity.expected_profit if opportunity else 0,
        "gross_profit": opportunity.gross_profit if opportunity else 0,
        "kalshi_contracts": opportunity.kalshi_contracts if opportunity else 0,
        "kalshi_fee": opportunity.kalshi_fee if opportunity else 0,
        "arbitrage_type": f"Kalshi{opportunity.kalshi_side.capitalize()}Polymarket{opportunity.polymarket_side.capitalize()}" if opportunity else None
    }
    
    return result
