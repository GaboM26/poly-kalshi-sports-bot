"""Polymarket API 客户端

关键改动: 不拆分市场！
- 一个 Poly 事件只有一个市场
- 市场包含两个 outcomes (两个队伍)
- 在匹配时根据队伍名称确定使用哪个价格
"""
import aiohttp
import asyncio
import websockets
import json
import logging
from typing import List, Optional, Dict, Callable
from datetime import datetime
from models import PolymarketMarket, PolymarketEvent, Platform, PriceUpdate
from config import PolymarketConfig

logger = logging.getLogger(__name__)


class PolymarketClient:
    """Polymarket API 客户端"""
    
    def __init__(self, config: PolymarketConfig):
        self.config = config
        self.base_url = config.base_url
        self.clob_url = config.clob_url
        self.ws_url = "wss://ws-subscriptions-clob.polymarket.com/ws/market"
        self.session: Optional[aiohttp.ClientSession] = None
        self.ws_connection = None
        self.ws_connected = False
        
        # NBA 球队映射 (全名/别名 -> 标准缩写)
        self.team_mappings = {
            # 东部
            "ATLANTA HAWKS": "ATL", "HAWKS": "ATL", "ATL": "ATL",
            "BOSTON CELTICS": "BOS", "CELTICS": "BOS", "BOS": "BOS",
            "BROOKLYN NETS": "BKN", "NETS": "BKN", "BKN": "BKN", "BRK": "BKN",
            "CHARLOTTE HORNETS": "CHA", "HORNETS": "CHA", "CHA": "CHA", "CHO": "CHA",
            "CHICAGO BULLS": "CHI", "BULLS": "CHI", "CHI": "CHI",
            "CLEVELAND CAVALIERS": "CLE", "CAVALIERS": "CLE", "CAVS": "CLE", "CLE": "CLE",
            "DETROIT PISTONS": "DET", "PISTONS": "DET", "DET": "DET",
            "INDIANA PACERS": "IND", "PACERS": "IND", "IND": "IND",
            "MIAMI HEAT": "MIA", "HEAT": "MIA", "MIA": "MIA",
            "MILWAUKEE BUCKS": "MIL", "BUCKS": "MIL", "MIL": "MIL",
            "NEW YORK KNICKS": "NYK", "KNICKS": "NYK", "NYK": "NYK", "NY": "NYK",
            "ORLANDO MAGIC": "ORL", "MAGIC": "ORL", "ORL": "ORL",
            "PHILADELPHIA 76ERS": "PHI", "76ERS": "PHI", "SIXERS": "PHI", "PHI": "PHI",
            "TORONTO RAPTORS": "TOR", "RAPTORS": "TOR", "TOR": "TOR",
            "WASHINGTON WIZARDS": "WAS", "WIZARDS": "WAS", "WAS": "WAS", "WSH": "WAS",
            
            # 西部
            "DALLAS MAVERICKS": "DAL", "MAVERICKS": "DAL", "MAVS": "DAL", "DAL": "DAL",
            "DENVER NUGGETS": "DEN", "NUGGETS": "DEN", "DEN": "DEN",
            "GOLDEN STATE WARRIORS": "GSW", "WARRIORS": "GSW", "GSW": "GSW", "GS": "GSW",
            "HOUSTON ROCKETS": "HOU", "ROCKETS": "HOU", "HOU": "HOU",
            "LOS ANGELES CLIPPERS": "LAC", "CLIPPERS": "LAC", "LAC": "LAC", "LA CLIPPERS": "LAC",
            "LOS ANGELES LAKERS": "LAL", "LAKERS": "LAL", "LAL": "LAL", "LA LAKERS": "LAL",
            "MEMPHIS GRIZZLIES": "MEM", "GRIZZLIES": "MEM", "MEM": "MEM",
            "MINNESOTA TIMBERWOLVES": "MIN", "TIMBERWOLVES": "MIN", "WOLVES": "MIN", "MIN": "MIN",
            "NEW ORLEANS PELICANS": "NOP", "PELICANS": "NOP", "NOP": "NOP", "NO": "NOP",
            "OKLAHOMA CITY THUNDER": "OKC", "THUNDER": "OKC", "OKC": "OKC",
            "PHOENIX SUNS": "PHX", "SUNS": "PHX", "PHX": "PHX", "PHO": "PHX",
            "PORTLAND TRAIL BLAZERS": "POR", "TRAIL BLAZERS": "POR", "BLAZERS": "POR", "POR": "POR",
            "SACRAMENTO KINGS": "SAC", "KINGS": "SAC", "SAC": "SAC",
            "SAN ANTONIO SPURS": "SAS", "SPURS": "SAS", "SAS": "SAS",
            "UTAH JAZZ": "UTA", "JAZZ": "UTA", "UTA": "UTA",
        }
        
    async def __aenter__(self):
        self.session = aiohttp.ClientSession()
        return self
        
    async def __aexit__(self, exc_type, exc_val, exc_tb):
        if self.session:
            await self.session.close()
    
    def normalize_team_name(self, team_name: str) -> str:
        """标准化球队名称为缩写"""
        team_upper = team_name.strip().upper()
        return self.team_mappings.get(team_upper, team_upper)
    
    async def get_nba_events_and_markets(self) -> tuple[List[PolymarketEvent], List[PolymarketMarket]]:
        """获取 NBA 事件和市场
        
        重要: 不拆分市场！一个事件对应一个市场
        """
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            # 1. 获取体育联赛
            sports_url = f"{self.base_url}/sports"
            async with self.session.get(sports_url) as resp:
                if resp.status != 200:
                    logger.error(f"获取体育联赛失败: {resp.status}")
                    return [], []
                sports = await resp.json()
            
            # 2. 找到 NBA 联赛
            nba_leagues = [
                s for s in sports 
                if 'NBA' in s.get('sport', '').upper() 
                and 'WNBA' not in s.get('sport', '').upper()
            ]
            
            if not nba_leagues:
                logger.warning("未找到 NBA 联赛")
                return [], []
            
            events = []
            markets = []
            
            # 3. 获取 NBA 事件
            for league in nba_leagues[:1]:
                series_id = league.get('series')
                
                events_url = f"{self.base_url}/events"
                params = {
                    'series_id': str(series_id),
                    'tag_id': '100639',
                    'active': 'true',
                    'closed': 'false',
                    'limit': '100'
                }
                
                async with self.session.get(events_url, params=params) as resp:
                    if resp.status != 200:
                        continue
                    api_events = await resp.json()
                
                logger.info(f"📥 获取到 {len(api_events)} 个 Polymarket NBA 事件")
                
                # 4. 处理每个事件（不拆分！）
                for api_event in api_events:
                    event_title = api_event.get('title', '')
                    event_markets = api_event.get('markets', [])
                    event_slug = api_event.get('slug', '')
                    
                    # 从 slug 提取日期
                    event_date = self._extract_date_from_slug(event_slug)
                    
                    for market_data in event_markets:
                        parsed = self._parse_market(market_data, event_title, event_date)
                        if parsed:
                            event, market = parsed
                            events.append(event)
                            markets.append(market)
            
            logger.info(f"✅ Polymarket: {len(events)} 个事件, {len(markets)} 个市场")
            return events, markets
            
        except Exception as e:
            logger.error(f"❌ 获取 Polymarket 事件异常: {e}")
            import traceback
            traceback.print_exc()
            return [], []
    
    def _extract_date_from_slug(self, slug: str) -> Optional[datetime]:
        """从 slug 提取日期"""
        try:
            parts = slug.split('-')
            if len(parts) >= 3:
                year_str = parts[-3]
                month_str = parts[-2]
                day_str = parts[-1]
                
                year = int(year_str)
                month = int(month_str)
                day = int(day_str)
                
                return datetime(year, month, day, 12, 0, 0)
        except:
            pass
        return None
    
    def _parse_market(
        self, 
        market_data: dict, 
        event_title: str,
        event_date: Optional[datetime]
    ) -> Optional[tuple[PolymarketEvent, PolymarketMarket]]:
        """解析单个市场 - 不拆分！
        
        返回: (事件, 市场) 或 None
        """
        market_id = market_data.get('id')
        condition_id = market_data.get('conditionId', market_id)
        
        if not market_id:
            return None
        
        question = market_data.get('question', event_title)
        
        # 获取 outcomes 和价格
        outcomes_str = market_data.get('outcomes')
        prices_str = market_data.get('outcomePrices')
        
        if not outcomes_str or not prices_str:
            return None
        
        try:
            outcomes = json.loads(outcomes_str)
            prices = json.loads(prices_str)
        except:
            return None
        
        # 必须是二元市场
        if len(outcomes) != 2 or len(prices) != 2:
            return None
        
        try:
            price1 = float(prices[0])
            price2 = float(prices[1])
        except:
            return None
        
        # 验证价格有效性
        if price1 < 0 or price1 > 1 or price2 < 0 or price2 > 1:
            return None
        
        # 过滤无效价格
        if (price1 == 0 and price2 == 1) or (price1 == 1 and price2 == 0):
            return None
        
        # 过滤极端价格
        if price1 < 0.01 or price2 < 0.01 or price1 > 0.99 or price2 > 0.99:
            return None
        
        # 过滤 Yes/No 格式
        if any(o.lower() == "yes" for o in outcomes) and any(o.lower() == "no" for o in outcomes):
            return None
        
        # 只保留全场输赢市场
        if question != event_title:
            return None
        
        # 排除 Over/Under 市场
        if outcomes[0].lower() in ["over", "under"]:
            return None
        
        # 标准化球队名称
        team1_abbr = self.normalize_team_name(outcomes[0])
        team2_abbr = self.normalize_team_name(outcomes[1])
        
        # 按字母序排序（与 Kalshi 保持一致）
        if team1_abbr > team2_abbr:
            team_a, team_b = team2_abbr, team1_abbr
            price_a, price_b = price2, price1
            # token 也要交换
            token_index_a, token_index_b = 1, 0
        else:
            team_a, team_b = team1_abbr, team2_abbr
            price_a, price_b = price1, price2
            token_index_a, token_index_b = 0, 1
        
        # 构建标准化事件名
        event_name = f"{team_a}-{team_b}"
        
        # 获取交易量
        volume = 0.0
        try:
            volume_str = market_data.get('volume')
            if volume_str:
                volume = float(volume_str)
        except:
            pass
        
        # 获取 token IDs（用于 WebSocket 订阅）
        tokens = market_data.get('clobTokenIds')
        token_ids = []
        if tokens:
            try:
                token_ids = json.loads(tokens)
            except:
                pass
        
        # 创建市场（不拆分！）
        market = PolymarketMarket(
            market_id=condition_id,
            event_name=event_name,
            team_a=team_a,
            team_b=team_b,
            price_a=price_a,  # team_a 获胜价格
            price_b=price_b,  # team_b 获胜价格
            start_time=event_date,
            volume=volume,
            token_id_a=token_ids[token_index_a] if len(token_ids) > token_index_a else None,
            token_id_b=token_ids[token_index_b] if len(token_ids) > token_index_b else None
        )
        
        # 创建事件
        event = PolymarketEvent(
            event_id=condition_id,
            platform=Platform.POLYMARKET,
            name=event_name,
            team_a=team_a,
            team_b=team_b,
            start_time=event_date,
            category="NBA",
            market=market  # 一个事件一个市场
        )
        
        return event, market
    
    # ==================== WebSocket 部分 ====================
    
    async def connect_websocket(
        self,
        token_ids: List[str],
        on_price_update: Callable[[PriceUpdate], None],
        on_log: Callable[[str], None] = None
    ):
        """连接 WebSocket 并订阅市场"""
        if not token_ids:
            logger.warning("⚠️ Polymarket: 没有市场需要订阅")
            return
        
        def log(msg: str):
            logger.info(msg)
            if on_log:
                on_log(msg)
        
        # 去重
        unique_ids = list(set(token_ids))
        
        # 限制订阅数量
        max_subscriptions = 40
        limited_ids = unique_ids[:max_subscriptions]
        
        if len(unique_ids) > max_subscriptions:
            log(f"⚠️ [Polymarket] 订阅数限制为 {max_subscriptions} 个")
        
        log(f"🔌 [Polymarket] 开始连接 WebSocket，订阅 {len(limited_ids)} 个 token")
        
        retry_count = 0
        max_retries = 20
        retry_delay = 5
        
        while retry_count < max_retries:
            try:
                async with websockets.connect(self.ws_url) as ws:
                    self.ws_connection = ws
                    self.ws_connected = True
                    log("✅ [Polymarket] WebSocket 连接成功")
                    
                    # 等待连接确认
                    try:
                        msg = await asyncio.wait_for(ws.recv(), timeout=10)
                        data = json.loads(msg)
                        if data.get("event_type") == "connected":
                            log("✅ [Polymarket] 连接已确认")
                    except asyncio.TimeoutError:
                        log("⚠️ [Polymarket] 未收到连接确认，继续...")
                    
                    # 订阅市场
                    subscribe_msg = {
                        "assets_ids": limited_ids,
                        "type": "market"
                    }
                    await ws.send(json.dumps(subscribe_msg))
                    log(f"✅ [Polymarket] 订阅请求已发送")
                    
                    # 接收消息 - 使用 wait_for 支持取消
                    msg_count = 0
                    while True:
                        try:
                            message = await asyncio.wait_for(ws.recv(), timeout=30.0)
                            msg_count += 1
                            if msg_count % 100 == 0:
                                log(f"📊 [Polymarket] 已接收 {msg_count} 条消息")
                            
                            update = self._parse_ws_message(message)
                            if update:
                                on_price_update(update)
                        except asyncio.TimeoutError:
                            # 超时但继续等待
                            continue
                    
            except asyncio.CancelledError:
                log("🛑 [Polymarket] WebSocket 任务被取消")
                self.ws_connected = False
                raise  # 重新抛出以正确退出
            except websockets.ConnectionClosed as e:
                log(f"⚠️ [Polymarket] WebSocket 连接关闭: {e}")
            except Exception as e:
                log(f"❌ [Polymarket] WebSocket 错误: {e}")
            
            self.ws_connected = False
            retry_count += 1
            
            if retry_count < max_retries:
                log(f"🔄 [Polymarket] {retry_delay}s 后重连 (尝试 {retry_count}/{max_retries})")
                try:
                    await asyncio.sleep(retry_delay)
                except asyncio.CancelledError:
                    log("🛑 [Polymarket] 重连等待被取消")
                    raise
                retry_delay = min(retry_delay + 5, 120)
        
        log("⚠️ [Polymarket] 达到最大重试次数，停止重连")
    
    def _parse_ws_message(self, text: str) -> Optional[PriceUpdate]:
        """解析 WebSocket 消息"""
        try:
            data = json.loads(text)
            event_type = data.get("event_type", "")
            
            if event_type in ["book", "price_change"]:
                # token_id 作为 market_id
                market_id = data.get("asset_id") or data.get("market", "")
                
                if not market_id:
                    return None
                
                # 解析价格
                bids = data.get("bids", [])
                asks = data.get("asks", [])
                
                yes_bid = None
                yes_ask = None
                
                if bids and len(bids) > 0:
                    try:
                        yes_bid = float(bids[0].get("price", 0))
                    except:
                        pass
                
                if asks and len(asks) > 0:
                    try:
                        yes_ask = float(asks[0].get("price", 0))
                    except:
                        pass
                
                return PriceUpdate(
                    platform=Platform.POLYMARKET,
                    market_id=market_id,  # 这是 token_id
                    yes_bid=yes_bid,
                    yes_ask=yes_ask,
                    no_bid=None,
                    no_ask=None,
                    timestamp=datetime.now()
                )
        except Exception as e:
            logger.debug(f"解析 Polymarket WS 消息失败: {e}")
        
        return None
