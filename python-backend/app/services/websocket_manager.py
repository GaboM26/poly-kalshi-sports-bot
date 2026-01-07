"""WebSocket 管理器 - 管理双平台 WebSocket 连接和价格更新

关键改动:
- Polymarket 价格更新时，需要更新所有使用该 token 的 MatchedMarket
- 一个 Poly 市场的 token 可能被多个 Kalshi 市场引用（但实际上是同一队伍）
- 使用 SQLite 异步存储套利记录，存储与业务解耦
"""
import asyncio
import logging
from typing import List, Dict, Callable, Optional
from datetime import datetime
from dataclasses import dataclass
from app.core.models import (
    PriceUpdate, MatchedMarket, Platform, 
    ArbitrageOpportunity, KalshiMarket, PolymarketMarket
)
from app.clients.kalshi import KalshiClient
from app.clients.polymarket import PolymarketClient
from app.core.calculator import ArbitrageCalculator
from app.services.storage import ArbitrageStorage, get_storage

logger = logging.getLogger(__name__)

# 套利追踪阈值
ARBITRAGE_TRACKING_THRESHOLD = 3.0  # 3%


@dataclass
class ArbitrageTrackingRecord:
    """套利机会追踪记录（内存中的活跃追踪）"""
    event_name: str
    team_name: str
    kalshi_market_id: str
    polymarket_market_id: str
    start_time: datetime
    end_time: Optional[datetime] = None
    max_profit_margin: float = 0.0
    max_profit_time: Optional[datetime] = None
    
    def to_dict(self) -> dict:
        """转换为字典（用于API响应）"""
        return {
            "event_name": self.event_name,
            "team_name": self.team_name,
            "kalshi_market_id": self.kalshi_market_id,
            "polymarket_market_id": self.polymarket_market_id,
            "start_time": self.start_time.isoformat(),
            "end_time": self.end_time.isoformat() if self.end_time else None,
            "duration_seconds": (self.end_time - self.start_time).total_seconds() if self.end_time else None,
            "max_profit_margin": self.max_profit_margin,
            "max_profit_time": self.max_profit_time.isoformat() if self.max_profit_time else None
        }


