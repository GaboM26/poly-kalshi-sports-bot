"""Polymarket API 客户端"""
import aiohttp
import json
import logging
from typing import List, Optional, Dict
from datetime import datetime
from models import Market, Platform
from config import PolymarketConfig

logger = logging.getLogger(__name__)


class PolymarketClient:
    """Polymarket API 客户端"""
    
    def __init__(self, config: PolymarketConfig):
        self.config = config
        self.base_url = config.base_url
        self.clob_url = config.clob_url
        self.session: Optional[aiohttp.ClientSession] = None
        self.team_mappings: Dict[str, str] = {}
        
    async def __aenter__(self):
        self.session = aiohttp.ClientSession()
        return self
        
    async def __aexit__(self, exc_type, exc_val, exc_tb):
        if self.session:
            await self.session.close()
    
    async def initialize_team_mappings(self):
        """初始化球队映射"""
        # 这里可以从 API 获取或使用预定义的映射
        self.team_mappings = {
            "Nets": "Nets",
            "Wizards": "Wizards",
            "Spurs": "Spurs",
            "Pacers": "Pacers",
            "Nuggets": "Nuggets",
            "Cavaliers": "Cavaliers",
            "Hawks": "Hawks",
            "Knicks": "Knicks",
            "Magic": "Magic",
            "Bulls": "Bulls",
            "Hornets": "Hornets",
            "Bucks": "Bucks",
            # 添加更多映射...
        }
        logger.info("✅ Polymarket 球队映射初始化完成")
    
    async def get_nba_markets(self) -> List[Market]:
        """获取 NBA 市场"""
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            # 1. 获取体育联赛
            sports_url = f"{self.base_url}/sports"
            async with self.session.get(sports_url) as resp:
                if resp.status != 200:
                    logger.error(f"获取 Polymarket 体育联赛失败: {resp.status}")
                    return []
                
                sports = await resp.json()
            
            # 2. 找到 NBA 联赛
            nba_leagues = [
                s for s in sports 
                if 'NBA' in s.get('sport', '').upper() 
                and 'WNBA' not in s.get('sport', '').upper()
            ]
            
            if not nba_leagues:
                logger.warning("未找到 NBA 联赛")
                return []
            
            markets = []
            
            # 3. 获取 NBA 赛事
            for league in nba_leagues[:1]:  # 只取第一个联赛
                series_id = league.get('series')
                
                events_url = f"{self.base_url}/events"
                params = {
                    'series_id': str(series_id),
                    'tag_id': '100639',  # NBA 标签
                    'active': 'true',
                    'closed': 'false',
                    'limit': '50'
                }
                
                async with self.session.get(events_url, params=params) as resp:
                    if resp.status != 200:
                        continue
                    
                    events = await resp.json()
                
                # 4. 处理每个赛事
                for event in events:
                    title = event.get('title', '')
                    event_markets = event.get('markets', [])
                    
                    for market in event_markets:
                        question = market.get('question', '')
                        
                        # 只处理主市场（问题 == 标题）
                        if question != title:
                            continue
                        
                        # 解析结果和 token IDs
                        outcomes_str = market.get('outcomes')
                        clob_token_ids_str = market.get('clobTokenIds')
                        
                        if not outcomes_str or not clob_token_ids_str:
                            continue
                        
                        try:
                            outcomes = json.loads(outcomes_str)
                            token_ids = json.loads(clob_token_ids_str)
                            
                            # 获取价格（从 market 数据本身）
                            outcome_prices_str = market.get('outcomePrices')
                            if not outcome_prices_str:
                                logger.debug(f"市场 {market.get('id')} 缺少 outcomePrices")
                                continue
                            
                            outcome_prices = json.loads(outcome_prices_str)
                            
                            # 过滤掉 Yes/No 类型的市场
                            if any(o.lower() in ['yes', 'no', 'over', 'under'] for o in outcomes):
                                continue
                            
                            # 只处理双方对决的市场
                            if len(outcomes) != 2 or len(token_ids) != 2 or len(outcome_prices) != 2:
                                continue
                            
                            team_a = outcomes[0]
                            team_b = outcomes[1]
                            
                            # 解析价格（字符串转浮点数）
                            try:
                                price_a = float(outcome_prices[0])
                                price_b = float(outcome_prices[1])
                            except (ValueError, TypeError):
                                logger.debug(f"无法解析价格: {outcome_prices}")
                                continue
                            
                            # 验证价格有效性
                            if price_a < 0 or price_a > 1 or price_b < 0 or price_b > 1:
                                logger.debug(f"价格无效: {price_a}, {price_b}")
                                continue
                            
                            market_obj = Market(
                                market_id=market.get('id', ''),
                                platform=Platform.POLYMARKET,
                                event_name=f"{team_a}-{team_b}",
                                team_a=team_a,
                                team_b=team_b,
                                price_a=price_a,
                                price_b=price_b,
                                start_time=self._parse_datetime(event.get('startDate')),
                                end_time=self._parse_datetime(event.get('endDate'))
                            )
                            markets.append(market_obj)
                            
                        except Exception as e:
                            logger.debug(f"解析市场失败: {e}")
                            continue
            
            logger.info(f"✅ 获取到 {len(markets)} 个 Polymarket NBA 市场")
            return markets
            
        except Exception as e:
            logger.error(f"❌ 获取 Polymarket NBA 市场异常: {e}")
            return []
    
    
    def _parse_datetime(self, date_str: Optional[str]) -> Optional[datetime]:
        """解析日期时间"""
        if not date_str:
            return None
        try:
            return datetime.fromisoformat(date_str.replace('Z', '+00:00'))
        except:
            return None
