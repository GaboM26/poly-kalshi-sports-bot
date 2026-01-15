#!/usr/bin/env python3
"""
Polymarket Order Service - FastAPI服务
处理Polymarket下单请求，使用官方py-clob-client SDK
"""

import os
import logging
import time
import asyncio
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
    price: Optional[float] = None  # 可选：由Rust传递的价格（避免重复获取订单簿）
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
    latency_ms: Optional[int] = None  # API调用延迟（毫秒）


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
    
    # === 修复 HTTP 客户端配置，防止长期运行超时 ===
    import httpx
    from py_clob_client.http_helpers import helpers
    
    # 配置更健壮的 HTTP 客户端
    timeout = httpx.Timeout(
        connect=10.0,   # 连接超时 10 秒
        read=30.0,      # 读取超时 30 秒
        write=10.0,     # 写入超时 10 秒
        pool=5.0        # 连接池获取超时 5 秒
    )
    
    limits = httpx.Limits(
        max_keepalive_connections=5,  # 最多保持 5 个 keep-alive 连接
        max_connections=10,            # 最大连接数 10
        keepalive_expiry=30.0          # keep-alive 连接 30 秒后过期
    )
    
    # 替换全局 HTTP 客户端
    helpers._http_client = httpx.Client(
        http2=True,
        timeout=timeout,
        limits=limits,
        transport=httpx.HTTPTransport(retries=2)  # 自动重试 2 次
    )
    
    logger.info("✅ HTTP 客户端配置完成: timeout=30s, keepalive_expiry=30s, retries=2")
    # === 修复部分结束 ===
    
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


# ==================== 后台任务 ====================

