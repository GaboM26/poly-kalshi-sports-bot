"""API 路由定义"""
from fastapi import APIRouter
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from app.services.arbitrage import ArbitrageService
    from app.services.websocket_manager import WebSocketManager

router = APIRouter()

# 全局变量（由 main.py 设置）
arbitrage_service: 'ArbitrageService' = None
ws_manager: 'WebSocketManager' = None
latest_opportunities = []


def set_services(arb_service: 'ArbitrageService', ws_mgr: 'WebSocketManager', opportunities: list):
    """设置服务实例（由 main.py 调用）"""
    global arbitrage_service, ws_manager, latest_opportunities
    arbitrage_service = arb_service
    ws_manager = ws_mgr
    latest_opportunities = opportunities


@router.get("/")
async def root():
    """根路径"""
    return {
        "message": "预测市场套利扫描器 API (模块化版)",
        "version": "2.0.0",
        "status": "running"
    }


@router.get("/api/stats")
async def get_stats():
    """获取统计信息"""
    if not arbitrage_service:
        return {"error": "服务未初始化"}
    
    stats = arbitrage_service.get_stats()
    ws_stats = ws_manager.get_stats() if ws_manager else {}
    
    return {
        **stats.model_dump(mode='json'),
        "ws_stats": ws_stats
    }


@router.get("/api/opportunities")
async def get_opportunities():
    """获取套利机会"""
    from app.api.websocket import convert_opportunity_to_frontend
    
    if not latest_opportunities:
        return []
    
    return [convert_opportunity_to_frontend(opp) for opp in latest_opportunities[:20]]


@router.get("/api/matched-markets")
async def get_matched_markets():
    """获取配对市场"""
    if not arbitrage_service:
        return []
    
    matched_markets = arbitrage_service.matched_markets
    
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


@router.get("/api/tracking")
async def get_tracking():
    """获取套利追踪信息"""
    if ws_manager:
        return ws_manager.get_tracking_stats()
    return {"active_count": 0, "completed_count": 0, "active": [], "recent_completed": []}
