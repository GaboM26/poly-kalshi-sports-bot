#!/usr/bin/env python3
"""
Polymarket Order Service - FastAPI服务
处理Polymarket下单请求，使用官方py-clob-client SDK
"""

import os
import logging
from typing import Optional
from contextlib import asynccontextmanager

import toml
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel

from py_clob_client.client import ClobClient
from py_clob_client.clob_types import ApiCreds, OrderArgs, MarketOrderArgs, OrderType
from py_clob_client.order_builder.constants import BUY, SELL

# 配置日志
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger("poly-order-service")

# 全局CLOB客户端
clob_client: Optional[ClobClient] = None


# ==================== 请求/响应模型 ====================

class MarketOrderRequest(BaseModel):
    """市价单请求"""
    token_id: str
    side: str  # "buy" or "sell"
    amount: float  # BUY: USDC金额, SELL: token数量
    order_type: Optional[str] = "FAK"  # GTC, FOK, FAK


class LimitOrderRequest(BaseModel):
    """限价单请求"""
    token_id: str
    side: str  # "buy" or "sell"
    price: float  # 0.0 - 1.0
    size: float  # token数量
    order_type: Optional[str] = "GTC"


class CancelOrderRequest(BaseModel):
    """取消订单请求"""
    order_id: str


class OrderResponse(BaseModel):
    """订单响应"""
    success: bool
    order_id: Optional[str] = None
    status: Optional[str] = None
    error: Optional[str] = None
    data: Optional[dict] = None


# ==================== 配置加载 ====================

def load_config():
    """从配置文件加载配置"""
    config_paths = [
        os.path.join(os.path.dirname(__file__), 'config.toml'),
        os.path.join(os.path.dirname(__file__), '..', 'rust-backend', 'config.toml'),
        os.path.join(os.path.dirname(__file__), '..', 'config.toml'),
    ]
    
    for path in config_paths:
        if os.path.exists(path):
            logger.info(f"加载配置文件: {path}")
            return toml.load(path)
    
    raise FileNotFoundError("找不到配置文件 config.toml")


def init_clob_client():
    """初始化CLOB客户端"""
    global clob_client
    
    config = load_config()
    poly_config = config.get('polymarket', {})
    
    host = poly_config.get('clob_url', 'https://clob.polymarket.com')
    private_key = poly_config.get('private_key')
    wallet_address = poly_config.get('wallet_address')
    signature_type = poly_config.get('signature_type', 1)  # 1 = PolyProxy (Magic Link)
    chain_id = 137  # Polygon Mainnet
    
    if not private_key:
        raise ValueError("配置文件中缺少 polymarket.private_key")
    
    logger.info(f"初始化CLOB客户端: host={host}, chain_id={chain_id}")
    logger.info(f"Wallet地址: {wallet_address}")
    
    # 创建客户端
    clob_client = ClobClient(
        host,
        chain_id=chain_id,
        key=private_key,
        signature_type=signature_type,
        funder=wallet_address
    )
    
    # 派生API凭据
    logger.info("正在派生API凭据...")
    creds = clob_client.create_or_derive_api_creds()
    if creds:
        clob_client.set_api_creds(creds)
        logger.info(f"API凭据派生成功: {creds.api_key[:20]}...")
    else:
        raise ValueError("无法派生API凭据")
    
    return clob_client


# ==================== FastAPI应用 ====================

@asynccontextmanager
async def lifespan(app: FastAPI):
    """应用生命周期管理"""
    # 启动时初始化
    logger.info("启动Polymarket下单服务...")
    try:
        init_clob_client()
        logger.info("CLOB客户端初始化成功")
    except Exception as e:
        logger.error(f"CLOB客户端初始化失败: {e}")
        raise
    
    yield
    
    # 关闭时清理
    logger.info("关闭Polymarket下单服务...")


app = FastAPI(
    title="Polymarket Order Service",
    description="Polymarket下单服务 - 使用官方py-clob-client SDK",
    version="1.0.0",
    lifespan=lifespan
)


# ==================== API端点 ====================

@app.get("/health")
async def health_check():
    """健康检查"""
    return {
        "status": "healthy",
        "client_initialized": clob_client is not None,
        "address": clob_client.get_address() if clob_client else None
    }


