"""数据模型定义

核心概念:
- Kalshi: 一个事件有 2 个独立市场，每个市场对应一个队伍的 Yes/No
- Polymarket: 一个事件只有 1 个市场，包含两个 outcomes（两个队伍）

匹配关系:
- Kalshi 的 2 个市场都指向同一个 Poly 市场
- 根据队伍名称确定使用 Poly 的哪个价格
"""
from pydantic import BaseModel
from typing import Optional, List, Dict
from datetime import datetime
from enum import Enum


class Platform(str, Enum):
    """平台枚举"""
    KALSHI = "kalshi"
    POLYMARKET = "polymarket"


class KalshiMarket(BaseModel):
    """Kalshi 市场模型
    
    Kalshi 的每个市场是独立的，预测单个队伍的输赢
    """
    market_id: str  # ticker (如 KXNBAGAME-26JAN04MEMLAL-MEM)
    event_id: str   # 所属事件 ID
    event_name: str  # 标准化事件名称 (如 MEM-LAL)
    team_name: str   # 该市场预测的队伍 (如 MEM)
    opponent_name: str  # 对手队伍 (如 LAL)
    yes_price: float  # Yes 价格 (该队伍获胜)
    no_price: float   # No 价格 (该队伍不获胜)
    start_time: Optional[datetime] = None
    volume: Optional[float] = None
    liquidity: Optional[float] = None


class PolymarketMarket(BaseModel):
    """Polymarket 市场模型
    
    Polymarket 的一个市场包含两个 outcomes（两个队伍）
    不拆分，保持原始结构
    """
    market_id: str  # condition_id
    event_name: str  # 标准化事件名称 (如 MEM-LAL)
    team_a: str  # 队伍 A (字母序靠前)
    team_b: str  # 队伍 B (字母序靠后)
    price_a: float  # 队伍 A 获胜价格
    price_b: float  # 队伍 B 获胜价格
    start_time: Optional[datetime] = None
    volume: Optional[float] = None
    
    # WebSocket 订阅信息
    token_id_a: Optional[str] = None  # 队伍 A 的 token ID
    token_id_b: Optional[str] = None  # 队伍 B 的 token ID
    
    def get_price_for_team(self, team: str) -> tuple[float, float]:
        """根据队伍名称获取 Yes/No 价格
        
        返回: (yes_price, no_price) 对于该队伍
        """
        team_upper = team.upper()
        if team_upper == self.team_a.upper():
            return (self.price_a, self.price_b)  # yes=A获胜, no=B获胜
        elif team_upper == self.team_b.upper():
            return (self.price_b, self.price_a)  # yes=B获胜, no=A获胜
        else:
            raise ValueError(f"队伍 {team} 不在市场 {self.event_name} 中")
    
    def get_token_for_team(self, team: str) -> Optional[str]:
        """根据队伍名称获取 token ID"""
        team_upper = team.upper()
        if team_upper == self.team_a.upper():
            return self.token_id_a
        elif team_upper == self.team_b.upper():
            return self.token_id_b
        return None


class Event(BaseModel):
    """事件模型 - 表示一场比赛"""
    event_id: str
    platform: Platform
    name: str  # 标准化事件名称 (如 MEM-LAL)
    team_a: str
    team_b: str
    start_time: Optional[datetime] = None
    category: str = "NBA"


class KalshiEvent(Event):
    """Kalshi 事件"""
    markets: List[KalshiMarket] = []
    
    def get_market_by_team(self, team: str) -> Optional[KalshiMarket]:
        """根据队伍名称获取市场"""
        for market in self.markets:
            if market.team_name.upper() == team.upper():
                return market
        return None


class PolymarketEvent(Event):
    """Polymarket 事件"""
    market: Optional[PolymarketMarket] = None  # 一个事件只有一个市场


class MatchedMarket(BaseModel):
    """匹配的市场对 - 用于套利计算
    
    一个 Kalshi 市场对应一个 Poly 市场（的某个视角）
    """
    event_name: str
    team_name: str  # Kalshi 市场预测的队伍
    
    # Kalshi 端
    kalshi_market: KalshiMarket
    
    # Polymarket 端（不拆分，引用原始市场）
    polymarket_market: PolymarketMarket
    
    # 缓存的 Poly 价格（根据 team_name 计算）
    poly_yes_price: float  # 该队伍获胜价格
    poly_no_price: float   # 该队伍不获胜价格
    
    confidence: float = 0.0
    
    def update_poly_prices(self):
        """根据 team_name 更新 Poly 价格"""
        self.poly_yes_price, self.poly_no_price = self.polymarket_market.get_price_for_team(self.team_name)


class MatchedEvent(BaseModel):
    """匹配的事件"""
    event_name: str
    kalshi_event: Optional[KalshiEvent] = None
    polymarket_event: Optional[PolymarketEvent] = None
    confidence: float = 0.0


class PriceUpdate(BaseModel):
    """价格更新"""
    platform: Platform
    market_id: str  # Kalshi: ticker, Polymarket: token_id
    yes_bid: Optional[float] = None
    yes_ask: Optional[float] = None
    no_bid: Optional[float] = None
    no_ask: Optional[float] = None
    timestamp: datetime


class ArbitrageOpportunity(BaseModel):
    """套利机会"""
    event_name: str
    team_name: str
    
    # Kalshi 端
    kalshi_market_id: str
    kalshi_price: float
    kalshi_side: str  # "yes" 或 "no"
    kalshi_bet: float
    
    # Polymarket 端
    polymarket_market_id: str
    polymarket_price: float
    polymarket_side: str  # "yes" 或 "no"
    polymarket_bet: float
    
    # 套利信息
    total_bet: float
    profit_margin: float
    expected_profit: float
    timestamp: datetime
    start_time: Optional[datetime] = None  # 比赛开始时间


class SystemStats(BaseModel):
    """系统统计信息"""
    total_kalshi_events: int = 0
    total_kalshi_markets: int = 0
    total_polymarket_events: int = 0
    total_polymarket_markets: int = 0  # 不拆分后，等于事件数
    matched_events: int = 0
    matched_markets: int = 0  # Kalshi 市场数（每个都有对应的 Poly 视角）
    arbitrage_opportunities: int = 0
    kalshi_ws_connected: bool = False
    polymarket_ws_connected: bool = False
    last_update: Optional[datetime] = None
