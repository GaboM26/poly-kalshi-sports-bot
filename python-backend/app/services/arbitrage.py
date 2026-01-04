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
