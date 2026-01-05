"""套利服务 - 核心业务逻辑封装"""
import logging
from typing import List, Tuple, Optional
from datetime import datetime

from app.core.config import Config
from app.core.models import (
    KalshiEvent, KalshiMarket,
    PolymarketEvent, PolymarketMarket,
    MatchedEvent, MatchedMarket,
    ArbitrageOpportunity,
    SystemStats
)
from app.core.matcher import EventMatcher
from app.core.calculator import ArbitrageCalculator
from app.clients.kalshi import KalshiClient
from app.clients.polymarket import PolymarketClient

logger = logging.getLogger(__name__)


class ArbitrageService:
    """套利服务 - 封装核心业务逻辑"""
    
    def __init__(self, config: Config):
        """初始化服务
        
        Args:
            config: 配置对象
        """
        self.config = config
        self.kalshi_client = KalshiClient(config.kalshi)
        self.polymarket_client = PolymarketClient(config.polymarket)
        self.matcher = EventMatcher()
        self.calculator = ArbitrageCalculator(
            min_profit_margin=config.settings.min_profit_margin,
            default_bet_amount=config.settings.default_bet_amount
        )
        
        # 数据存储
        self.kalshi_events: List[KalshiEvent] = []
        self.kalshi_markets: List[KalshiMarket] = []
        self.polymarket_events: List[PolymarketEvent] = []
        self.polymarket_markets: List[PolymarketMarket] = []
        self.matched_events: List[MatchedEvent] = []
        self.matched_markets: List[MatchedMarket] = []
        
        # 统计信息
        self.stats = SystemStats()
    
    async def initialize(self) -> bool:
        """初始化服务
        
        Returns:
            是否初始化成功
        """
        logger.info("=" * 60)
        logger.info("🚀 初始化套利服务")
        logger.info("=" * 60)
        
        # 登录 Kalshi
        logger.info("🔐 正在连接 Kalshi...")
        success = await self.kalshi_client.login()
        if not success:
            logger.error("❌ Kalshi 连接失败")
            return False
        
        logger.info("✅ Kalshi 连接成功")
        return True
    
    async def fetch_market_data(self) -> Tuple[int, int, int, int]:
        """获取市场数据
        
        Returns:
            (kalshi_events, kalshi_markets, poly_events, poly_markets) 数量元组
        """
        logger.info("=" * 60)
        logger.info("📥 开始获取市场数据")
        logger.info("=" * 60)
        
        # 获取 Kalshi 数据
        self.kalshi_events, self.kalshi_markets = await self.kalshi_client.get_nba_events_and_markets()
        
        # 获取 Polymarket 数据
        self.polymarket_events, self.polymarket_markets = await self.polymarket_client.get_nba_events_and_markets()
        
        # 更新统计
        self.stats.total_kalshi_events = len(self.kalshi_events)
        self.stats.total_kalshi_markets = len(self.kalshi_markets)
        self.stats.total_polymarket_events = len(self.polymarket_events)
        self.stats.total_polymarket_markets = len(self.polymarket_markets)
        
        logger.info(f"   Kalshi: {self.stats.total_kalshi_events} 事件, {self.stats.total_kalshi_markets} 市场")
        logger.info(f"   Polymarket: {self.stats.total_polymarket_events} 事件, {self.stats.total_polymarket_markets} 市场")
        
        return (
            self.stats.total_kalshi_events,
            self.stats.total_kalshi_markets,
            self.stats.total_polymarket_events,
            self.stats.total_polymarket_markets
        )
    
    async def match_markets(self) -> Tuple[int, int]:
        """匹配市场
        
        Returns:
            (matched_events, matched_markets) 数量元组
        """
        logger.info("=" * 60)
        logger.info("🔍 开始匹配市场")
        logger.info("=" * 60)
        
        # 执行两阶段匹配
        self.matched_events, self.matched_markets = self.matcher.match_events_and_markets(
            self.kalshi_events, self.kalshi_markets,
            self.polymarket_events, self.polymarket_markets
        )
        
        # 更新统计
        self.stats.matched_events = len(self.matched_events)
        self.stats.matched_markets = len(self.matched_markets)
        
        logger.info(f"   匹配: {self.stats.matched_events} 事件, {self.stats.matched_markets} 市场对")
        
        return (self.stats.matched_events, self.stats.matched_markets)
    
    def calculate_opportunities(self) -> List[ArbitrageOpportunity]:
        """计算套利机会
        
        Returns:
            套利机会列表
        """
        opportunities = []
        
        for mm in self.matched_markets:
            opp = self.calculator.calculate_single(
                event_name=mm.event_name,
                team_name=mm.team_name,
                kalshi_market=mm.kalshi_market,
                kalshi_yes_price=mm.kalshi_market.yes_price,
                kalshi_no_price=mm.kalshi_market.no_price,
                polymarket_market=mm.polymarket_market,
                polymarket_yes_price=mm.poly_yes_price,
                polymarket_no_price=mm.poly_no_price
            )
            
            if opp:
                opportunities.append(opp)
        
        # 按利润率排序
        opportunities.sort(key=lambda x: x.profit_margin, reverse=True)
        
        # 更新统计
        self.stats.arbitrage_opportunities = len(opportunities)
        self.stats.last_update = datetime.now()
        
        return opportunities
    
    def get_subscription_info(self) -> Tuple[List[str], List[str], dict]:
        """获取 WebSocket 订阅信息
        
        Returns:
            (kalshi_tickers, polymarket_token_ids, market_lookup) 元组
        """
        return self.matcher.get_subscription_info(self.matched_markets)
    
    async def scan_for_new_markets(self) -> Tuple[List[MatchedMarket], List[str], List[str], dict]:
        """扫描新市场，返回增量数据
        
        定时调用此方法检测新市场。只返回新增的匹配市场及其订阅信息，
        用于热订阅到现有 WebSocket 连接。
        
        Returns:
            (new_matched_markets, new_kalshi_tickers, new_poly_token_ids, new_market_lookup) 元组
            如果没有新市场，返回空列表和空字典
        """
        logger.info("=" * 60)
        logger.info("🔄 开始扫描新市场...")
        logger.info("=" * 60)
        
        # 1. 保存旧的已匹配市场 ID 集合
        old_matched_ids = set()
        for mm in self.matched_markets:
            # 使用 kalshi_market_id + team_name 作为唯一标识
            key = f"{mm.kalshi_market.market_id}_{mm.team_name}"
            old_matched_ids.add(key)
        
        old_count = len(self.matched_markets)
        old_kalshi_count = len(self.kalshi_markets)
        old_poly_count = len(self.polymarket_markets)
        logger.info(f"   扫描前状态:")
        logger.info(f"   - Kalshi: {old_kalshi_count} 个市场")
        logger.info(f"   - Polymarket: {old_poly_count} 个市场")
        logger.info(f"   - 已匹配: {old_count} 个市场对")
        
        # 2. 重新获取市场数据
        try:
            await self.fetch_market_data()
            new_kalshi_count = len(self.kalshi_markets)
            new_poly_count = len(self.polymarket_markets)
            
            kalshi_diff = new_kalshi_count - old_kalshi_count
            poly_diff = new_poly_count - old_poly_count
            
            logger.info(f"   扫描后状态:")
            logger.info(f"   - Kalshi: {new_kalshi_count} 个市场 ({'+' if kalshi_diff >= 0 else ''}{kalshi_diff})")
            logger.info(f"   - Polymarket: {new_poly_count} 个市场 ({'+' if poly_diff >= 0 else ''}{poly_diff})")
        except Exception as e:
            logger.error(f"❌ 获取市场数据失败: {e}")
            return [], [], [], {}
        
        # 3. 重新执行匹配
        try:
            await self.match_markets()
        except Exception as e:
            logger.error(f"❌ 匹配市场失败: {e}")
            return [], [], [], {}
        
        # 4. 找出新增的匹配市场
        new_matched_markets = []
        for mm in self.matched_markets:
            key = f"{mm.kalshi_market.market_id}_{mm.team_name}"
            if key not in old_matched_ids:
                new_matched_markets.append(mm)
        
        if not new_matched_markets:
            logger.info("   ✅ 没有发现新的匹配市场")
            logger.info("=" * 60)
            return [], [], [], {}
        
        logger.info(f"   🆕 发现 {len(new_matched_markets)} 个新匹配市场:")
        for i, mm in enumerate(new_matched_markets, 1):
            logger.info(f"      {i}. {mm.event_name} ({mm.team_name})")
        
        # 5. 生成新市场的订阅信息
        new_kalshi_tickers, new_poly_token_ids, new_market_lookup = \
            self.matcher.get_subscription_info(new_matched_markets)
        
        logger.info(f"   📡 新订阅需求:")
        logger.info(f"      - Kalshi: {len(new_kalshi_tickers)} 个市场")
        logger.info(f"      - Polymarket: {len(new_poly_token_ids)} 个 token")
        logger.info("=" * 60)
        
        return new_matched_markets, new_kalshi_tickers, new_poly_token_ids, new_market_lookup
    
    def get_stats(self) -> SystemStats:
        """获取统计信息
        
        Returns:
            系统统计信息
        """
        return self.stats
    
    async def close(self):
        """关闭服务"""
        if self.kalshi_client.session:
            await self.kalshi_client.session.close()
        if self.polymarket_client.session:
            await self.polymarket_client.session.close()