@app.post("/order/market", response_model=OrderResponse)
async def place_market_order(request: MarketOrderRequest):
    """下市价单"""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB客户端未初始化")
    
    try:
        logger.info(f"市价单请求: token_id={request.token_id[:20]}..., side={request.side}, amount={request.amount}")
        
        # 确定side
        side = BUY if request.side.lower() == "buy" else SELL
        
        # 确定order_type
        order_type_map = {
            "GTC": OrderType.GTC,
            "FOK": OrderType.FOK,
            "FAK": OrderType.FAK,
            "GTD": OrderType.GTD,
        }
        order_type = order_type_map.get(request.order_type.upper(), OrderType.FAK)
        
        # 获取市场信息
        tick_size = clob_client.get_tick_size(request.token_id)
        neg_risk = clob_client.get_neg_risk(request.token_id)
        orderbook = clob_client.get_order_book(request.token_id)
        
        logger.info(f"市场信息: tick_size={tick_size}, neg_risk={neg_risk}")
        
        # 计算价格
        if side == BUY:
            if not orderbook.asks:
                return OrderResponse(success=False, error="订单簿没有卖单")
            # 取best_ask价格，加5%滑点
            best_ask_price = float(orderbook.asks[-1].price)
            price = min(best_ask_price * 1.05, 0.99)
        else:
            if not orderbook.bids:
                return OrderResponse(success=False, error="订单簿没有买单")
            # 取best_bid价格，减5%滑点
            best_bid_price = float(orderbook.bids[-1].price)
            price = max(best_bid_price * 0.95, 0.01)
        
        logger.info(f"计算价格: {price}")
        
        # 创建市价单参数
        market_order_args = MarketOrderArgs(
            token_id=request.token_id,
            amount=request.amount,
            side=side,
            price=price,
            fee_rate_bps=0,
            nonce=0,
            order_type=order_type,
        )
        
        # 创建并提交订单
        signed_order = clob_client.create_market_order(market_order_args)
        response = clob_client.post_order(signed_order, order_type)
        
        logger.info(f"订单响应: {response}")
        
        # 解析响应
        order_id = response.get('orderID') or response.get('order_id')
        status = response.get('status', 'unknown')
        
        return OrderResponse(
            success=True,
            order_id=order_id,
            status=status,
            data=response
        )
        
    except Exception as e:
        logger.error(f"市价单失败: {e}", exc_info=True)
        return OrderResponse(success=False, error=str(e))


@app.post("/order/limit", response_model=OrderResponse)
async def place_limit_order(request: LimitOrderRequest):
    """下限价单"""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB客户端未初始化")
    
    try:
        logger.info(f"限价单请求: token_id={request.token_id[:20]}..., side={request.side}, price={request.price}, size={request.size}")
        
        side = BUY if request.side.lower() == "buy" else SELL
        
        order_type_map = {
            "GTC": OrderType.GTC,
            "FOK": OrderType.FOK,
            "FAK": OrderType.FAK,
            "GTD": OrderType.GTD,
        }
        order_type = order_type_map.get(request.order_type.upper(), OrderType.GTC)
        
        # 创建限价单参数
        order_args = OrderArgs(
            token_id=request.token_id,
            price=request.price,
            size=request.size,
            side=side,
            fee_rate_bps=0,
            nonce=0,
            expiration=0,
        )
        
        # 创建并提交订单
        signed_order = clob_client.create_order(order_args)
        response = clob_client.post_order(signed_order, order_type)
        
        logger.info(f"订单响应: {response}")
        
        order_id = response.get('orderID') or response.get('order_id')
        status = response.get('status', 'unknown')
        
        return OrderResponse(
            success=True,
            order_id=order_id,
            status=status,
            data=response
        )
        
    except Exception as e:
        logger.error(f"限价单失败: {e}", exc_info=True)
        return OrderResponse(success=False, error=str(e))


@app.post("/order/cancel", response_model=OrderResponse)
async def cancel_order(request: CancelOrderRequest):
    """取消订单"""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB客户端未初始化")
    
    try:
        logger.info(f"取消订单: {request.order_id}")
        
        response = clob_client.cancel(request.order_id)
        
        return OrderResponse(
            success=True,
            order_id=request.order_id,
            status="cancelled",
            data=response if isinstance(response, dict) else {"result": response}
        )
        
    except Exception as e:
        logger.error(f"取消订单失败: {e}", exc_info=True)
        return OrderResponse(success=False, error=str(e))


@app.get("/orders")
async def get_orders():
    """获取订单列表"""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB客户端未初始化")
    
    try:
        orders = clob_client.get_orders()
        return {"success": True, "orders": orders}
    except Exception as e:
        logger.error(f"获取订单失败: {e}", exc_info=True)
        return {"success": False, "error": str(e), "orders": []}


@app.get("/balance")
async def get_balance():
    """获取余额"""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB客户端未初始化")
    
    try:
        from py_clob_client.clob_types import BalanceAllowanceParams
        params = BalanceAllowanceParams(asset_type="COLLATERAL", signature_type=-1)
        balance = clob_client.get_balance_allowance(params)
        return {"success": True, "balance": balance}
    except Exception as e:
        logger.error(f"获取余额失败: {e}", exc_info=True)
        return {"success": False, "error": str(e)}


# ==================== 主入口 ====================

if __name__ == "__main__":
    import uvicorn
    uvicorn.run(
        "main:app",
        host="127.0.0.1",
        port=8001,
        reload=False,
        log_level="info"
    )
