"""API 路由定义"""
from fastapi import APIRouter, Query
from typing import TYPE_CHECKING, Optional

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


@router.get("/api/data-coverage")
async def get_data_coverage():
    """获取数据覆盖率信息"""
    if ws_manager:
        return ws_manager.get_data_coverage()
    return {
        "total_markets": 0,
        "kalshi_ready": 0,
        "polymarket_ready": 0,
        "both_ready": 0,
        "kalshi_coverage": "0/0",
        "polymarket_coverage": "0/0",
        "full_coverage": "0/0",
        "kalshi_connected": False,
        "polymarket_connected": False,
        "kalshi_latency_ms": None,
        "polymarket_latency_ms": None
    }


@router.get("/api/arbitrage-history")
async def get_arbitrage_history():
    """获取历史套利机会（从 SQLite 数据库）"""
    if ws_manager:
        active = ws_manager.active_tracking
        
        # 从 SQLite 获取已完成的记录（包含完整的 profit_history）
        completed = ws_manager.storage.get_all_completed_with_history(limit=100)
        
        return {
            "active": [
                {
                    "event_name": r.event_name,
                    "team_name": r.team_name,
                    "kalshi_market_id": r.kalshi_market_id,
                    "polymarket_market_id": r.polymarket_market_id,
                    "start_time": r.start_time.isoformat(),
                    "duration_seconds": None,  # 活跃记录没有结束时间
                    "max_profit_margin": r.max_profit_margin,
                    "max_profit_time": r.max_profit_time.isoformat() if r.max_profit_time else None
                }
                for r in active.values()
            ],
            "completed": completed  # 已包含 profit_history
        }
    return {"active": [], "completed": []}


@router.get("/api/account-balance")
async def get_account_balance():
    """获取两个平台的账户余额"""
    if not arbitrage_service:
        return {
            "kalshi": {"available": False, "error": "服务未初始化"},
            "polymarket": {"available": False, "error": "服务未初始化"}
        }
    
    # 获取 Kalshi 余额
    kalshi_data = {"available": False}
    try:
        kalshi_balance = await arbitrage_service.kalshi_client.get_balance()
        if kalshi_balance:
            kalshi_data = {
                "available": True,
                "balance": kalshi_balance.get('balance', 0) / 100.0,  # 美分转美元
                "portfolio_value": kalshi_balance.get('portfolio_value', 0) / 100.0,  # 美分转美元
                "updated_ts": kalshi_balance.get('updated_ts', 0)
            }
        else:
            kalshi_data = {"available": False, "error": "获取失败"}
    except Exception as e:
        kalshi_data = {"available": False, "error": str(e)}
    
    # 获取 Polymarket 余额
    poly_data = {"available": False}
    try:
        poly_balance = await arbitrage_service.polymarket_client.get_balance()
        if poly_balance:
            poly_data = {
                "available": True,
                "balance": poly_balance.get('balance', 0),  # 已经是美元
                "pnl": poly_balance.get('pnl', '0'),
                "trades": poly_balance.get('trades', 0),
                "positions": poly_balance.get('positions', 0)
            }
        else:
            poly_data = {"available": False, "error": "未配置钱包地址或获取失败"}
    except Exception as e:
        poly_data = {"available": False, "error": str(e)}
    
    return {
        "kalshi": kalshi_data,
        "polymarket": poly_data
    }


@router.get("/api/history/search")
async def search_history(
    min_profit: Optional[float] = Query(None, description="最小利润率"),
    max_profit: Optional[float] = Query(None, description="最大利润率"),
    min_duration: Optional[float] = Query(None, description="最小持续时间（秒）"),
    max_duration: Optional[float] = Query(None, description="最大持续时间（秒）"),
    event_name: Optional[str] = Query(None, description="事件名称（模糊匹配）"),
    team_name: Optional[str] = Query(None, description="队伍名称（模糊匹配）"),
    start_date: Optional[str] = Query(None, description="开始日期（ISO格式）"),
    end_date: Optional[str] = Query(None, description="结束日期（ISO格式）"),
    sort_by: str = Query("start_time", description="排序字段"),
    sort_order: str = Query("desc", description="排序方向"),
    limit: int = Query(50, ge=1, le=500, description="返回数量"),
    offset: int = Query(0, ge=0, description="偏移量"),
    include_history: bool = Query(False, description="是否包含利润历史")
):
    """搜索历史套利记录"""
    if not ws_manager:
        return {"records": [], "total": 0, "limit": limit, "offset": offset, "has_more": False}
    
    return ws_manager.storage.search_records(
        min_profit=min_profit,
        max_profit=max_profit,
        min_duration=min_duration,
        max_duration=max_duration,
        event_name=event_name,
        team_name=team_name,
        start_date=start_date,
        end_date=end_date,
        sort_by=sort_by,
        sort_order=sort_order,
        limit=limit,
        offset=offset,
        include_history=include_history
    )


@router.get("/api/history/statistics")
async def get_history_statistics():
    """获取历史记录统计信息"""
    if not ws_manager:
        return {
            "total_records": 0,
            "avg_profit": 0,
            "max_profit": 0,
            "min_profit": 0,
            "avg_duration": 0,
            "max_duration": 0,
            "min_duration": 0,
            "top_events": [],
            "top_teams": [],
            "profit_distribution": []
        }
    
    return ws_manager.storage.get_statistics()
