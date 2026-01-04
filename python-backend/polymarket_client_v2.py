"""Polymarket API 客户端 - 完全按照 Rust 版本重写"""
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
        
        # NBA 球队映射 (全名/别名 -> 标准缩写)
        self.team_mappings = {
            # 东部
            "ATLANTA HAWKS": "ATL", "HAWKS": "ATL", "ATL": "ATL",
            "BOSTON CELTICS": "BOS", "CELTICS": "BOS", "BOS": "BOS",
            "BROOKLYN NETS": "BKN", "NETS": "BKN", "BKN": "BKN",
            "CHARLOTTE HORNETS": "CHA", "HORNETS": "CHA", "CHA": "CHA",
            "CHICAGO BULLS": "CHI", "BULLS": "CHI", "CHI": "CHI",
            "CLEVELAND CAVALIERS": "CLE", "CAVALIERS": "CLE", "CAVS": "CLE", "CLE": "CLE",
            "DETROIT PISTONS": "DET", "PISTONS": "DET", "DET": "DET",
            "INDIANA PACERS": "IND", "PACERS": "IND", "IND": "IND",
            "MIAMI HEAT": "MIA", "HEAT": "MIA", "MIA": "MIA",
            "MILWAUKEE BUCKS": "MIL", "BUCKS": "MIL", "MIL": "MIL",
            "NEW YORK KNICKS": "NYK", "KNICKS": "NYK", "NYK": "NYK",
            "ORLANDO MAGIC": "ORL", "MAGIC": "ORL", "ORL": "ORL",
            "PHILADELPHIA 76ERS": "PHI", "76ERS": "PHI", "SIXERS": "PHI", "PHI": "PHI",
            "TORONTO RAPTORS": "TOR", "RAPTORS": "TOR", "TOR": "TOR",
            "WASHINGTON WIZARDS": "WAS", "WIZARDS": "WAS", "WAS": "WAS",
            
            # 西部
            "DALLAS MAVERICKS": "DAL", "MAVERICKS": "DAL", "MAVS": "DAL", "DAL": "DAL",
            "DENVER NUGGETS": "DEN", "NUGGETS": "DEN", "DEN": "DEN",
            "GOLDEN STATE WARRIORS": "GSW", "WARRIORS": "GSW", "GSW": "GSW",
            "HOUSTON ROCKETS": "HOU", "ROCKETS": "HOU", "HOU": "HOU",
            "LOS ANGELES CLIPPERS": "LAC", "CLIPPERS": "LAC", "LAC": "LAC",
            "LOS ANGELES LAKERS": "LAL", "LAKERS": "LAL", "LAL": "LAL",
            "MEMPHIS GRIZZLIES": "MEM", "GRIZZLIES": "MEM", "MEM": "MEM",
            "MINNESOTA TIMBERWOLVES": "MIN", "TIMBERWOLVES": "MIN", "WOLVES": "MIN", "MIN": "MIN",
            "NEW ORLEANS PELICANS": "NOP", "PELICANS": "NOP", "NOP": "NOP",
            "OKLAHOMA CITY THUNDER": "OKC", "THUNDER": "OKC", "OKC": "OKC",
            "PHOENIX SUNS": "PHX", "SUNS": "PHX", "PHX": "PHX",
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
    
    async def initialize_team_mappings(self):
        """初始化球队映射（已经在 __init__ 中完成）"""
        logger.info("✅ Polymarket 球队映射初始化完成")
    
    def normalize_team_name(self, team_name: str) -> str:
        """标准化球队名称为缩写"""
        team_upper = team_name.strip().upper()
        return self.team_mappings.get(team_upper, team_upper)
    
    async def get_nba_markets(self) -> List[Market]:
        """获取 NBA 市场 - 完全按照 Rust 版本的逻辑"""
        try:
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            # 1. 获取体育联赛
            sports_url = f"{self.base_url}/sports"
            async with self.session.get(sports_url) as resp:
                if resp.status != 200:
                    logger.error(f"获取体育联赛失败: {resp.status}")
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
            
            all_markets = []
            
            # 3. 获取 NBA 赛事
            for league in nba_leagues[:1]:
                series_id = league.get('series')
                
                events_url = f"{self.base_url}/events"
                params = {
                    'series_id': str(series_id),
                    'tag_id': '100639',  # NBA 标签
                    'active': 'true',
                    'closed': 'false',
                    'limit': '100'
                }
                
                async with self.session.get(events_url, params=params) as resp:
                    if resp.status != 200:
                        continue
                    events = await resp.json()
                
                # 4. 处理每个赛事
                for event in events:
                    event_title = event.get('title', '')
                    event_markets = event.get('markets', [])
                    
                    # 从 slug 提取日期
                    event_date = self._extract_date_from_slug(event.get('slug', ''))
                    
                    for market in event_markets:
                        # 解析市场，返回 0-2 个 Market 对象
                        markets = self._parse_event_market(
                            market, 
                            event_title, 
                            "NBA",
                            event_date
                        )
                        all_markets.extend(markets)
            
            logger.info(f"✅ 获取到 {len(all_markets)} 个 Polymarket NBA 市场")
            return all_markets
            
        except Exception as e:
            logger.error(f"❌ 获取 Polymarket NBA 市场异常: {e}")
            import traceback
            traceback.print_exc()
            return []
    
    def _extract_date_from_slug(self, slug: str) -> Optional[datetime]:
        """从 slug 提取日期
        例如: "nba-lakers-vs-celtics-2026-01-05" -> 2026-01-05
        """
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
    
    def _parse_event_market(
        self, 
        market: dict, 
        event_title: str, 
        category: str,
        event_date: Optional[datetime]
    ) -> List[Market]:
        """解析单个市场
        返回: 0-2 个 Market 对象（每个队伍一个）
        """
        market_id = market.get('id')
        if not market_id:
            return []
        
        question = market.get('question', event_title)
        
        # 获取 outcomes 和 outcomePrices
        outcomes_str = market.get('outcomes')
        prices_str = market.get('outcomePrices')
        
        if not outcomes_str or not prices_str:
            return []
        
        try:
            outcomes = json.loads(outcomes_str)
            prices = json.loads(prices_str)
        except:
            return []
        
        # 必须是二元市场
        if len(outcomes) != 2 or len(prices) != 2:
            return []
        
        # 解析价格
        try:
            price1 = float(prices[0])
            price2 = float(prices[1])
        except:
            return []
        
        # 验证价格有效性
        if price1 < 0 or price1 > 1 or price2 < 0 or price2 > 1:
            return []
        
        # 检查是否是 Yes/No 格式（过滤掉）
        is_yes_no = any(o.lower() == "yes" for o in outcomes) and any(o.lower() == "no" for o in outcomes)
        if is_yes_no:
            return []
        
        # 只保留全场输赢市场（question == event_title）
        if question != event_title:
            return []
        
        # 排除 Over/Under 市场
        if outcomes[0].lower() in ["over", "under"]:
            return []
        
        # 提取并标准化球队名称
        team1_raw = outcomes[0]
        team2_raw = outcomes[1]
        
        team1_abbr = self.normalize_team_name(team1_raw)
        team2_abbr = self.normalize_team_name(team2_raw)
        
        # 按字母顺序排序（与 Kalshi 保持一致）
        if team1_abbr < team2_abbr:
            abbr1, abbr2 = team1_abbr, team2_abbr
            abbr1_price, abbr2_price = price1, price2
            team1_is_first = True
        else:
            abbr1, abbr2 = team2_abbr, team1_abbr
            abbr1_price, abbr2_price = price2, price1
            team1_is_first = False
        
        # 构建标准化的事件名称
        event_name = f"{abbr1}-{abbr2}"
        
        # 获取交易量
        volume = 0.0
        try:
            volume_str = market.get('volume')
            if volume_str:
                volume = float(volume_str)
        except:
            pass
        
        # 为每个队伍创建一个 Market 对象
        markets = []
        
        # Market 1: abbr1 获胜的市场
        market1 = Market(
            market_id=f"{market_id}-{abbr1}",
            platform=Platform.POLYMARKET,
            event_name=event_name,
            team_a=abbr1,
            team_b=abbr2,
            price_a=abbr1_price,  # abbr1 获胜的价格
            price_b=abbr2_price,  # abbr2 获胜的价格
            start_time=event_date,
            end_time=event_date,
            volume=volume,
            liquidity=0
        )
        markets.append(market1)
        
        # Market 2: abbr2 获胜的市场
        market2 = Market(
            market_id=f"{market_id}-{abbr2}",
            platform=Platform.POLYMARKET,
            event_name=event_name,
            team_a=abbr2,
            team_b=abbr1,
            price_a=abbr2_price,  # abbr2 获胜的价格
            price_b=abbr1_price,  # abbr1 获胜的价格
            start_time=event_date,
            end_time=event_date,
            volume=volume,
            liquidity=0
        )
        markets.append(market2)
        
        logger.debug(f"✅ 解析市场: {event_name} -> {abbr1} ({abbr1_price:.2f}) vs {abbr2} ({abbr2_price:.2f})")
        
        return markets