async def periodic_http_client_refresh():
    """每 30 分钟重建一次 HTTP 客户端，防止连接僵死"""
    while True:
        await asyncio.sleep(1800)  # 30 分钟
        try:
            from py_clob_client.http_helpers import helpers
            import httpx
            
            # 关闭旧客户端
            old_client = helpers._http_client
            if old_client:
                old_client.close()
            
            # 创建新客户端
            timeout = httpx.Timeout(
                connect=10.0,
                read=30.0,
                write=10.0,
                pool=5.0
            )
            limits = httpx.Limits(
                max_keepalive_connections=5,
                max_connections=10,
                keepalive_expiry=30.0
            )
            helpers._http_client = httpx.Client(
                http2=True,
                timeout=timeout,
                limits=limits,
                transport=httpx.HTTPTransport(retries=2)
            )
            logger.info("🔄 HTTP 客户端已定期重建（防止连接僵死）")
        except Exception as e:
            logger.error(f"重建 HTTP 客户端失败: {e}")


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
    
    # 启动定期刷新任务
    refresh_task = asyncio.create_task(periodic_http_client_refresh())
    logger.info("✅ 已启动 HTTP 客户端定期刷新任务（每 30 分钟）")
    
    yield
    
    # 关闭时清理
    refresh_task.cancel()
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
        
        # 计算价格：优先使用Rust传递的价格，否则从订单簿获取
        if request.price is not None:
            # Rust已经计算好价格（包含滑点），直接使用
            price = min(max(request.price, 0.01), 0.99)  # 确保在有效范围内
            logger.info(f"使用Rust传递的价格: {price}")
        else:
            # 兼容模式：从订单簿获取价格（旧逻辑）
            logger.info("未提供价格，从订单簿获取...")
            orderbook = clob_client.get_order_book(request.token_id)
            
            if side == BUY:
                if not orderbook.asks:
                    return OrderResponse(success=False, error="订单簿没有卖单")
                # 取best_ask价格，加0.01固定滑点（与Kalshi的+1¢策略一致）
                best_ask_price = float(orderbook.asks[-1].price)
                price = min(best_ask_price + 0.01, 0.99)
            else:
                if not orderbook.bids:
                    return OrderResponse(success=False, error="订单簿没有买单")
                # 取best_bid价格，减0.01固定滑点
                best_bid_price = float(orderbook.bids[-1].price)
                price = max(best_bid_price - 0.01, 0.01)
            
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
        
        # 创建并提交订单（测量延迟）- 添加重试逻辑
        api_start = time.time()
        signed_order = clob_client.create_market_order(market_order_args)
        
        # 添加重试机制，处理超时问题
        max_retries = 3
        last_error = None
        for attempt in range(max_retries):
            try:
                response = clob_client.post_order(signed_order, order_type)
                break
            except Exception as e:
                last_error = e
                error_msg = str(e)
                
                # 如果是超时错误，尝试重建 HTTP 客户端
                if "timeout" in error_msg.lower() or "timed out" in error_msg.lower():
                    logger.warning(f"⚠️  检测到超时错误 (尝试 {attempt + 1}/{max_retries}): {error_msg}")
                    
                    if attempt < max_retries - 1:
                        # 重建 HTTP 客户端
                        from py_clob_client.http_helpers import helpers
                        import httpx
                        
                        old_client = helpers._http_client
                        if old_client:
                            old_client.close()
                        
                        timeout = httpx.Timeout(
                            connect=10.0,
                            read=30.0,
                            write=10.0,
                            pool=5.0
                        )
                        limits = httpx.Limits(
                            max_keepalive_connections=5,
                            max_connections=10,
                            keepalive_expiry=30.0
                        )
                        helpers._http_client = httpx.Client(
                            http2=True,
                            timeout=timeout,
                            limits=limits,
                            transport=httpx.HTTPTransport(retries=2)
                        )
                        logger.info("🔄 已重建 HTTP 客户端，重试中...")
                        time.sleep(1)  # 等待 1 秒后重试
                    else:
                        raise last_error
                else:
                    # 非超时错误，直接抛出
                    raise
        
        latency_ms = int((time.time() - api_start) * 1000)
        
        logger.info(f"订单响应: {response} (延迟: {latency_ms}ms)")
        
        # 解析响应
        order_id = response.get('orderID') or response.get('order_id')
        status = response.get('status', 'unknown')
        
        # 检查订单状态：只有 MATCHED 才算成功
        # delayed 状态表示订单未立即成交，视为失败
        is_success = status.upper() == 'MATCHED'
        
        if not is_success:
            logger.warning(f"⚠️ 订单状态为 {status}，未立即成交，视为失败")
            if status == 'delayed':
                logger.warning(f"   订单 {order_id} 可能因价格不够激进而未成交")
        
        return OrderResponse(
            success=is_success,
            order_id=order_id,
            status=status,
            data=response,
            latency_ms=latency_ms
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
        
        # 创建并提交订单（测量延迟）- 添加重试逻辑
        api_start = time.time()
        signed_order = clob_client.create_order(order_args)
        
        # 添加重试机制，处理超时问题
        max_retries = 3
        last_error = None
        for attempt in range(max_retries):
            try:
                response = clob_client.post_order(signed_order, order_type)
                break
            except Exception as e:
                last_error = e
                error_msg = str(e)
                
                # 如果是超时错误，尝试重建 HTTP 客户端
                if "timeout" in error_msg.lower() or "timed out" in error_msg.lower():
                    logger.warning(f"⚠️  检测到超时错误 (尝试 {attempt + 1}/{max_retries}): {error_msg}")
                    
                    if attempt < max_retries - 1:
                        # 重建 HTTP 客户端
                        from py_clob_client.http_helpers import helpers
                        import httpx
                        
                        old_client = helpers._http_client
                        if old_client:
                            old_client.close()
                        
                        timeout = httpx.Timeout(
                            connect=10.0,
                            read=30.0,
                            write=10.0,
                            pool=5.0
                        )
                        limits = httpx.Limits(
                            max_keepalive_connections=5,
                            max_connections=10,
                            keepalive_expiry=30.0
                        )
                        helpers._http_client = httpx.Client(
                            http2=True,
                            timeout=timeout,
                            limits=limits,
                            transport=httpx.HTTPTransport(retries=2)
                        )
                        logger.info("🔄 已重建 HTTP 客户端，重试中...")
                        time.sleep(1)  # 等待 1 秒后重试
                    else:
                        raise last_error
                else:
                    # 非超时错误，直接抛出
                    raise
        
        latency_ms = int((time.time() - api_start) * 1000)
        
        logger.info(f"订单响应: {response} (延迟: {latency_ms}ms)")
        
        order_id = response.get('orderID') or response.get('order_id')
        status = response.get('status', 'unknown')
        
        # 检查订单状态：只有 MATCHED 才算成功
        # delayed 状态表示订单未立即成交，视为失败
        is_success = status.upper() == 'MATCHED'
        
        if not is_success:
            logger.warning(f"⚠️ 订单状态为 {status}，未立即成交，视为失败")
            if status == 'delayed':
                logger.warning(f"   订单 {order_id} 可能因价格不够激进而未成交")
        
        return OrderResponse(
            success=is_success,
            order_id=order_id,
            status=status,
            data=response,
            latency_ms=latency_ms
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
        
        # 测量延迟
        api_start = time.time()
        response = clob_client.cancel(request.order_id)
        latency_ms = int((time.time() - api_start) * 1000)
        
        logger.info(f"取消订单响应: {response} (延迟: {latency_ms}ms)")
        
        return OrderResponse(
            success=True,
            order_id=request.order_id,
            status="cancelled",
            data=response if isinstance(response, dict) else {"result": response},
            latency_ms=latency_ms
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
