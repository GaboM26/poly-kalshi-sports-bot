"""API 路由定义"""
from fastapi import APIRouter, Query
from pydantic import BaseModel
from typing import TYPE_CHECKING, Optional

if TYPE_CHECKING:
    from app.services.arbitrage import ArbitrageService
    from app.services.websocket_manager import WebSocketManager


# 请求模型
class KalshiOrderRequest(BaseModel):
    """Kalshi 下单请求"""
    ticker: str
    side: str  # "yes" 或 "no"
    action: str  # "buy" 或 "sell"
    count: int = 1


class PolymarketOrderRequest(BaseModel):
    """Polymarket 下单请求"""
    token_id: str
    side: str  # "buy" 或 "sell"
    amount: float  # USDC 金额


class ArbitrageExecuteRequest(BaseModel):
    """套利执行请求"""
    # Kalshi 端
    kalshi_ticker: str
    kalshi_side: str  # "yes" 或 "no"
    kalshi_bet: float  # 下注金额（美元）
    kalshi_price: float  # 价格（用于计算合约数量）
    
    # Polymarket 端
    poly_token_id: str
    poly_side: str  # "buy" 或 "sell"
    poly_amount: float  # USDC 金额

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


# ==================== 交易相关 API ====================

@router.post("/api/order/kalshi")
async def create_kalshi_order(request: KalshiOrderRequest):
    """Kalshi 市价下单"""
    if not arbitrage_service:
        return {"success": False, "error": "服务未初始化"}
    
    try:
        result, elapsed_ms = await arbitrage_service.kalshi_client.create_market_order(
            ticker=request.ticker,
            side=request.side,
            action=request.action,
            count=request.count
        )
        
        if result:
            return {
                "success": True,
                "order": result.get("order", {}),
                "elapsed_ms": elapsed_ms
            }
        else:
            return {
                "success": False,
                "error": "下单失败",
                "elapsed_ms": elapsed_ms
            }
    except Exception as e:
        return {"success": False, "error": str(e)}


@router.get("/api/orders/kalshi")
async def get_kalshi_orders(status: Optional[str] = Query(None, description="订单状态过滤")):
    """获取 Kalshi 订单列表"""
    if not arbitrage_service:
        return {"orders": [], "error": "服务未初始化"}
    
    try:
        orders = await arbitrage_service.kalshi_client.get_orders(status=status)
        if orders is not None:
            return {"orders": orders}
        else:
            return {"orders": [], "error": "获取订单失败"}
    except Exception as e:
        return {"orders": [], "error": str(e)}


@router.get("/api/positions/kalshi")
async def get_kalshi_positions():
    """获取 Kalshi 持仓列表"""
    if not arbitrage_service:
        return {"positions": [], "error": "服务未初始化"}
    
    try:
        positions = await arbitrage_service.kalshi_client.get_positions()
        if positions is not None:
            return {"positions": positions}
        else:
            return {"positions": [], "error": "获取持仓失败"}
    except Exception as e:
        return {"positions": [], "error": str(e)}


@router.delete("/api/orders/kalshi/{order_id}")
async def cancel_kalshi_order(order_id: str):
    """取消 Kalshi 订单"""
    if not arbitrage_service:
        return {"success": False, "error": "服务未初始化"}
    
    try:
        success = await arbitrage_service.kalshi_client.cancel_order(order_id)
        return {"success": success}
    except Exception as e:
        return {"success": False, "error": str(e)}


# ==================== Polymarket 交易 API ====================

@router.post("/api/order/polymarket")
async def create_polymarket_order(request: PolymarketOrderRequest):
    """Polymarket 市价下单"""
    if not arbitrage_service:
        return {"success": False, "error": "服务未初始化"}
    
    try:
        result, elapsed_ms = await arbitrage_service.polymarket_client.create_market_order(
            token_id=request.token_id,
            side=request.side,
            amount=request.amount
        )
        
        if result and result.get("success"):
            return {
                "success": True,
                "order_id": result.get("orderID"),
                "status": result.get("status"),
                "taking_amount": result.get("takingAmount"),
                "making_amount": result.get("makingAmount"),
                "elapsed_ms": elapsed_ms
            }
        else:
            return {
                "success": False,
                "error": result.get("errorMsg", "下单失败") if result else "下单失败",
                "elapsed_ms": elapsed_ms
            }
    except Exception as e:
        return {"success": False, "error": str(e)}


