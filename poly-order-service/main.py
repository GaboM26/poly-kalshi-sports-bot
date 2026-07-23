#!/usr/bin/env python3
"""
Polymarket Order Service - FastAPI Service
Handles Polymarket order requests using the official py-clob-client SDK.
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

# Configure logging.
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger("poly-order-service")

# Global CLOB client.
clob_client: Optional[ClobClient] = None


# ==================== Request/Response Models ====================

class MarketOrderRequest(BaseModel):
    """Market order request."""
    token_id: str
    side: str  # "buy" or "sell"
    amount: float  # BUY: USDC amount, SELL: token quantity
    price: Optional[float] = None  # Optional price supplied by Rust (avoids fetching the order book again).
    order_type: Optional[str] = "FAK"  # GTC, FOK, FAK


class LimitOrderRequest(BaseModel):
    """Limit order request."""
    token_id: str
    side: str  # "buy" or "sell"
    price: float  # 0.0 - 1.0
    size: float  # Token quantity
    order_type: Optional[str] = "GTC"


class CancelOrderRequest(BaseModel):
    """Cancel order request."""
    order_id: str


class OrderResponse(BaseModel):
    """Order response."""
    success: bool
    order_id: Optional[str] = None
    status: Optional[str] = None
    error: Optional[str] = None
    data: Optional[dict] = None
    latency_ms: Optional[int] = None  # API call latency (milliseconds)


# ==================== Configuration Loading ====================

def load_config():
    """Load configuration from a configuration file."""
    config_paths = [
        os.path.join(os.path.dirname(__file__), 'config.toml'),
        os.path.join(os.path.dirname(__file__), '..', 'rust-backend', 'config.toml'),
        os.path.join(os.path.dirname(__file__), '..', 'config.toml'),
    ]
    
    for path in config_paths:
        if os.path.exists(path):
            logger.info(f"Loading configuration file: {path}")
            return toml.load(path)
    
    raise FileNotFoundError("Configuration file config.toml was not found")


def init_clob_client():
    """Initialize the CLOB client."""
    global clob_client
    
    # === Configure the HTTP client to prevent long-running timeout issues ===
    import httpx
    from py_clob_client.http_helpers import helpers
    
    # Configure a more robust HTTP client.
    timeout = httpx.Timeout(
        connect=10.0,   # 10-second connection timeout
        read=30.0,      # 30-second read timeout
        write=10.0,     # 10-second write timeout
        pool=5.0        # 5-second connection-pool timeout
    )
    
    limits = httpx.Limits(
        max_keepalive_connections=5,  # Keep at most five keep-alive connections.
        max_connections=10,            # Maximum of 10 connections.
        keepalive_expiry=30.0          # Keep-alive connections expire after 30 seconds.
    )
    
    # Replace the global HTTP client.
    helpers._http_client = httpx.Client(
        http2=True,
        timeout=timeout,
        limits=limits,
        transport=httpx.HTTPTransport(retries=2)  # Retry automatically twice.
    )
    
    logger.info("✅ HTTP client configured: timeout=30s, keepalive_expiry=30s, retries=2")
    # === End configuration ===
    
    config = load_config()
    poly_config = config.get('polymarket', {})
    
    host = poly_config.get('clob_url', 'https://clob.polymarket.com')
    private_key = poly_config.get('private_key')
    wallet_address = poly_config.get('wallet_address')
    signature_type = poly_config.get('signature_type', 1)  # 1 = PolyProxy (Magic Link)
    chain_id = 137  # Polygon Mainnet
    
    if not private_key:
        raise ValueError("Configuration file is missing polymarket.private_key")
    
    logger.info(f"Initializing CLOB client: host={host}, chain_id={chain_id}")
    logger.info(f"Wallet address: {wallet_address}")
    
    # Create the client.
    clob_client = ClobClient(
        host,
        chain_id=chain_id,
        key=private_key,
        signature_type=signature_type,
        funder=wallet_address
    )
    
    # Derive API credentials.
    logger.info("Deriving API credentials...")
    creds = clob_client.create_or_derive_api_creds()
    if creds:
        clob_client.set_api_creds(creds)
        logger.info(f"API credentials derived: {creds.api_key[:20]}...")
    else:
        raise ValueError("Unable to derive API credentials")
    
    return clob_client


# ==================== Background Tasks ====================

async def periodic_http_client_refresh():
    """Rebuild the HTTP client every 30 minutes to prevent stale connections."""
    while True:
        await asyncio.sleep(1800)  # 30 minutes
        try:
            from py_clob_client.http_helpers import helpers
            import httpx
            
            # Close the old client.
            old_client = helpers._http_client
            if old_client:
                old_client.close()
            
            # Create a new client.
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
            logger.info("🔄 HTTP client rebuilt periodically to prevent stale connections")
        except Exception as e:
            logger.error(f"Failed to rebuild HTTP client: {e}")


# ==================== FastAPI Application ====================

@asynccontextmanager
async def lifespan(app: FastAPI):
    """Manage the application lifecycle."""
    # Initialize at startup.
    logger.info("Starting Polymarket order service...")
    try:
        init_clob_client()
        logger.info("CLOB client initialized successfully")
    except Exception as e:
        logger.error(f"Failed to initialize CLOB client: {e}")
        raise
    
    # Start the periodic refresh task.
    refresh_task = asyncio.create_task(periodic_http_client_refresh())
    logger.info("✅ Started periodic HTTP client refresh task (every 30 minutes)")
    
    yield
    
    # Clean up at shutdown.
    refresh_task.cancel()
    logger.info("Stopping Polymarket order service...")


app = FastAPI(
    title="Polymarket Order Service",
    description="Polymarket order service using the official py-clob-client SDK",
    version="1.0.0",
    lifespan=lifespan
)


# ==================== API Endpoints ====================

@app.get("/health")
async def health_check():
    """Health check."""
    return {
        "status": "healthy",
        "client_initialized": clob_client is not None,
        "address": clob_client.get_address() if clob_client else None
    }


@app.post("/order/market", response_model=OrderResponse)
async def place_market_order(request: MarketOrderRequest):
    """Place a market order."""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB client is not initialized")
    
    try:
        logger.info(f"Market order request: token_id={request.token_id[:20]}..., side={request.side}, amount={request.amount}")
        
        # Determine the side.
        side = BUY if request.side.lower() == "buy" else SELL
        
        # Determine the order type.
        order_type_map = {
            "GTC": OrderType.GTC,
            "FOK": OrderType.FOK,
            "FAK": OrderType.FAK,
            "GTD": OrderType.GTD,
        }
        order_type = order_type_map.get(request.order_type.upper(), OrderType.FAK)
        
        # Calculate the price: prefer the price supplied by Rust, otherwise fetch the order book.
        if request.price is not None:
            # Rust has already calculated the price (including slippage), so use it directly.
            price = min(max(request.price, 0.01), 0.99)  # Ensure it is within the valid range.
            logger.info(f"Using price supplied by Rust: {price}")
        else:
            # Compatibility mode: get the price from the order book (legacy behavior).
            logger.info("No price supplied; fetching from order book...")
            orderbook = clob_client.get_order_book(request.token_id)
            
            if side == BUY:
                if not orderbook.asks:
                    return OrderResponse(success=False, error="Order book has no asks")
                # Use best ask plus fixed 0.01 slippage (consistent with Kalshi's +1¢ strategy).
                best_ask_price = float(orderbook.asks[-1].price)
                price = min(best_ask_price + 0.01, 0.99)
            else:
                if not orderbook.bids:
                    return OrderResponse(success=False, error="Order book has no bids")
                # Use best bid minus fixed 0.01 slippage.
                best_bid_price = float(orderbook.bids[-1].price)
                price = max(best_bid_price - 0.01, 0.01)
            
            logger.info(f"Calculated price: {price}")
        
        # Create market order parameters.
        market_order_args = MarketOrderArgs(
            token_id=request.token_id,
            amount=request.amount,
            side=side,
            price=price,
            fee_rate_bps=0,
            nonce=0,
            order_type=order_type,
        )
        
        # Create and submit the order (measuring latency) with retry logic.
        api_start = time.time()
        signed_order = clob_client.create_market_order(market_order_args)
        
        # Add retries to handle timeouts.
        max_retries = 3
        last_error = None
        for attempt in range(max_retries):
            try:
                response = clob_client.post_order(signed_order, order_type)
                break
            except Exception as e:
                last_error = e
                error_msg = str(e)
                
                # Rebuild the HTTP client for timeout errors.
                if "timeout" in error_msg.lower() or "timed out" in error_msg.lower():
                    logger.warning(f"⚠️  Timeout error detected (attempt {attempt + 1}/{max_retries}): {error_msg}")
                    
                    if attempt < max_retries - 1:
                        # Rebuild the HTTP client.
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
                        logger.info("🔄 HTTP client rebuilt; retrying...")
                        time.sleep(1)  # Wait one second before retrying.
                    else:
                        raise last_error
                else:
                    # Raise non-timeout errors directly.
                    raise
        
        latency_ms = int((time.time() - api_start) * 1000)
        
        logger.info(f"Order response: {response} (latency: {latency_ms}ms)")
        
        # Parse the response.
        order_id = response.get('orderID') or response.get('order_id')
        status = response.get('status', 'unknown')
        
        # Check the order status: only MATCHED counts as success.
        # The delayed status means the order did not fill immediately and is treated as failure.
        is_success = status.upper() == 'MATCHED'
        
        if not is_success:
            logger.warning(f"⚠️ Order status is {status}; it did not fill immediately and is treated as failure")
            if status == 'delayed':
                logger.warning(f"   Order {order_id} may not have filled because the price was not aggressive enough")
        
        return OrderResponse(
            success=is_success,
            order_id=order_id,
            status=status,
            data=response,
            latency_ms=latency_ms
        )
        
    except Exception as e:
        logger.error(f"Market order failed: {e}", exc_info=True)
        return OrderResponse(success=False, error=str(e))


@app.post("/order/limit", response_model=OrderResponse)
async def place_limit_order(request: LimitOrderRequest):
    """Place a limit order."""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB client is not initialized")
    
    try:
        logger.info(f"Limit order request: token_id={request.token_id[:20]}..., side={request.side}, price={request.price}, size={request.size}")
        
        side = BUY if request.side.lower() == "buy" else SELL
        
        order_type_map = {
            "GTC": OrderType.GTC,
            "FOK": OrderType.FOK,
            "FAK": OrderType.FAK,
            "GTD": OrderType.GTD,
        }
        order_type = order_type_map.get(request.order_type.upper(), OrderType.GTC)
        
        # Create limit order parameters.
        order_args = OrderArgs(
            token_id=request.token_id,
            price=request.price,
            size=request.size,
            side=side,
            fee_rate_bps=0,
            nonce=0,
            expiration=0,
        )
        
        # Create and submit the order (measuring latency) with retry logic.
        api_start = time.time()
        signed_order = clob_client.create_order(order_args)
        
        # Add retries to handle timeouts.
        max_retries = 3
        last_error = None
        for attempt in range(max_retries):
            try:
                response = clob_client.post_order(signed_order, order_type)
                break
            except Exception as e:
                last_error = e
                error_msg = str(e)
                
                # Rebuild the HTTP client for timeout errors.
                if "timeout" in error_msg.lower() or "timed out" in error_msg.lower():
                    logger.warning(f"⚠️  Timeout error detected (attempt {attempt + 1}/{max_retries}): {error_msg}")
                    
                    if attempt < max_retries - 1:
                        # Rebuild the HTTP client.
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
                        logger.info("🔄 HTTP client rebuilt; retrying...")
                        time.sleep(1)  # Wait one second before retrying.
                    else:
                        raise last_error
                else:
                    # Raise non-timeout errors directly.
                    raise
        
        latency_ms = int((time.time() - api_start) * 1000)
        
        logger.info(f"Order response: {response} (latency: {latency_ms}ms)")
        
        order_id = response.get('orderID') or response.get('order_id')
        status = response.get('status', 'unknown')
        
        # Check the order status: only MATCHED counts as success.
        # The delayed status means the order did not fill immediately and is treated as failure.
        is_success = status.upper() == 'MATCHED'
        
        if not is_success:
            logger.warning(f"⚠️ Order status is {status}; it did not fill immediately and is treated as failure")
            if status == 'delayed':
                logger.warning(f"   Order {order_id} may not have filled because the price was not aggressive enough")
        
        return OrderResponse(
            success=is_success,
            order_id=order_id,
            status=status,
            data=response,
            latency_ms=latency_ms
        )
        
    except Exception as e:
        logger.error(f"Limit order failed: {e}", exc_info=True)
        return OrderResponse(success=False, error=str(e))


@app.post("/order/cancel", response_model=OrderResponse)
async def cancel_order(request: CancelOrderRequest):
    """Cancel an order."""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB client is not initialized")
    
    try:
        logger.info(f"Cancelling order: {request.order_id}")
        
        # Measure latency.
        api_start = time.time()
        response = clob_client.cancel(request.order_id)
        latency_ms = int((time.time() - api_start) * 1000)
        
        logger.info(f"Cancel order response: {response} (latency: {latency_ms}ms)")
        
        return OrderResponse(
            success=True,
            order_id=request.order_id,
            status="cancelled",
            data=response if isinstance(response, dict) else {"result": response},
            latency_ms=latency_ms
        )
        
    except Exception as e:
        logger.error(f"Failed to cancel order: {e}", exc_info=True)
        return OrderResponse(success=False, error=str(e))


@app.get("/orders")
async def get_orders():
    """Get the order list."""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB client is not initialized")
    
    try:
        orders = clob_client.get_orders()
        return {"success": True, "orders": orders}
    except Exception as e:
        logger.error(f"Failed to get orders: {e}", exc_info=True)
        return {"success": False, "error": str(e), "orders": []}


@app.get("/balance")
async def get_balance():
    """Get the balance."""
    if not clob_client:
        raise HTTPException(status_code=503, detail="CLOB client is not initialized")
    
    try:
        from py_clob_client.clob_types import BalanceAllowanceParams
        params = BalanceAllowanceParams(asset_type="COLLATERAL", signature_type=-1)
        balance = clob_client.get_balance_allowance(params)
        return {"success": True, "balance": balance}
    except Exception as e:
        logger.error(f"Failed to get balance: {e}", exc_info=True)
        return {"success": False, "error": str(e)}


# ==================== Main Entry Point ====================

if __name__ == "__main__":
    import uvicorn
    uvicorn.run(
        "main:app",
        host="127.0.0.1",
        port=8001,
        reload=False,
        log_level="info"
    )
