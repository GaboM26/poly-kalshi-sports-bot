#!/usr/bin/env python3
"""Polymarket US order service backed by the official polymarket-us SDK."""

import asyncio
import logging
import os
from contextlib import asynccontextmanager
from typing import Any, Optional

import toml
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field, field_validator
from polymarket_us import PolymarketUS

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
)
logger = logging.getLogger("poly-order-service")

polymarket_client: Optional[PolymarketUS] = None

TIF_MAP = {
    "GTC": "TIME_IN_FORCE_GOOD_TILL_CANCEL",
    "GTD": "TIME_IN_FORCE_GOOD_TILL_DATE",
    "FAK": "TIME_IN_FORCE_IMMEDIATE_OR_CANCEL",
    "IOC": "TIME_IN_FORCE_IMMEDIATE_OR_CANCEL",
    "FOK": "TIME_IN_FORCE_FILL_OR_KILL",
}


class MarketOrderRequest(BaseModel):
    """A Polymarket US market order."""

    market_slug: str = Field(min_length=1)
    outcome: str
    side: str
    amount: float = Field(gt=0)
    price: Optional[float] = Field(default=None, gt=0, lt=1)
    order_type: str = "FAK"

    @field_validator("outcome")
    @classmethod
    def validate_outcome(cls, value: str) -> str:
        outcome = value.lower()
        if outcome not in {"yes", "no"}:
            raise ValueError("outcome must be 'yes' or 'no'")
        return outcome

    @field_validator("side")
    @classmethod
    def validate_side(cls, value: str) -> str:
        side = value.lower()
        if side not in {"buy", "sell"}:
            raise ValueError("side must be 'buy' or 'sell'")
        return side


class LimitOrderRequest(MarketOrderRequest):
    """A Polymarket US limit order."""

    price: float = Field(gt=0, lt=1)
    size: float = Field(gt=0)
    amount: Optional[float] = None
    order_type: str = "GTC"


class CancelOrderRequest(BaseModel):
    """Cancel a Polymarket US order."""

    order_id: str = Field(min_length=1)
    market_slug: Optional[str] = Field(default=None, min_length=1)


class OrderResponse(BaseModel):
    success: bool
    order_id: Optional[str] = None
    status: Optional[str] = None
    error: Optional[str] = None
    data: Optional[dict[str, Any]] = None
    latency_ms: Optional[int] = None


def load_config() -> dict[str, Any]:
    """Load the first project configuration file found."""
    config_paths = [
        os.path.join(os.path.dirname(__file__), "config.toml"),
        os.path.join(os.path.dirname(__file__), "..", "rust-backend", "config.toml"),
        os.path.join(os.path.dirname(__file__), "..", "config.toml"),
    ]
    for path in config_paths:
        if os.path.exists(path):
            logger.info("Loading configuration file: %s", path)
            return toml.load(path)
    raise FileNotFoundError("Configuration file config.toml was not found")


def us_credentials(config: dict[str, Any]) -> tuple[str, str]:
    """Read US API credentials, preferring environment variables."""
    polymarket = config.get("polymarket", {})
    key_id = os.getenv("POLYMARKET_KEY_ID") or polymarket.get("key_id")
    secret_key = os.getenv("POLYMARKET_SECRET_KEY") or polymarket.get("secret_key")
    if not key_id or not secret_key:
        raise ValueError(
            "Polymarket US credentials are required. Set POLYMARKET_KEY_ID and "
            "POLYMARKET_SECRET_KEY, or configure polymarket.key_id and "
            "polymarket.secret_key."
        )
    return key_id, secret_key


def init_polymarket_client() -> PolymarketUS:
    """Initialize the official Polymarket US API client."""
    global polymarket_client
    key_id, secret_key = us_credentials(load_config())
    polymarket_client = PolymarketUS(
        key_id=key_id,
        secret_key=secret_key,
        timeout=30.0,
    )
    logger.info("Initialized Polymarket US client with API key ID %s...", key_id[:8])
    return polymarket_client


def order_intent(outcome: str, side: str) -> str:
    """Map the local YES/NO action to the US API's explicit order intent."""
    return f"ORDER_INTENT_{side.upper()}_{'LONG' if outcome == 'yes' else 'SHORT'}"


def order_status(order: dict[str, Any]) -> Optional[str]:
    return order.get("state") or order.get("status")


def market_order_params(request: MarketOrderRequest) -> dict[str, Any]:
    """Build an SDK payload without applying client-side price fallbacks."""
    params: dict[str, Any] = {
        "marketSlug": request.market_slug,
        "intent": order_intent(request.outcome, request.side),
        "type": "ORDER_TYPE_MARKET",
        "tif": TIF_MAP.get(request.order_type.upper(), TIF_MAP["FAK"]),
        "manualOrderIndicator": "MANUAL_ORDER_INDICATOR_AUTOMATIC",
        "synchronousExecution": True,
        "maxBlockTime": "5",
    }
    if request.side == "buy":
        params["cashOrderQty"] = {"value": str(request.amount), "currency": "USD"}
    else:
        params["quantity"] = request.amount
    if request.price is not None:
        params["slippageTolerance"] = {
            "currentPrice": {"value": str(request.price), "currency": "USD"},
            "ticks": 2,
        }
    return params