@router.get("/api/orders/polymarket")
async def get_polymarket_orders():
    """获取 Polymarket 订单列表"""
    if not arbitrage_service:
        return {"orders": [], "error": "服务未初始化"}
    
    try:
        orders = await arbitrage_service.polymarket_client.get_open_orders()
        if orders is not None:
            return {"orders": orders}
        else:
            return {"orders": [], "error": "获取订单失败"}
    except Exception as e:
        return {"orders": [], "error": str(e)}


@router.delete("/api/orders/polymarket/{order_id}")
async def cancel_polymarket_order(order_id: str):
    """取消 Polymarket 订单"""
    if not arbitrage_service:
        return {"success": False, "error": "服务未初始化"}
    
    try:
        success = await arbitrage_service.polymarket_client.cancel_order(order_id)
        return {"success": success}
    except Exception as e:
        return {"success": False, "error": str(e)}


# ==================== 套利执行 API ====================

@router.post("/api/arbitrage/execute")
async def execute_arbitrage(request: ArbitrageExecuteRequest):
    """执行套利交易（同时在两个平台下单）
    
    注意：这会同时在 Kalshi 和 Polymarket 下单
    Kalshi 使用合约数量，Polymarket 使用 USDC 金额
    """
    if not arbitrage_service:
        return {"success": False, "error": "服务未初始化"}
    
    results = {
        "success": False,
        "kalshi": {"success": False},
        "polymarket": {"success": False}
    }
    
    try:
        import asyncio
        
        # 计算 Kalshi 合约数量（单位转换）
        # count = bet / price
        # 例如：bet = $50, price = 0.50 -> count = 100
        kalshi_count = max(1, int(round(request.kalshi_bet / request.kalshi_price)))
        
        # 同时执行两个下单请求
        kalshi_task = arbitrage_service.kalshi_client.create_market_order(
            ticker=request.kalshi_ticker,
            side=request.kalshi_side,
            action="buy",
            count=kalshi_count
        )
        
        poly_task = arbitrage_service.polymarket_client.create_market_order(
            token_id=request.poly_token_id,
            side=request.poly_side,
            amount=request.poly_amount
        )
        
        # 并行执行
        kalshi_result, poly_result = await asyncio.gather(
            kalshi_task,
            poly_task,
            return_exceptions=True
        )
        
        # 处理 Kalshi 结果
        if isinstance(kalshi_result, Exception):
            results["kalshi"] = {"success": False, "error": str(kalshi_result)}
        else:
            kalshi_data, kalshi_ms = kalshi_result
            if kalshi_data:
                results["kalshi"] = {
                    "success": True,
                    "order": kalshi_data.get("order", {}),
                    "elapsed_ms": kalshi_ms,
                    "count": kalshi_count
                }
            else:
                results["kalshi"] = {"success": False, "error": "下单失败", "elapsed_ms": kalshi_ms}
        
        # 处理 Polymarket 结果
        if isinstance(poly_result, Exception):
            results["polymarket"] = {"success": False, "error": str(poly_result)}
        else:
            poly_data, poly_ms = poly_result
            if poly_data and poly_data.get("success"):
                results["polymarket"] = {
                    "success": True,
                    "order_id": poly_data.get("orderID"),
                    "status": poly_data.get("status"),
                    "elapsed_ms": poly_ms,
                    "amount": request.poly_amount
                }
            else:
                error_msg = poly_data.get("errorMsg", "下单失败") if poly_data else "下单失败"
                results["polymarket"] = {"success": False, "error": error_msg, "elapsed_ms": poly_ms}
        
        # 判断整体是否成功（两边都成功才算成功）
        results["success"] = results["kalshi"]["success"] and results["polymarket"]["success"]
        
        return results
        
    except Exception as e:
        return {"success": False, "error": str(e), "kalshi": results["kalshi"], "polymarket": results["polymarket"]}
