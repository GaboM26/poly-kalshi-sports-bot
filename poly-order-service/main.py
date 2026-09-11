#!/usr/bin/env python3
"""Polymarket US order service backed by the official polymarket-us SDK."""

import asyncio
import logging
import math
import os
import time
from contextlib import asynccontextmanager
from typing import Any, Optional

import toml
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field, field_validator, model_validator
from polymarket_us import PolymarketUS

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s",
)
logger = logging.getLogger("poly-order-service")

polymarket_client: Optional[PolymarketUS] = None
balance_cache: Optional[tuple[float, float, list[dict[str, Any]]]] = None
positions_cache: Optional[tuple[float, list[dict[str, Any]]]] = None
balance_lock = asyncio.Lock()
positions_lock = asyncio.Lock()

CACHE_TTL_SECONDS = 30

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
    position_side: Optional[str] = None
    # Compatibility only for direct clients that still use an endpoint where
    # YES/NO has a verified direction. Rust always sends position_side.
    outcome: Optional[str] = None
    side: str
    amount: float = Field(gt=0)
    price: Optional[float] = Field(default=None, gt=0, lt=1)
    order_type: str = "FAK"

    @field_validator("position_side")
    @classmethod
    def validate_position_side(cls, value: Optional[str]) -> Optional[str]:
        if value is None:
            return None
        position_side = value.lower()
        if position_side not in {"long", "short"}:
            raise ValueError("position_side must be 'long' or 'short'")
        return position_side

    @field_validator("outcome")
    @classmethod
    def validate_outcome(cls, value: Optional[str]) -> Optional[str]:
        if value is None:
            return None
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

    @model_validator(mode="after")
    def require_position_side(self) -> "MarketOrderRequest":
        if self.position_side is None:
            if self.outcome is None:
                raise ValueError("position_side is required")
            self.position_side = "long" if self.outcome == "yes" else "short"
        return self


class LimitOrderRequest(MarketOrderRequest):
    """An immediate-or-cancel Polymarket US limit order."""

    price: float = Field(gt=0, lt=1)
    size: float = Field(gt=0)
    amount: Optional[float] = None
    order_type: str = "FAK"


class CancelOrderRequest(BaseModel):
    """Cancel a Polymarket US order."""

    order_id: str = Field(min_length=1)
    market_slug: Optional[str] = Field(default=None, min_length=1)


class OrderResponse(BaseModel):
    success: bool
    order_id: Optional[str] = None
    status: Optional[str] = None
    filled_contracts: Optional[float] = None
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


def order_intent(position_side: str, side: str) -> str:
    """Build the official US SDK intent from an explicit position direction."""
    return f"ORDER_INTENT_{side.upper()}_{position_side.upper()}"


def order_status(order: dict[str, Any]) -> Optional[str]:
    return order.get("state") or order.get("status")


def amount_value(value: Any) -> Optional[float]:
    """Read the numeric value from the SDK's Amount object."""
    if isinstance(value, dict):
        value = value.get("value")
    try:
        amount = float(value)
    except (TypeError, ValueError):
        return None
    return amount if math.isfinite(amount) else None


def order_result(
    response: dict[str, Any],
    started: float,
) -> OrderResponse:
    """Convert the SDK execution envelope into an explicit order result."""
    latency_ms = int((asyncio.get_running_loop().time() - started) * 1000)
    executions = response.get("executions")
    if not isinstance(executions, list) or not executions:
        return OrderResponse(
            success=False,
            error="Polymarket US did not return an order execution",
            data=response,
            latency_ms=latency_ms,
        )

    execution = executions[-1]
    order = execution.get("order") if isinstance(execution, dict) else None
    if not isinstance(order, dict):
        return OrderResponse(
            success=False,
            error="Polymarket US execution did not include order details",
            data=response,
            latency_ms=latency_ms,
        )

    filled_contracts = amount_value(order.get("cumQuantity")) or 0.0
    status = order_status(order)
    if filled_contracts > 0:
        return OrderResponse(
            success=True,
            order_id=order.get("id"),
            status=status,
            filled_contracts=filled_contracts,
            data=response,
            latency_ms=latency_ms,
        )

    rejection_reason = execution.get("orderRejectReason") or execution.get("text")
    return OrderResponse(
        success=False,
        order_id=order.get("id"),
        status=status,
        filled_contracts=0.0,
        error=rejection_reason or f"Order received no fill (state: {status or 'unknown'})",
        data=response,
        latency_ms=latency_ms,
    )


def normalize_positions(response: dict[str, Any]) -> list[dict[str, Any]]:
    """Convert the SDK's market-keyed position map into a frontend-safe list."""
    positions = response.get("positions")
    if isinstance(positions, list):
        return positions
    if not isinstance(positions, dict):
        raise ValueError("Polymarket US positions response did not contain a position list or map")

    normalized = []
    for market_slug, position in positions.items():
        if not isinstance(position, dict):
            continue
        metadata = position.get("marketMetadata")
        if not isinstance(metadata, dict):
            metadata = {}
        normalized.append(
            {
                "id": market_slug,
                "asset": market_slug,
                "conditionId": market_slug,
                "title": metadata.get("title") or market_slug,
                "size": position.get("netPosition", "0"),
                "value": amount_value(position.get("cashValue")),
                "pnl": amount_value(position.get("realized")),
                "outcome": metadata.get("outcome"),
            }
        )
    return normalized


