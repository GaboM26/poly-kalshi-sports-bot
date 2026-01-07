"""Kalshi API 客户端"""
import aiohttp
import asyncio
import websockets
import json
import logging
import time
import base64
from typing import List, Optional, Dict, Callable
from datetime import datetime
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.backends import default_backend
from app.core.models import KalshiMarket, KalshiEvent, Platform, PriceUpdate
from app.core.config import KalshiConfig

logger = logging.getLogger(__name__)


class KalshiClient:
    """Kalshi API 客户端"""
    
    def __init__(self, config: KalshiConfig):
        self.config = config
        self.base_url = config.base_url
        self.ws_url = "wss://api.elections.kalshi.com/trade-api/ws/v2"
        self.session: Optional[aiohttp.ClientSession] = None
        self.private_key = None
        self.ws_connection = None
        self.ws_connected = False
        
        # 加载私钥
        try:
            self.private_key = serialization.load_pem_private_key(
                config.api_secret.encode(),
                password=None,
                backend=default_backend()
            )
            logger.info("✅ Kalshi 私钥加载成功")
        except Exception as e:
            logger.error(f"❌ 加载 Kalshi 私钥失败: {e}")
        
    async def __aenter__(self):
        self.session = aiohttp.ClientSession()
        return self
        
    async def __aexit__(self, exc_type, exc_val, exc_tb):
        if self.session:
            await self.session.close()
    
    def _sign_request(self, timestamp_ms: int, method: str, path: str) -> str:
        """生成请求签名"""
        try:
            message = f"{timestamp_ms}{method}{path}"
            signature = self.private_key.sign(
                message.encode(),
                padding.PSS(
                    mgf=padding.MGF1(hashes.SHA256()),
                    salt_length=padding.PSS.MAX_LENGTH
                ),
                hashes.SHA256()
            )
            return base64.b64encode(signature).decode()
        except Exception as e:
            logger.error(f"签名失败: {e}")
            return ""
    
    def _get_headers(self, method: str, path: str) -> Dict[str, str]:
        """获取请求头（带签名）"""
        timestamp_ms = int(time.time() * 1000)
        signature = self._sign_request(timestamp_ms, method, path)
        return {
            "Content-Type": "application/json",
            "KALSHI-ACCESS-KEY": self.config.api_key,
            "KALSHI-ACCESS-TIMESTAMP": str(timestamp_ms),
            "KALSHI-ACCESS-SIGNATURE": signature,
        }
    
    async def login(self) -> bool:
        """测试连接"""
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            path = "/trade-api/v2/exchange/status"
            url = f"{self.base_url}/exchange/status"
            headers = self._get_headers("GET", path)
            
            async with self.session.get(url, headers=headers) as resp:
                if resp.status == 200:
                    logger.info("✅ Kalshi 连接成功")
                    return True
                else:
                    error_text = await resp.text()
                    logger.error(f"❌ Kalshi 连接失败: {resp.status} - {error_text}")
                    return False
        except Exception as e:
            logger.error(f"❌ Kalshi 连接异常: {e}")
            return False
    
    async def get_balance(self) -> Optional[Dict]:
        """获取账户余额
        
        Returns:
            Dict with keys:
            - balance: 可用余额（美分）
            - portfolio_value: 持仓价值（美分）
            - updated_ts: 更新时间戳
            返回 None 如果请求失败
        """
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            path = "/trade-api/v2/portfolio/balance"
            url = f"{self.base_url}/portfolio/balance"
            headers = self._get_headers("GET", path)
            
            async with self.session.get(url, headers=headers) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    logger.debug(f"✅ Kalshi 余额: ${data.get('balance', 0) / 100:.2f}")
                    return data
                else:
                    error_text = await resp.text()
                    logger.error(f"❌ 获取 Kalshi 余额失败: {resp.status} - {error_text}")
                    return None
        except Exception as e:
            logger.error(f"❌ 获取 Kalshi 余额异常: {e}")
            return None
    
    async def create_market_order(
        self,
        ticker: str,
        side: str,
        action: str,
        count: int = 1
    ) -> tuple[Optional[Dict], float]:
        """创建市价订单
        
        Args:
            ticker: 市场 ticker (如 KXNBAGAME-26JAN07CLELAL-CLE)
            side: "yes" 或 "no"
            action: "buy" 或 "sell"
            count: 合约数量（最小 1）
            
        Returns:
            (订单响应, 下单耗时ms)
            订单响应为 None 表示下单失败
        """
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            path = "/trade-api/v2/portfolio/orders"
            url = f"{self.base_url}/portfolio/orders"
            headers = self._get_headers("POST", path)
            
            # 市价订单：买入用高价（99），卖出用低价（1），确保能成交
            price = 99 if action == "buy" else 1
            
            payload = {
                "ticker": ticker,
                "side": side,
                "action": action,
                "count": count,
                "type": "market"
            }
            
            # 根据 side 设置对应的价格参数
            if side == "yes":
                payload["yes_price"] = price
            else:
                payload["no_price"] = price
            
            logger.info(f"📤 [Kalshi] 下单请求: {action} {count}x {side} @ {ticker}, price={price}¢")
            
            # 计时开始
            start_time = time.perf_counter()
            
            async with self.session.post(url, headers=headers, json=payload) as resp:
                # 计时结束
                elapsed_ms = (time.perf_counter() - start_time) * 1000
                
                if resp.status == 201:
                    data = await resp.json()
                    order = data.get("order", {})
                    order_id = order.get("order_id", "unknown")
                    status = order.get("status", "unknown")
                    fill_count = order.get("fill_count", 0)
                    
                    logger.info(f"✅ [Kalshi] 下单成功: order_id={order_id}, status={status}, "
                               f"fill_count={fill_count}, 耗时={elapsed_ms:.2f}ms")
                    return data, elapsed_ms
                else:
                    error_text = await resp.text()
                    logger.error(f"❌ [Kalshi] 下单失败: {resp.status} - {error_text}, 耗时={elapsed_ms:.2f}ms")
                    return None, elapsed_ms
                    
        except Exception as e:
            elapsed_ms = (time.perf_counter() - start_time) * 1000 if 'start_time' in locals() else 0
            logger.error(f"❌ [Kalshi] 下单异常: {e}, 耗时={elapsed_ms:.2f}ms")
            return None, elapsed_ms
    
    async def get_orders(self, status: str = None) -> Optional[List[Dict]]:
        """获取订单列表
        
        Args:
            status: 订单状态过滤 (resting, canceled, executed)
            
        Returns:
            订单列表，失败返回 None
        """
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            path = "/trade-api/v2/portfolio/orders"
            url = f"{self.base_url}/portfolio/orders"
            headers = self._get_headers("GET", path)
            
            params = {}
            if status:
                params["status"] = status
            
            async with self.session.get(url, headers=headers, params=params) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    orders = data.get("orders", [])
                    logger.debug(f"✅ [Kalshi] 获取 {len(orders)} 个订单")
                    return orders
                else:
                    error_text = await resp.text()
                    logger.error(f"❌ [Kalshi] 获取订单失败: {resp.status} - {error_text}")
                    return None
        except Exception as e:
            logger.error(f"❌ [Kalshi] 获取订单异常: {e}")
            return None
    
    async def get_positions(self) -> Optional[List[Dict]]:
        """获取持仓列表
        
        Returns:
            持仓列表，失败返回 None
        """
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            path = "/trade-api/v2/portfolio/positions"
            url = f"{self.base_url}/portfolio/positions"
            headers = self._get_headers("GET", path)
            
            async with self.session.get(url, headers=headers) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    positions = data.get("market_positions", [])
                    logger.debug(f"✅ [Kalshi] 获取 {len(positions)} 个持仓")
                    return positions
                else:
                    error_text = await resp.text()
                    logger.error(f"❌ [Kalshi] 获取持仓失败: {resp.status} - {error_text}")
                    return None
        except Exception as e:
            logger.error(f"❌ [Kalshi] 获取持仓异常: {e}")
            return None
    
    async def cancel_order(self, order_id: str) -> bool:
        """取消订单
        
        Args:
            order_id: 订单 ID
            
        Returns:
            是否成功取消
        """
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            path = f"/trade-api/v2/portfolio/orders/{order_id}"
            url = f"{self.base_url}/portfolio/orders/{order_id}"
            headers = self._get_headers("DELETE", path)
            
            async with self.session.delete(url, headers=headers) as resp:
                if resp.status == 200:
                    logger.info(f"✅ [Kalshi] 订单已取消: {order_id}")
                    return True
                else:
                    error_text = await resp.text()
                    logger.error(f"❌ [Kalshi] 取消订单失败: {resp.status} - {error_text}")
                    return False
        except Exception as e:
            logger.error(f"❌ [Kalshi] 取消订单异常: {e}")
            return False
    
    async def get_nba_events_and_markets(self) -> tuple[List[KalshiEvent], List[KalshiMarket]]:
        """获取 NBA 事件和市场"""
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            path = "/trade-api/v2/events"
            url = f"{self.base_url}/events"
            params = {
                "series_ticker": "KXNBAGAME",
                "status": "open",
                "limit": 200,
                "with_nested_markets": "true",
            }
            
            headers = self._get_headers("GET", path)
            
            async with self.session.get(url, params=params, headers=headers) as resp:
                if resp.status != 200:
                    error_text = await resp.text()
                    logger.error(f"获取 Kalshi NBA 事件失败: {resp.status} - {error_text}")
                    return [], []
                
                data = await resp.json()
                api_events = data.get("events", [])
            
            if not api_events:
                logger.warning("未找到任何 Kalshi NBA 事件")
                return [], []
            
            logger.info(f"📥 获取到 {len(api_events)} 个 Kalshi NBA 事件")
            
            events = []
            markets = []
            
            for api_event in api_events:
                event_ticker = api_event.get("event_ticker", "")
                event_markets = api_event.get("markets", [])
                
                if not event_markets:
                    continue
                
                # 从 event_ticker 提取球队和日期
                teams = self._extract_teams_from_ticker(event_ticker)
                game_date = self._extract_game_date(event_ticker)
                
                if not teams:
                    logger.warning(f"无法从 ticker 提取队伍: {event_ticker}")
                    continue
                
                team_a, team_b = teams
                # 按字母序排序
                if team_a > team_b:
                    team_a, team_b = team_b, team_a
                event_name = f"{team_a}-{team_b}"
                
                # 创建事件
                event = KalshiEvent(
                    event_id=event_ticker,
                    platform=Platform.KALSHI,
                    name=event_name,
                    team_a=team_a,
                    team_b=team_b,
                    start_time=game_date,
                    category="NBA",
                    markets=[]
                )
                
                # 处理该事件的市场
                for market_data in event_markets:
                    if market_data.get("status") != "active":
                        continue
                    
                    ticker = market_data.get("ticker", "")
                    
                    # 获取价格 - 使用 Ask 价格（买入价格）以与 WebSocket 保持一致
                    # 套利需要买入，所以必须使用 Ask 价格
                    yes_bid = market_data.get("yes_bid", 0)
                    yes_ask = market_data.get("yes_ask", 0)
                    # 优先使用 ask 价格，没有则用 bid，都没有用默认值
                    yes_price = (yes_ask / 100.0) if yes_ask else ((yes_bid / 100.0) if yes_bid else 0.5)
                    
                    no_bid = market_data.get("no_bid", 0)
                    no_ask = market_data.get("no_ask", 0)
                    no_price = (no_ask / 100.0) if no_ask else ((no_bid / 100.0) if no_bid else (1.0 - yes_price))
                    
                    # 从 ticker 提取预测的队伍
                    team_name = self._extract_team_from_ticker(ticker)
                    if not team_name:
                        continue
                    
                    # 确定对手队伍
                    opponent = team_b if team_name.upper() == team_a.upper() else team_a
                    
                    market = KalshiMarket(
                        market_id=ticker,
                        event_id=event_ticker,
                        event_name=event_name,
                        team_name=team_name.upper(),
                        opponent_name=opponent,
                        yes_price=yes_price,
                        no_price=no_price,
                        start_time=game_date,
                        volume=market_data.get("volume", 0),
                        liquidity=market_data.get("open_interest", 0)
                    )
                    markets.append(market)
                    event.markets.append(market)
                
                if event.markets:
                    events.append(event)
            
            logger.info(f"✅ Kalshi: {len(events)} 个事件, {len(markets)} 个市场")
            return events, markets
            
        except Exception as e:
            logger.error(f"❌ 获取 Kalshi 事件异常: {e}")
            import traceback
            traceback.print_exc()
            return [], []
    
    def _extract_teams_from_ticker(self, event_ticker: str) -> Optional[tuple]:
        """从 event_ticker 提取球队信息"""
        try:
            parts = event_ticker.split('-')
            if len(parts) < 2:
                return None
            
            last_part = parts[-1]
            if len(last_part) <= 7:
                return None
            
            teams_str = last_part[7:]
            
            if len(teams_str) == 6:
                return (teams_str[:3].upper(), teams_str[3:].upper())
            elif len(teams_str) == 7:
                return (teams_str[:3].upper(), teams_str[3:].upper())
            elif len(teams_str) >= 4:
                mid = len(teams_str) // 2
                return (teams_str[:mid].upper(), teams_str[mid:].upper())
            
            return None
        except:
            return None
    
    def _extract_team_from_ticker(self, ticker: str) -> Optional[str]:
        """从 ticker 提取预测队伍"""
        try:
            parts = ticker.split('-')
            if len(parts) < 3:
                return None
            return parts[-1].upper()
        except:
            return None
    
    def _extract_game_date(self, event_ticker: str) -> Optional[datetime]:
        """从 event_ticker 提取比赛日期"""
        try:
            parts = event_ticker.split('-')
            if len(parts) < 2:
                return None
            
            date_part = parts[1]
            if len(date_part) < 7:
                return None
            
            date_str = date_part[:7]
            year_str = date_str[:2]
            month_str = date_str[2:5]
            day_str = date_str[5:7]
            
            year = 2000 + int(year_str)
            
            month_map = {
                "JAN": 1, "FEB": 2, "MAR": 3, "APR": 4,
                "MAY": 5, "JUN": 6, "JUL": 7, "AUG": 8,
                "SEP": 9, "OCT": 10, "NOV": 11, "DEC": 12
            }
            month = month_map.get(month_str.upper())
            if not month:
                return None
            
            day = int(day_str)
            return datetime(year, month, day, 12, 0, 0)
        except:
            return None
    
    # ==================== WebSocket 部分 ====================
    
    async def connect_websocket(
        self,
        market_tickers: List[str],
        on_price_update: Callable[[PriceUpdate], None],
        on_log: Callable[[str], None] = None
    ):
        """连接 WebSocket 并订阅市场"""
        if not market_tickers:
            logger.warning("⚠️ Kalshi: 没有市场需要订阅")
            return
        
        def log(msg: str):
            logger.info(msg)
            if on_log:
                on_log(msg)
        
        log(f"🔌 [Kalshi] 开始连接 WebSocket，订阅 {len(market_tickers)} 个市场")
        
        retry_count = 0
        max_retries = 20
        retry_delay = 1
        
        while retry_count < max_retries:
            try:
                # 每次重连时重新生成认证签名（防止签名过期）
                timestamp_ms = int(time.time() * 1000)
                path = "/trade-api/ws/v2"
                signature = self._sign_request(timestamp_ms, "GET", path)
                
                headers = {
                    "KALSHI-ACCESS-KEY": self.config.api_key,
                    "KALSHI-ACCESS-SIGNATURE": signature,
                    "KALSHI-ACCESS-TIMESTAMP": str(timestamp_ms),
                }
                
                log(f"🔑 [Kalshi] 生成新的认证签名 (尝试 {retry_count + 1}/{max_retries})")
                
                async with websockets.connect(self.ws_url, extra_headers=headers) as ws:
                    self.ws_connection = ws
                    self.ws_connected = True
                    log("✅ [Kalshi] WebSocket 连接成功")
                    
                    # 订阅市场
                    for idx, ticker in enumerate(market_tickers):
                        subscribe_msg = {
                            "id": idx + 1,
                            "cmd": "subscribe",
                            "params": {
                                "channels": ["orderbook_delta"],
                                "market_ticker": ticker
                            }
                        }
                        await ws.send(json.dumps(subscribe_msg))
                    
                    log(f"✅ [Kalshi] 已订阅 {len(market_tickers)} 个市场")
                    
                    # 接收消息 - 使用 wait_for 支持取消
                    msg_count = 0
                    while True:
                        try:
                            message = await asyncio.wait_for(ws.recv(), timeout=30.0)
                            msg_count += 1
                            if msg_count % 100 == 0:
                                log(f"📊 [Kalshi] 已接收 {msg_count} 条消息")
                            
                            update = self._parse_ws_message(message)
                            if update:
                                on_price_update(update)
                        except asyncio.TimeoutError:
                            # 超时但继续等待
                            continue
                    
            except asyncio.CancelledError:
                log("🛑 [Kalshi] WebSocket 任务被取消")
                self.ws_connected = False
                raise  # 重新抛出以正确退出
            except websockets.ConnectionClosed as e:
                log(f"⚠️ [Kalshi] WebSocket 连接关闭: {e.code} - {e.reason}")
            except websockets.InvalidStatusCode as e:
                log(f"❌ [Kalshi] WebSocket 状态码错误: {e.status_code}")
                import traceback
                logger.error(f"详细错误:\n{traceback.format_exc()}")
            except Exception as e:
                log(f"❌ [Kalshi] WebSocket 错误: {type(e).__name__}: {e}")
                import traceback
                logger.error(f"详细错误:\n{traceback.format_exc()}")
            
            self.ws_connected = False
            retry_count += 1
            
            if retry_count < max_retries:
                log(f"🔄 [Kalshi] {retry_delay}s 后重连 (尝试 {retry_count}/{max_retries})")
                try:
                    await asyncio.sleep(retry_delay)
                except asyncio.CancelledError:
                    log("🛑 [Kalshi] 重连等待被取消")
                    raise
                retry_delay = min(retry_delay * 2, 60)
        
        log("⚠️ [Kalshi] 达到最大重试次数，停止重连")
    
    async def subscribe_markets(self, market_tickers: List[str], on_log: Callable[[str], None] = None) -> bool:
        """热订阅新的市场（不关闭连接）
        
        在现有 WebSocket 连接上发送订阅命令订阅新的市场
        
        Args:
            market_tickers: 要订阅的市场 ticker 列表
            on_log: 日志回调函数
            
        Returns:
            是否成功发送订阅请求
        """
        def log(msg: str):
            logger.info(msg)
            if on_log:
                on_log(msg)
        
        if not self.ws_connection or not self.ws_connected:
            log("⚠️ [Kalshi] WebSocket 未连接，无法热订阅")
            return False
        
        if not market_tickers:
            return True
        
        try:
            base_id = int(time.time() * 1000)
            for idx, ticker in enumerate(market_tickers):
                subscribe_msg = {
                    "id": base_id + idx,
                    "cmd": "subscribe",
                    "params": {
                        "channels": ["orderbook_delta"],
                        "market_ticker": ticker
                    }
                }
                await self.ws_connection.send(json.dumps(subscribe_msg))
            
            log(f"✅ [Kalshi] 热订阅 {len(market_tickers)} 个新市场")
            return True
        except Exception as e:
            log(f"❌ [Kalshi] 热订阅失败: {e}")
            return False
    
    _ws_msg_log_count = 0
    _ws_msg_file_count = 0  # 文件中已保存的消息数
    # 订单簿缓存: market_ticker -> {"yes": [[price, qty], ...], "no": [[price, qty], ...]}
    _orderbook_cache: Dict[str, dict] = {}
    
    def _apply_delta(self, ticker: str, side: str, price: int, delta: int):
        """应用增量更新到订单簿
        
        Args:
            ticker: 市场 ticker
            side: "yes" 或 "no"
            price: 价格（美分）
            delta: 数量变化（正数为增加，负数为减少）
        """
        if ticker not in KalshiClient._orderbook_cache:
            return
        
        book = KalshiClient._orderbook_cache[ticker][side]
        
        # 查找该价格是否存在
        found_idx = -1
        for i, entry in enumerate(book):
            if entry[0] == price:
                found_idx = i
                break
        
        if found_idx >= 0:
            # 更新现有价格的数量
            new_qty = book[found_idx][1] + delta
            if new_qty <= 0:
                # 数量为0或负数，删除该价格
                book.pop(found_idx)
            else:
                book[found_idx][1] = new_qty
        else:
            # 新价格，添加到订单簿（仅当 delta > 0）
            if delta > 0:
                book.append([price, delta])
                # 按价格排序
                book.sort(key=lambda x: x[0])
    
    def _get_best_bid(self, ticker: str, side: str) -> Optional[float]:
        """获取订单簿的最佳买价（最高价）"""
        if ticker not in KalshiClient._orderbook_cache:
            return None
        
        book = KalshiClient._orderbook_cache[ticker][side]
        if book and len(book) > 0:
            # 最高买价 = 列表最后一个（已按价格升序排列）
            return book[-1][0] / 100.0
        return None
    
    def _parse_ws_message(self, text: str) -> Optional[PriceUpdate]:
        """解析 WebSocket 消息
        
        支持两种消息类型:
        - orderbook_snapshot: 完整订单簿快照（包含 yes 和 no 数组）
        - orderbook_delta: 增量更新（单个 price/delta/side 更新）
        
        维护订单簿缓存以正确处理增量更新
        """
        try:
            data = json.loads(text)
            msg_type = data.get("type", "")
            
            # 记录前几条消息的原始格式
            KalshiClient._ws_msg_log_count += 1
            if KalshiClient._ws_msg_log_count <= 3:
                logger.info(f"🔍 [Kalshi] 原始消息 #{KalshiClient._ws_msg_log_count}: type={msg_type}")
                logger.info(f"   keys: {list(data.keys())}")
                if "msg" in data:
                    logger.info(f"   msg keys: {list(data['msg'].keys())}")
            
            if msg_type == "orderbook_snapshot":
                # 完整快照：替换整个订单簿
                msg = data.get("msg", {})
                market_ticker = msg.get("market_ticker", "")
                
                if not market_ticker:
                    return None
                
                yes_data = msg.get("yes", [])
                no_data = msg.get("no", [])
                
                # 初始化/替换订单簿缓存
                KalshiClient._orderbook_cache[market_ticker] = {
                    "yes": [list(entry) for entry in yes_data],  # 深拷贝
                    "no": [list(entry) for entry in no_data]     # 深拷贝
                }
                
                # 计算价格
                yes_bid = self._get_best_bid(market_ticker, "yes")
                no_bid = self._get_best_bid(market_ticker, "no")
                
                if KalshiClient._ws_msg_log_count <= 5:
                    logger.info(f"   [Snapshot] {market_ticker}: yes_bid={yes_bid}, no_bid={no_bid}")
                
                # 计算 Ask 价格
                if yes_bid is not None and no_bid is not None:
                    yes_ask = 1.0 - no_bid
                    no_ask = 1.0 - yes_bid
                    
                    return PriceUpdate(
                        platform=Platform.KALSHI,
                        market_id=market_ticker,
                        yes_bid=yes_bid,
                        yes_ask=yes_ask,
                        no_bid=no_bid,
                        no_ask=no_ask,
                        timestamp=datetime.now()
                    )
            
            elif msg_type == "orderbook_delta":
                # 增量更新：单个价格点的变化
                # 格式: {"price": 31, "delta": 125, "side": "yes", "market_ticker": "..."}
                msg = data.get("msg", {})
                market_ticker = msg.get("market_ticker", "")
                
                if not market_ticker:
                    return None
                
                # 如果没有该市场的快照，忽略 delta
                if market_ticker not in KalshiClient._orderbook_cache:
                    return None
                
                price = msg.get("price")
                delta = msg.get("delta")
                side = msg.get("side")
                
                if price is not None and delta is not None and side in ["yes", "no"]:
                    # 应用增量更新
                    self._apply_delta(market_ticker, side, price, delta)
                    
                    # 重新计算价格
                    yes_bid = self._get_best_bid(market_ticker, "yes")
                    no_bid = self._get_best_bid(market_ticker, "no")
                    
                    # 计算 Ask 价格
                    if yes_bid is not None and no_bid is not None:
                        yes_ask = 1.0 - no_bid
                        no_ask = 1.0 - yes_bid
                        
                        return PriceUpdate(
                            platform=Platform.KALSHI,
                            market_id=market_ticker,
                            yes_bid=yes_bid,
                            yes_ask=yes_ask,
                            no_bid=no_bid,
                            no_ask=no_ask,
                            timestamp=datetime.now()
                        )
                
        except Exception as e:
            logger.debug(f"解析 Kalshi WS 消息失败: {e}")
        
        return None
