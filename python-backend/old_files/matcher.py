"""事件和市场匹配引擎

匹配逻辑优化:
- 事件匹配: 使用队伍缩写 + 开赛日期匹配两个平台的事件
- 市场匹配: 2:1 匹配（两个 Kalshi 市场对应一个 Poly 市场）

关键点:
- Kalshi: 一个事件有 2 个市场（每个队伍一个）
- Polymarket: 一个事件只有 1 个市场（包含两个队伍的价格）
- 匹配时根据队伍名称确定 Poly 的价格视角
"""
import logging
from typing import List, Dict, Optional, Tuple
from datetime import datetime
from models import (
    KalshiEvent, KalshiMarket, 
    PolymarketEvent, PolymarketMarket,
    MatchedEvent, MatchedMarket
)

logger = logging.getLogger(__name__)


class EventMatcher:
    """事件匹配器 - 两阶段匹配"""
    
    def __init__(self, time_tolerance_hours: int = 24):
        self.time_tolerance_hours = time_tolerance_hours
    
    def match_events_and_markets(
        self,
        kalshi_events: List[KalshiEvent],
        kalshi_markets: List[KalshiMarket],
        polymarket_events: List[PolymarketEvent],
        polymarket_markets: List[PolymarketMarket]
    ) -> Tuple[List[MatchedEvent], List[MatchedMarket]]:
        """执行两阶段匹配"""
        logger.info("=" * 60)
        logger.info("🔍 开始两阶段匹配 (不拆分版)")
        logger.info(f"   Kalshi: {len(kalshi_events)} 个事件, {len(kalshi_markets)} 个市场")
        logger.info(f"   Polymarket: {len(polymarket_events)} 个事件, {len(polymarket_markets)} 个市场")
        logger.info("=" * 60)
        
        # 第一阶段: 事件匹配
        matched_events = self._match_events(kalshi_events, polymarket_events)
        logger.info(f"📊 第一阶段完成: 匹配到 {len(matched_events)} 个事件")
        
        # 第二阶段: 市场匹配 (2:1)
        matched_markets = self._match_markets(matched_events)
        logger.info(f"📊 第二阶段完成: 匹配到 {len(matched_markets)} 个市场对")
        
        return matched_events, matched_markets
    
    def _match_events(
        self,
        kalshi_events: List[KalshiEvent],
        polymarket_events: List[PolymarketEvent]
    ) -> List[MatchedEvent]:
        """第一阶段: 事件匹配"""
        logger.info("-" * 40)
        logger.info("🎯 第一阶段: 事件匹配")
        logger.info("-" * 40)
        
        matched_events = []
        used_poly_ids = set()
        
        # 构建 Polymarket 事件索引: event_name -> [events]
        poly_index: Dict[str, List[PolymarketEvent]] = {}
        for event in polymarket_events:
            key = event.name.upper()
            if key not in poly_index:
                poly_index[key] = []
            poly_index[key].append(event)
        
        # 为每个 Kalshi 事件寻找匹配
        for k_event in kalshi_events:
            k_name = k_event.name.upper()
            k_date = k_event.start_time.date() if k_event.start_time else None
            
            # 也检查反向名称 (如 MEM-LAL vs LAL-MEM)
            parts = k_name.split('-')
            k_name_reversed = f"{parts[1]}-{parts[0]}" if len(parts) == 2 else None
            
            best_match: Optional[PolymarketEvent] = None
            best_confidence = 0.0
            
            # 查找精确匹配或反向匹配
            for name_to_check, is_reversed in [(k_name, False), (k_name_reversed, True)]:
                if not name_to_check:
                    continue
                
                candidates = poly_index.get(name_to_check, [])
                
                for p_event in candidates:
                    if p_event.event_id in used_poly_ids:
                        continue
                    
                    p_date = p_event.start_time.date() if p_event.start_time else None
                    
                    # 验证日期
                    if k_date and p_date:
                        if k_date != p_date:
                            logger.debug(f"   ❌ 日期不匹配: {k_event.name} ({k_date}) vs {p_event.name} ({p_date})")
                            continue
                        confidence = 1.0 if not is_reversed else 0.95
                    else:
                        logger.warning(f"   ⚠️ 缺少日期: {k_event.name} ({k_date}) vs {p_event.name} ({p_date})")
                        confidence = 0.7 if not is_reversed else 0.65
                    
                    if confidence > best_confidence:
                        best_confidence = confidence
                        best_match = p_event
            
            if best_match and best_confidence >= 0.7:
                matched = MatchedEvent(
                    event_name=k_event.name,
                    kalshi_event=k_event,
                    polymarket_event=best_match,
                    confidence=best_confidence
                )
                matched_events.append(matched)
                used_poly_ids.add(best_match.event_id)
                
                logger.info(f"   ✅ 匹配: {k_event.name} <-> {best_match.name} (置信度: {best_confidence:.2f})")
            else:
                logger.warning(f"   ❌ 未找到匹配: {k_event.name}")
        
        return matched_events
    
    def _match_markets(
        self,
        matched_events: List[MatchedEvent]
    ) -> List[MatchedMarket]:
        """第二阶段: 市场匹配 (2:1)
        
        对于每个匹配的事件:
        - Kalshi 有 2 个市场（每个队伍一个）
        - Polymarket 有 1 个市场（包含两个队伍）
        - 创建 2 个 MatchedMarket，每个对应一个 Kalshi 市场
        """
        logger.info("-" * 40)
        logger.info("🎯 第二阶段: 市场匹配 (2:1)")
        logger.info("-" * 40)
        
        matched_markets = []
        
        for matched_event in matched_events:
            k_event = matched_event.kalshi_event
            p_event = matched_event.polymarket_event
            
            if not k_event or not p_event or not p_event.market:
                continue
            
            poly_market = p_event.market
            
            # 对于 Kalshi 的每个市场，创建一个 MatchedMarket
            for k_market in k_event.markets:
                team_name = k_market.team_name.upper()
                
                # 获取 Poly 市场对于该队伍的价格
                try:
                    poly_yes, poly_no = poly_market.get_price_for_team(team_name)
                except ValueError as e:
                    logger.warning(f"   ⚠️ {e}")
                    continue
                
                matched = MatchedMarket(
                    event_name=matched_event.event_name,
                    team_name=team_name,
                    kalshi_market=k_market,
                    polymarket_market=poly_market,
                    poly_yes_price=poly_yes,
                    poly_no_price=poly_no,
                    confidence=matched_event.confidence
                )
                matched_markets.append(matched)
                
                logger.info(f"   ✅ 市场匹配: {matched_event.event_name} - {team_name}")
                logger.debug(f"      Kalshi: Yes={k_market.yes_price:.2f}, No={k_market.no_price:.2f}")
                logger.debug(f"      Poly:   Yes={poly_yes:.2f}, No={poly_no:.2f}")
        
        return matched_markets
    
    def get_subscription_info(
        self,
        matched_markets: List[MatchedMarket]
    ) -> Tuple[List[str], List[str], Dict[str, List[MatchedMarket]]]:
        """获取 WebSocket 订阅信息
        
        Returns:
            kalshi_tickers: Kalshi 需要订阅的 ticker 列表
            polymarket_token_ids: Polymarket 需要订阅的 token ID 列表
            market_lookup: subscription_id -> [MatchedMarket] 的映射
        """
        kalshi_tickers = []
        polymarket_token_ids = []
        market_lookup: Dict[str, List[MatchedMarket]] = {}
        
        seen_kalshi = set()
        seen_poly = set()
        
        for mm in matched_markets:
            # Kalshi ticker
            k_id = mm.kalshi_market.market_id
            if k_id and k_id not in seen_kalshi:
                kalshi_tickers.append(k_id)
                seen_kalshi.add(k_id)
            
            # 添加到 lookup
            if k_id not in market_lookup:
                market_lookup[k_id] = []
            market_lookup[k_id].append(mm)
            
            # Polymarket token (根据队伍获取对应的 token)
            p_token = mm.polymarket_market.get_token_for_team(mm.team_name)
            if p_token and p_token not in seen_poly:
                polymarket_token_ids.append(p_token)
                seen_poly.add(p_token)
            
            # Poly token 也添加到 lookup
            if p_token:
                if p_token not in market_lookup:
                    market_lookup[p_token] = []
                market_lookup[p_token].append(mm)
        
        logger.info(f"📡 订阅信息: Kalshi {len(kalshi_tickers)} 个, Polymarket {len(polymarket_token_ids)} 个 token")
        
        return kalshi_tickers, polymarket_token_ids, market_lookup