class WebSocketManager:
    """WebSocket 管理器"""
    
    def __init__(
        self,
        kalshi_client: KalshiClient,
        polymarket_client: PolymarketClient,
        calculator: ArbitrageCalculator,
        on_opportunity: Callable[[ArbitrageOpportunity], None] = None,
        on_log: Callable[[str], None] = None,
        storage: ArbitrageStorage = None
    ):
        self.kalshi_client = kalshi_client
        self.polymarket_client = polymarket_client
        self.calculator = calculator
        self.on_opportunity = on_opportunity
        self.on_log = on_log
        
        # 存储服务（异步队列，不阻塞业务）
        self.storage = storage or get_storage()
        
        # 配对市场数据
        self.matched_markets: List[MatchedMarket] = []
        # subscription_id (ticker 或 token_id) -> [MatchedMarket]
        self.market_lookup: Dict[str, List[MatchedMarket]] = {}
        
        # Kalshi 价格缓存: market_id -> (yes_price, no_price)
        self.kalshi_prices: Dict[str, tuple] = {}
        
        # Polymarket 价格缓存: token_id -> price (该 token 对应队伍的价格)
        self.poly_token_prices: Dict[str, float] = {}
        
        # WebSocket 状态
        self.kalshi_connected = False
        self.polymarket_connected = False
        
        # 最新价格更新时间（用于计算延迟）
        self.kalshi_last_update_time: Optional[datetime] = None
        self.polymarket_last_update_time: Optional[datetime] = None
        
        # 套利机会
        self.opportunities: List[ArbitrageOpportunity] = []
        
        # 统计
        self.kalshi_update_count = 0
        self.polymarket_update_count = 0
        self.calculation_count = 0
        
        # 套利追踪（仅内存中的活跃追踪，历史记录在 SQLite 中）
        # key: "{event_name}_{team_name}" -> ArbitrageTrackingRecord
        self.active_tracking: Dict[str, ArbitrageTrackingRecord] = {}
    
    def log(self, msg: str):
        """记录日志"""
        logger.info(msg)
        if self.on_log:
            self.on_log(msg)
    
    def _is_market_ready(self, matched_market: MatchedMarket) -> bool:
        """检查单个市场对是否数据完整（两个平台都有实时价格）
        
        Polymarket 需要两个 token 的价格：自己的和对手的
        """
        k_id = matched_market.kalshi_market.market_id
        poly_market = matched_market.polymarket_market
        
        # 检查 Kalshi 是否有实时价格（4个值的格式：yes_bid, yes_ask, no_bid, no_ask）
        k_prices = self.kalshi_prices.get(k_id)
        has_kalshi = k_prices is not None and len(k_prices) == 4
        
        # 检查 Polymarket 两个 token 是否都有实时价格
        own_token = poly_market.get_token_for_team(matched_market.team_name)
        
        # 获取对手 token
        if matched_market.team_name.upper() == poly_market.team_a.upper():
            opponent_token = poly_market.token_id_b
        else:
            opponent_token = poly_market.token_id_a
        
        has_own_poly = own_token and own_token in self.poly_token_prices
        has_opponent_poly = opponent_token and opponent_token in self.poly_token_prices
        
        return has_kalshi and has_own_poly and has_opponent_poly
    
    def get_data_coverage(self) -> dict:
        """获取数据覆盖率统计
        
        Polymarket 需要两个 token 都有价格才算 ready
        """
        total = len(self.matched_markets)
        kalshi_ready = 0
        poly_ready = 0
        both_ready = 0
        
        for mm in self.matched_markets:
            k_id = mm.kalshi_market.market_id
            poly_market = mm.polymarket_market
            
            k_prices = self.kalshi_prices.get(k_id)
            has_kalshi = k_prices is not None and len(k_prices) == 4
            
            # 检查两个 Poly token
            own_token = poly_market.get_token_for_team(mm.team_name)
            if mm.team_name.upper() == poly_market.team_a.upper():
                opponent_token = poly_market.token_id_b
            else:
                opponent_token = poly_market.token_id_a
            
            has_own_poly = own_token and own_token in self.poly_token_prices
            has_opponent_poly = opponent_token and opponent_token in self.poly_token_prices
            has_poly = has_own_poly and has_opponent_poly
            
            if has_kalshi:
                kalshi_ready += 1
            if has_poly:
                poly_ready += 1
            if has_kalshi and has_poly:
                both_ready += 1
        
        # 计算延迟（毫秒）
        now = datetime.now()
        kalshi_latency_ms = None
        polymarket_latency_ms = None
        
        if self.kalshi_last_update_time:
            kalshi_latency_ms = int((now - self.kalshi_last_update_time).total_seconds() * 1000)
        
        if self.polymarket_last_update_time:
            polymarket_latency_ms = int((now - self.polymarket_last_update_time).total_seconds() * 1000)
        
        return {
            "total_markets": total,
            "kalshi_ready": kalshi_ready,
            "polymarket_ready": poly_ready,
            "both_ready": both_ready,
            "kalshi_coverage": f"{kalshi_ready}/{total}",
            "polymarket_coverage": f"{poly_ready}/{total}",
            "full_coverage": f"{both_ready}/{total}",
            "kalshi_connected": self.kalshi_connected,
            "polymarket_connected": self.polymarket_connected,
            "kalshi_latency_ms": kalshi_latency_ms,
            "polymarket_latency_ms": polymarket_latency_ms
        }
    
    def is_ready(self) -> bool:
        """检查是否有任何市场对数据完整"""
        for mm in self.matched_markets:
            if self._is_market_ready(mm):
                return True
        return False
    
    
    def _get_tracking_key(self, event_name: str, team_name: str) -> str:
        """生成追踪记录的唯一key"""
        return f"{event_name}_{team_name}"
    
    def _track_opportunity(self, opportunity: ArbitrageOpportunity):
        """追踪套利机会（利润超过阈值时开始，低于阈值时结束）
        
        使用异步存储服务，不阻塞业务逻辑
        """
        key = self._get_tracking_key(opportunity.event_name, opportunity.team_name)
        now = datetime.now()
        profit_margin = opportunity.profit_margin
        
        if profit_margin >= ARBITRAGE_TRACKING_THRESHOLD:
            # 利润超过阈值
            if key not in self.active_tracking:
                # 开始新的追踪（内存 + SQLite）
                record = ArbitrageTrackingRecord(
                    event_name=opportunity.event_name,
                    team_name=opportunity.team_name,
                    kalshi_market_id=opportunity.kalshi_market_id,
                    polymarket_market_id=opportunity.polymarket_market_id,
                    start_time=now,
                    max_profit_margin=profit_margin,
                    max_profit_time=now
                )
                self.active_tracking[key] = record
                self.log(f"🎯 开始追踪套利: {opportunity.event_name} {opportunity.team_name} 利润={profit_margin:.2f}%")
                
                # 异步存储到 SQLite（非阻塞）
                self.storage.track_start(
                    tracking_key=key,
                    event_name=opportunity.event_name,
                    team_name=opportunity.team_name,
                    kalshi_market_id=opportunity.kalshi_market_id,
                    polymarket_market_id=opportunity.polymarket_market_id,
                    profit_margin=profit_margin,
                    kalshi_price=opportunity.kalshi_price,
                    polymarket_price=opportunity.polymarket_price
                )
            else:
                # 更新现有追踪
                record = self.active_tracking[key]
                is_new_max = profit_margin > record.max_profit_margin
                
                if is_new_max:
                    record.max_profit_margin = profit_margin
                    record.max_profit_time = now
                    self.log(f"📈 套利新高: {opportunity.event_name} {opportunity.team_name} 最高利润={profit_margin:.2f}%")
                
                # 异步记录利润历史到 SQLite（非阻塞）
                self.storage.track_update(
                    tracking_key=key,
                    profit_margin=profit_margin,
                    is_new_max=is_new_max,
                    kalshi_price=opportunity.kalshi_price,
                    polymarket_price=opportunity.polymarket_price
                )
        else:
            # 利润低于阈值
            if key in self.active_tracking:
                # 结束追踪
                record = self.active_tracking[key]
                record.end_time = now
                duration = (now - record.start_time).total_seconds()
                
                self.log(f"⏹️ 套利结束: {record.event_name} {record.team_name}")
                self.log(f"   持续时间: {duration:.1f}秒, 最高利润: {record.max_profit_margin:.2f}%")
                
                # 从内存中移除
                del self.active_tracking[key]
                
                # 异步保存到 SQLite（非阻塞）
                self.storage.track_end(key)
    
    def _check_expired_tracking(self, current_opportunities: List[ArbitrageOpportunity]):
        """检查并结束不再存在的套利追踪"""
        current_keys = set()
        for opp in current_opportunities:
            if opp.profit_margin >= ARBITRAGE_TRACKING_THRESHOLD:
                current_keys.add(self._get_tracking_key(opp.event_name, opp.team_name))
        
        # 找出需要结束的追踪
        expired_keys = [key for key in self.active_tracking if key not in current_keys]
        
        now = datetime.now()
        for key in expired_keys:
            record = self.active_tracking[key]
            record.end_time = now
            duration = (now - record.start_time).total_seconds()
            
            self.log(f"⏹️ 套利结束(消失): {record.event_name} {record.team_name}")
            self.log(f"   持续时间: {duration:.1f}秒, 最高利润: {record.max_profit_margin:.2f}%")
            
            # 从内存中移除
            del self.active_tracking[key]
            
            # 异步保存到 SQLite（非阻塞）
            self.storage.track_end(key)
    
    def get_tracking_stats(self) -> dict:
        """获取追踪统计信息"""
        # 从 SQLite 获取统计信息
        storage_stats = self.storage.get_stats()
        
        # 从 SQLite 获取最近完成的记录
        recent_completed = self.storage.get_all_completed_with_history(limit=10)
        
        return {
            "active_count": len(self.active_tracking),
            "completed_count": storage_stats['completed_count'],
            "total_history_points": storage_stats['total_history_points'],
            "storage_queue_size": storage_stats['queue_size'],
            "active": [
                {
                    "event_name": r.event_name,
                    "team_name": r.team_name,
                    "start_time": r.start_time.isoformat(),
                    "duration_seconds": (datetime.now() - r.start_time).total_seconds(),
                    "max_profit_margin": r.max_profit_margin
                }
                for r in self.active_tracking.values()
            ],
            "recent_completed": recent_completed
        }
    
    def set_matched_markets(
        self,
        matched_markets: List[MatchedMarket],
        market_lookup: Dict[str, List[MatchedMarket]]
    ):
        """设置配对市场数据"""
        self.matched_markets = matched_markets
        self.market_lookup = market_lookup
        
        # 不再初始化价格缓存 - 等待 WebSocket 实时数据
        # 价格缓存保持为空，只有收到实时数据后才填充
        
        self.log(f"📊 已加载 {len(matched_markets)} 个配对市场 (等待实时价格)")
    
    async def add_subscriptions(
        self,
        new_matched_markets: List[MatchedMarket],
        new_kalshi_tickers: List[str],
        new_poly_token_ids: List[str],
        new_market_lookup: Dict[str, List[MatchedMarket]]
    ) -> bool:
        """动态添加新的订阅（热订阅）
        
        在现有 WebSocket 连接上订阅新发现的市场
        
        Args:
            new_matched_markets: 新的配对市场列表
            new_kalshi_tickers: 新的 Kalshi 市场 ticker 列表
            new_poly_token_ids: 新的 Polymarket token ID 列表
            new_market_lookup: 新的市场查找表
            
        Returns:
            是否成功添加订阅
        """
        if not new_matched_markets:
            return True
        
        self.log(f"🔄 动态添加 {len(new_matched_markets)} 个新配对市场")
        
        # 1. 更新内部数据结构
        old_count = len(self.matched_markets)
        self.matched_markets.extend(new_matched_markets)
        
        # 合并 market_lookup
        for key, markets in new_market_lookup.items():
            if key in self.market_lookup:
                # 避免重复添加
                existing_ids = {mm.kalshi_market.market_id for mm in self.market_lookup[key]}
                for mm in markets:
                    if mm.kalshi_market.market_id not in existing_ids:
                        self.market_lookup[key].append(mm)
            else:
                self.market_lookup[key] = markets
        
        self.log(f"📊 市场数据已更新: {old_count} → {len(self.matched_markets)} 个配对市场")
        
        success = True
        
        # 2. 热订阅 Kalshi
        if new_kalshi_tickers:
            self.log(f"🔌 开始热订阅 Kalshi {len(new_kalshi_tickers)} 个市场...")
            kalshi_success = await self.kalshi_client.subscribe_markets(
                new_kalshi_tickers,
                on_log=lambda msg: self.log(msg)
            )
            if not kalshi_success:
                self.log("⚠️ Kalshi 热订阅失败")
                success = False
        
        # 3. 热订阅 Polymarket
        if new_poly_token_ids:
            self.log(f"🔌 开始热订阅 Polymarket {len(new_poly_token_ids)} 个 token...")
            poly_success = await self.polymarket_client.subscribe_tokens(
                new_poly_token_ids,
                on_log=lambda msg: self.log(msg)
            )
            if not poly_success:
                self.log("⚠️ Polymarket 热订阅失败")
                success = False
        
        if success:
            self.log(f"✅ 动态订阅完成，当前共 {len(self.matched_markets)} 个配对市场")
        else:
            self.log(f"⚠️ 动态订阅部分失败，当前共 {len(self.matched_markets)} 个配对市场")
        
        return success
    
    async def start(
        self,
        kalshi_tickers: List[str],
        polymarket_token_ids: List[str]
    ):
        """启动 WebSocket 连接"""
        self.log("🚀 启动 WebSocket 连接...")
        
        self._tasks = []
        
        if kalshi_tickers:
            self._tasks.append(
                asyncio.create_task(
                    self._run_kalshi_ws(kalshi_tickers),
                    name="kalshi_ws"
                )
            )
        
        if polymarket_token_ids:
            self._tasks.append(
                asyncio.create_task(
                    self._run_polymarket_ws(polymarket_token_ids),
                    name="polymarket_ws"
                )
            )
        
        if self._tasks:
            try:
                await asyncio.gather(*self._tasks, return_exceptions=True)
            except asyncio.CancelledError:
                self.log("🛑 WebSocket 管理器被取消")
                # 取消所有子任务
                for task in self._tasks:
                    if not task.done():
                        task.cancel()
                # 等待任务完成
                await asyncio.gather(*self._tasks, return_exceptions=True)
                raise
    
    async def _run_kalshi_ws(self, tickers: List[str]):
        """运行 Kalshi WebSocket"""
        try:
            await self.kalshi_client.connect_websocket(
                market_tickers=tickers,
                on_price_update=self._on_kalshi_price_update,
                on_log=lambda msg: self.log(msg)
            )
        except Exception as e:
            self.log(f"❌ Kalshi WebSocket 异常: {e}")
        finally:
            self.kalshi_connected = False
    
    async def _run_polymarket_ws(self, token_ids: List[str]):
        """运行 Polymarket WebSocket"""
        try:
            await self.polymarket_client.connect_websocket(
                token_ids=token_ids,
                on_price_update=self._on_polymarket_price_update,
                on_log=lambda msg: self.log(msg)
            )
        except Exception as e:
            self.log(f"❌ Polymarket WebSocket 异常: {e}")
        finally:
            self.polymarket_connected = False
    
    def _on_kalshi_price_update(self, update: PriceUpdate):
        """处理 Kalshi 价格更新"""
        self.kalshi_update_count += 1
        
        # 更新最后接收时间
        self.kalshi_last_update_time = update.timestamp
        
        # 标记连接成功（收到第一条价格更新）
        if not self.kalshi_connected:
            self.kalshi_connected = True
            self.log("✅ [Kalshi] 开始接收实时价格数据")
        
        # 记录前几条价格更新（包含完整 bid/ask）
        if self.kalshi_update_count <= 3:
            logger.info(f"📈 [Kalshi] 价格更新 #{self.kalshi_update_count}: {update.market_id}")
            logger.info(f"   Yes: bid={update.yes_bid}, ask={update.yes_ask}")
            logger.info(f"   No:  bid={update.no_bid}, ask={update.no_ask}")
        
        # 每 100 次记录数据覆盖率
        if self.kalshi_update_count % 100 == 0:
            coverage = self.get_data_coverage()
            self.log(f"📊 [Kalshi] 消息 #{self.kalshi_update_count} | 数据覆盖: K={coverage['kalshi_coverage']}, P={coverage['polymarket_coverage']}, 完整={coverage['full_coverage']}")
        
        # 保存完整的 bid/ask 价格 (yes_bid, yes_ask, no_bid, no_ask)
        if update.yes_ask is not None and update.no_ask is not None:
            self.kalshi_prices[update.market_id] = (
                update.yes_bid,
                update.yes_ask,  # 买入 Yes 的价格
                update.no_bid,
                update.no_ask    # 买入 No 的价格
            )
            
            # 找到对应的配对市场并计算套利
            if update.market_id in self.market_lookup:
                if self.kalshi_update_count <= 3:
                    logger.info(f"   找到 {len(self.market_lookup[update.market_id])} 个配对市场")
                for mm in self.market_lookup[update.market_id]:
                    self._calculate_and_notify(mm)
            elif self.kalshi_update_count <= 5:
                # 仅记录前几条未匹配的情况
                logger.warning(f"⚠️ [Kalshi] market_id 未在 lookup 中找到: {update.market_id}")
    
    def _on_polymarket_price_update(self, update: PriceUpdate):
        """处理 Polymarket 价格更新
        
        Poly 的 WebSocket 返回的是 token_id 和该 token 的 Ask 价格
        
        重要: 每个 MatchedMarket 有两个相关 token:
        - 自己的 token: Ask 价格 = poly_yes_price (买入该队获胜)
        - 对手的 token: Ask 价格 = poly_no_price (买入该队不获胜 = 对手获胜)
        
        不能用 1 - yes_ask 来计算 no_price，因为 1 - ask ≠ 对手的 ask
        """
        self.polymarket_update_count += 1
        
        # 更新最后接收时间
        self.polymarket_last_update_time = update.timestamp
        
        # 标记连接成功（收到第一条价格更新）
        if not self.polymarket_connected:
            self.polymarket_connected = True
            self.log("✅ [Polymarket] 开始接收实时价格数据")
        
        # 每 100 次记录数据覆盖率
        if self.polymarket_update_count % 100 == 0:
            coverage = self.get_data_coverage()
            self.log(f"📊 [Polymarket] 消息 #{self.polymarket_update_count} | 数据覆盖: K={coverage['kalshi_coverage']}, P={coverage['polymarket_coverage']}, 完整={coverage['full_coverage']}")
        
        # 更新 token 价格缓存 - 使用 Ask 价格（买入价格）
        # 套利需要买入，所以必须使用 Ask 价格
        yes_price = update.yes_ask if update.yes_ask else update.yes_bid
        
        if yes_price is not None:
            self.poly_token_prices[update.market_id] = yes_price
            
            # 找到使用该 token 的所有配对市场
            if update.market_id in self.market_lookup:
                for mm in self.market_lookup[update.market_id]:
                    # 判断这个 token 是自己的还是对手的
                    own_token = mm.polymarket_market.get_token_for_team(mm.team_name)
                    
                    if update.market_id == own_token:
                        # 自己的 token → 更新 yes_price
                        mm.poly_yes_price = yes_price
                    else:
                        # 对手的 token → 更新 no_price (对手的 yes = 自己的 no)
                        mm.poly_no_price = yes_price
                    
                    self._calculate_and_notify(mm)
            elif self.polymarket_update_count <= 5:
                # 仅记录前几条未匹配的情况
                logger.warning(f"⚠️ [Poly] token_id 未在 lookup 中找到: {update.market_id}")
    
    def _calculate_and_notify(self, matched_market: MatchedMarket):
        """计算套利并通知
        
        使用 Ask 价格计算套利（Ask = 买入价格）
        套利需要在两个平台分别买入，所以必须使用 Ask 价格
        只有当该市场对的两个平台都有实时数据时才计算
        """
        # 只有当这个市场对的数据完整时才计算
        if not self._is_market_ready(matched_market):
            return
        
        self.calculation_count += 1
        
        # 获取最新 Kalshi 价格 (yes_bid, yes_ask, no_bid, no_ask)
        k_id = matched_market.kalshi_market.market_id
        k_prices = self.kalshi_prices.get(k_id)
        
        # 使用实时 Ask 价格（已确认有数据）
        k_yes_ask = k_prices[1]  # yes_ask - 买入 Yes 的价格
        k_no_ask = k_prices[3]   # no_ask - 买入 No 的价格
        
        # 获取实时 Polymarket 价格 - 需要两个 token 的价格
        poly_market = matched_market.polymarket_market
        own_token = poly_market.get_token_for_team(matched_market.team_name)
        
        # 获取对手 token
        if matched_market.team_name.upper() == poly_market.team_a.upper():
            opponent_token = poly_market.token_id_b
        else:
            opponent_token = poly_market.token_id_a
        
        # 使用缓存中的 MatchedMarket 价格（已由 _on_polymarket_price_update 更新）
        p_yes = matched_market.poly_yes_price
        p_no = matched_market.poly_no_price
        
        # 更新 MatchedMarket 中的 Kalshi 价格（用于其他地方引用）
        matched_market.kalshi_market.yes_price = k_yes_ask
        matched_market.kalshi_market.no_price = k_no_ask
        
        # 记录前几次计算
        if self.calculation_count <= 3:
            logger.info(f"🔢 计算 #{self.calculation_count}: {matched_market.event_name} {matched_market.team_name}")
            logger.info(f"   K Ask: yes={k_yes_ask:.3f}, no={k_no_ask:.3f}")
            logger.info(f"   P:     yes={p_yes:.3f}, no={p_no:.3f}")
        
        # 计算套利（使用 Ask 价格 = 买入价格）
        opportunity = self.calculator.calculate_single(
            event_name=matched_market.event_name,
            team_name=matched_market.team_name,
            kalshi_market=matched_market.kalshi_market,
            kalshi_yes_price=k_yes_ask,
            kalshi_no_price=k_no_ask,
            polymarket_market=matched_market.polymarket_market,
            polymarket_yes_price=p_yes,
            polymarket_no_price=p_no
        )
        
        if self.calculation_count <= 3:
            if opportunity:
                logger.info(f"   ✅ 有套利机会! 利润={opportunity.profit_margin:.2f}%")
            else:
                logger.info(f"   ❌ 无套利机会")
        
        # 记录计算结果统计
        if not hasattr(self, '_opportunity_count'):
            self._opportunity_count = 0
            self._no_opportunity_count = 0
        
        if opportunity:
            self._opportunity_count += 1
            if self.on_opportunity:
                self.on_opportunity(opportunity)
            # 追踪套利机会
            self._track_opportunity(opportunity)
        else:
            self._no_opportunity_count += 1
            # 检查是否有需要结束的追踪（当前市场无套利机会时）
            key = self._get_tracking_key(matched_market.event_name, matched_market.team_name)
            if key in self.active_tracking:
                # 利润降到0以下，结束追踪
                record = self.active_tracking[key]
                record.end_time = datetime.now()
                duration = (record.end_time - record.start_time).total_seconds()
                self.log(f"⏹️ 套利结束(无机会): {record.event_name} {record.team_name}")
                self.log(f"   持续时间: {duration:.1f}秒, 最高利润: {record.max_profit_margin:.2f}%")
                
                # 从内存中移除
                del self.active_tracking[key]
                
                # 异步保存到 SQLite（非阻塞）
                self.storage.track_end(key)
        
        # 每 100 次计算记录统计
        total = self._opportunity_count + self._no_opportunity_count
        if total % 100 == 0:
            logger.info(f"💰 套利计算统计: {self._opportunity_count} 有机会 / {self._no_opportunity_count} 无机会 ({total} 次计算)")
            if self.active_tracking:
                logger.info(f"📊 当前追踪中: {len(self.active_tracking)} 个套利机会")
    
    def calculate_all(self) -> List[ArbitrageOpportunity]:
        """计算所有数据完整的市场对的套利机会
        
        使用实时 Ask 价格计算（与 _calculate_and_notify 保持一致）
        只计算两个平台都有实时数据的市场对
        
        Polymarket 价格使用 MatchedMarket 中缓存的价格
        （由 _on_polymarket_price_update 分别更新 yes 和 no）
        """
        opportunities = []
        
        for mm in self.matched_markets:
            # 只计算数据完整的市场对
            if not self._is_market_ready(mm):
                continue
            
            k_id = mm.kalshi_market.market_id
            k_prices = self.kalshi_prices.get(k_id)
            
            # 使用实时 Kalshi Ask 价格
            k_yes_price = k_prices[1]  # yes_ask
            k_no_price = k_prices[3]   # no_ask
            
            # 使用 MatchedMarket 中缓存的 Polymarket 价格
            # 这些价格已由 _on_polymarket_price_update 正确更新
            p_yes_price = mm.poly_yes_price
            p_no_price = mm.poly_no_price
            
            opportunity = self.calculator.calculate_single(
                event_name=mm.event_name,
                team_name=mm.team_name,
                kalshi_market=mm.kalshi_market,
                kalshi_yes_price=k_yes_price,
                kalshi_no_price=k_no_price,
                polymarket_market=mm.polymarket_market,
                polymarket_yes_price=p_yes_price,
                polymarket_no_price=p_no_price
            )
            
            if opportunity:
                opportunities.append(opportunity)
                # 追踪高利润套利机会
                self._track_opportunity(opportunity)
        
        # 按利润率排序
        opportunities.sort(key=lambda x: x.profit_margin, reverse=True)
        self.opportunities = opportunities
        
        # 检查是否有需要结束的追踪
        self._check_expired_tracking(opportunities)
        
        return opportunities
    
    def get_stats(self) -> dict:
        """获取统计信息"""
        coverage = self.get_data_coverage()
        storage_stats = self.storage.get_stats()
        
        return {
            "matched_markets": len(self.matched_markets),
            "kalshi_connected": self.kalshi_connected,
            "polymarket_connected": self.polymarket_connected,
            "kalshi_updates": self.kalshi_update_count,
            "polymarket_updates": self.polymarket_update_count,
            "calculations": self.calculation_count,
            "opportunities": len(self.opportunities),
            "active_tracking": len(self.active_tracking),
            "completed_tracking": storage_stats['completed_count'],
            "total_history_points": storage_stats['total_history_points'],
            "storage_queue_size": storage_stats['queue_size'],
            # 数据覆盖率
            "data_coverage": coverage
        }