def concise_api_error(exc: Exception) -> str:
    """Keep upstream HTML error pages out of application logs and responses."""
    message = str(exc)
    if "1015" in message or "rate limited" in message.lower():
        return "Polymarket US is rate limiting this IP. Retry after the temporary restriction expires."
    return message[:500]


async def cached_balances() -> tuple[float, list[dict[str, Any]]]:
    """Fetch account balances at most once per cache period."""
    global balance_cache
    async with balance_lock:
        now = time.monotonic()
        if balance_cache is not None and now - balance_cache[0] < CACHE_TTL_SECONDS:
            return balance_cache[1], balance_cache[2]

        response = await asyncio.to_thread(get_client().account.balances)
        balances = response.get("balances")
        if not isinstance(balances, list):
            raise ValueError("Polymarket US balances response did not contain a balance list")
        buying_power = sum(
            amount_value(balance.get("buyingPower")) or 0.0
            for balance in balances
            if isinstance(balance, dict)
        )
        balance_cache = (now, buying_power, balances)
        return buying_power, balances


async def cached_buying_power() -> float:
    """Return cached account buying power."""
    buying_power, _ = await cached_balances()
    return buying_power


async def cached_positions() -> list[dict[str, Any]]:
    """Fetch positions at most once per cache period."""
    global positions_cache
    async with positions_lock:
        now = time.monotonic()
        if positions_cache is not None and now - positions_cache[0] < CACHE_TTL_SECONDS:
            return positions_cache[1]

        response = await asyncio.to_thread(get_client().portfolio.positions)
        positions = normalize_positions(response)
        positions_cache = (now, positions)
        return positions


def market_order_params(request: MarketOrderRequest) -> dict[str, Any]:
    """Build an SDK payload without applying client-side price fallbacks."""
    params: dict[str, Any] = {
        "marketSlug": request.market_slug,
        "intent": order_intent(request.position_side, request.side),
        "type": "ORDER_TYPE_MARKET",
        "tif": TIF_MAP.get(request.order_type.upper(), TIF_MAP["FAK"]),
        "manualOrderIndicator": "MANUAL_ORDER_INDICATOR_MANUAL",
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
        "intent": order_intent(request.position_side, request.side),
        "type": "ORDER_TYPE_LIMIT",
        "price": {"value": str(request.price), "currency": "USD"},
        "quantity": request.size,
        "tif": TIF_MAP.get(request.order_type.upper(), TIF_MAP["GTC"]),
        "manualOrderIndicator": "MANUAL_ORDER_INDICATOR_MANUAL",
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
        error = concise_api_error(exc)
        logger.error("Polymarket US market order failed: %s", error)
        return OrderResponse(success=False, error=error)

    return order_result(response, started)


@app.post("/order/limit", response_model=OrderResponse)
async def place_limit_order(request: LimitOrderRequest) -> OrderResponse:
    client = get_client()
    started = asyncio.get_running_loop().time()
    try:
        response = await asyncio.to_thread(client.orders.create, limit_order_params(request))
    except Exception as exc:
        error = concise_api_error(exc)
        logger.error("Polymarket US limit order failed: %s", error)
        return OrderResponse(success=False, error=error)

    return order_result(response, started)


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
        error = concise_api_error(exc)
        logger.error("Polymarket US cancellation failed: %s", error)
        return OrderResponse(success=False, error=error)

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
        error = concise_api_error(exc)
        logger.error("Polymarket US orders lookup failed: %s", error)
        return {"success": False, "error": error, "orders": []}


@app.get("/positions")
async def get_positions() -> dict[str, Any]:
    try:
        return {"success": True, "positions": await cached_positions()}
    except Exception as exc:
        error = concise_api_error(exc)
        logger.error("Polymarket US positions lookup failed: %s", error)
        return {"success": False, "error": error, "positions": []}


@app.get("/account/snapshot")
async def account_snapshot() -> dict[str, Any]:
    try:
        (buying_power, balances), positions, orders = await asyncio.gather(
            cached_balances(),
            cached_positions(),
            asyncio.to_thread(get_client().orders.list),
        )
        return {
            "success": True,
            "balance": buying_power,
            "snapshot": {
                "balances": balances,
                "positions": positions,
                "orders": orders.get("orders", []),
            },
        }
    except Exception as exc:
        error = concise_api_error(exc)
        logger.error("Polymarket US account snapshot failed: %s", error)
        return {"success": False, "error": error}


@app.get("/balance")
async def get_balance() -> dict[str, Any]:
    try:
        return {"success": True, "balance": await cached_buying_power()}
    except Exception as exc:
        error = concise_api_error(exc)
        logger.error("Polymarket US balance lookup failed: %s", error)
        return {"success": False, "error": error}


if __name__ == "__main__":
    import uvicorn

    uvicorn.run("main:app", host="127.0.0.1", port=8001, reload=False, log_level="info")