def limit_order_params(request: LimitOrderRequest) -> dict[str, Any]:
    return {
        "marketSlug": request.market_slug,
        "intent": order_intent(request.outcome, request.side),
        "type": "ORDER_TYPE_LIMIT",
        "price": {"value": str(request.price), "currency": "USD"},
        "quantity": request.size,
        "tif": TIF_MAP.get(request.order_type.upper(), TIF_MAP["GTC"]),
        "manualOrderIndicator": "MANUAL_ORDER_INDICATOR_AUTOMATIC",
    }


def get_client() -> PolymarketUS:
    if polymarket_client is None:
        raise HTTPException(status_code=503, detail="Polymarket US client is not initialized")
    return polymarket_client


@asynccontextmanager
async def lifespan(_: FastAPI):
    init_polymarket_client()
    try:
        yield
    finally:
        if polymarket_client is not None:
            polymarket_client.close()


app = FastAPI(
    title="Polymarket US Order Service",
    description="Polymarket US order service using the official polymarket-us SDK",
    version="2.0.0",
    lifespan=lifespan,
)


@app.get("/health")
async def health_check() -> dict[str, Any]:
    return {
        "status": "healthy" if polymarket_client is not None else "unavailable",
        "client_initialized": polymarket_client is not None,
        "platform": "polymarket-us",
    }


@app.post("/order/market", response_model=OrderResponse)
async def place_market_order(request: MarketOrderRequest) -> OrderResponse:
    client = get_client()
    started = asyncio.get_running_loop().time()
    try:
        response = await asyncio.to_thread(client.orders.create, market_order_params(request))
    except Exception as exc:
        logger.error("Polymarket US market order failed: %s", exc)
        return OrderResponse(success=False, error=str(exc))

    return OrderResponse(
        success=True,
        order_id=response.get("id"),
        status=order_status(response),
        data=response,
        latency_ms=int((asyncio.get_running_loop().time() - started) * 1000),
    )


@app.post("/order/limit", response_model=OrderResponse)
async def place_limit_order(request: LimitOrderRequest) -> OrderResponse:
    client = get_client()
    started = asyncio.get_running_loop().time()
    try:
        response = await asyncio.to_thread(client.orders.create, limit_order_params(request))
    except Exception as exc:
        logger.error("Polymarket US limit order failed: %s", exc)
        return OrderResponse(success=False, error=str(exc))

    return OrderResponse(
        success=True,
        order_id=response.get("id"),
        status=order_status(response),
        data=response,
        latency_ms=int((asyncio.get_running_loop().time() - started) * 1000),
    )


@app.post("/order/cancel", response_model=OrderResponse)
async def cancel_order(request: CancelOrderRequest) -> OrderResponse:
    client = get_client()
    started = asyncio.get_running_loop().time()
    try:
        market_slug = request.market_slug
        if market_slug is None:
            order = await asyncio.to_thread(client.orders.retrieve, request.order_id)
            market_slug = order.get("marketSlug")
        if not market_slug:
            raise ValueError("Polymarket US did not return a marketSlug for this order")
        response = await asyncio.to_thread(
            client.orders.cancel,
            request.order_id,
            {"marketSlug": market_slug},
        )
    except Exception as exc:
        logger.error("Polymarket US cancellation failed: %s", exc)
        return OrderResponse(success=False, error=str(exc))

    return OrderResponse(
        success=True,
        order_id=request.order_id,
        status="cancelled",
        data=response if isinstance(response, dict) else {"result": response},
        latency_ms=int((asyncio.get_running_loop().time() - started) * 1000),
    )


@app.get("/orders")
async def get_orders() -> dict[str, Any]:
    try:
        response = await asyncio.to_thread(get_client().orders.list)
        return {"success": True, "orders": response.get("orders", [])}
    except Exception as exc:
        logger.error("Polymarket US orders lookup failed: %s", exc)
        return {"success": False, "error": str(exc), "orders": []}


@app.get("/positions")
async def get_positions() -> dict[str, Any]:
    try:
        response = await asyncio.to_thread(get_client().portfolio.positions)
        return {"success": True, "positions": response.get("positions", [])}
    except Exception as exc:
        logger.error("Polymarket US positions lookup failed: %s", exc)
        return {"success": False, "error": str(exc), "positions": []}


@app.get("/account/snapshot")
async def account_snapshot() -> dict[str, Any]:
    try:
        client = get_client()
        balances, positions, orders = await asyncio.gather(
            asyncio.to_thread(client.account.balances),
            asyncio.to_thread(client.portfolio.positions),
            asyncio.to_thread(client.orders.list),
        )
        balance_list = balances.get("balances", [])
        buying_power = sum(
            float(balance.get("buyingPower", 0)) for balance in balance_list
        )
        return {
            "success": True,
            "balance": buying_power,
            "snapshot": {
                "balances": balance_list,
                "positions": positions.get("positions", []),
                "orders": orders.get("orders", []),
            },
        }
    except Exception as exc:
        logger.error("Polymarket US account snapshot failed: %s", exc)
        return {"success": False, "error": str(exc)}


@app.get("/balance")
async def get_balance() -> dict[str, Any]:
    snapshot = await account_snapshot()
    if snapshot["success"]:
        return {"success": True, "balance": snapshot["snapshot"]["balances"]}
    return snapshot


if __name__ == "__main__":
    import uvicorn

    uvicorn.run("main:app", host="127.0.0.1", port=8001, reload=False, log_level="info")
