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
import time
from typing import List, Optional, Dict, Callable, Tuple
from datetime import datetime
from app.core.models import PolymarketMarket, PolymarketEvent, Platform, PriceUpdate
from app.core.config import PolymarketConfig

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
    
    def _build_hmac_signature(self, timestamp: int, method: str, request_path: str, body: str = "") -> str:
        """
        构建 HMAC 签名（L2 认证）
        
        Args:
            timestamp: Unix 时间戳
            method: HTTP 方法
            request_path: API 端点路径
            body: 请求体字符串
            
        Returns:
            URL-safe base64 编码的 HMAC 签名
        """
        import hmac
        import hashlib
        import base64
        
        # 构建消息: timestamp + method + requestPath + body
        message = f"{timestamp}{method.upper()}{request_path}{body}"
        
        # 解码 base64 密钥
        secret = self.config.api_secret.replace('-', '+').replace('_', '/')
        # 添加填充
        padding_needed = (4 - len(secret) % 4) % 4
        secret += '=' * padding_needed
        
        secret_bytes = base64.b64decode(secret)
        
        # 创建 HMAC-SHA256 签名
        signature = hmac.new(secret_bytes, message.encode(), hashlib.sha256).digest()
        
        # 编码为 base64 并转为 URL-safe
        sig_b64 = base64.b64encode(signature).decode()
        sig_urlsafe = sig_b64.replace('+', '-').replace('/', '_')
        
        return sig_urlsafe
    
    def _get_controller_address(self) -> str:
        """从私钥获取控制器地址"""
        from eth_account import Account
        
        private_key = self.config.private_key
        if private_key.startswith('0x'):
            private_key = private_key[2:]
        
        account = Account.from_key(private_key)
        return account.address
    
    def _build_eip712_signature(self, timestamp: int, nonce: int = 0) -> str:
        """构建 EIP-712 签名用于 L1 认证"""
        from eth_account import Account
        from eth_account.messages import encode_typed_data
        
        private_key = self.config.private_key
        if private_key.startswith('0x'):
            private_key = private_key[2:]
        
        account = Account.from_key(private_key)
        
        # EIP-712 domain
        domain_data = {
            "name": "ClobAuthDomain",
            "version": "1",
            "chainId": 137,  # Polygon
        }
        
        message_types = {
            "ClobAuth": [
                {"name": "address", "type": "address"},
                {"name": "timestamp", "type": "string"},
                {"name": "nonce", "type": "uint256"},
                {"name": "message", "type": "string"},
            ]
        }
        
        message_data = {
            "address": account.address,
            "timestamp": str(timestamp),
            "nonce": nonce,
            "message": "This message attests that I control the given wallet",
        }
        
        signable_message = encode_typed_data(
            domain_data=domain_data,
            message_types=message_types,
            message_data=message_data
        )
        
        signed_message = account.sign_message(signable_message)
        return "0x" + signed_message.signature.hex()
    
    async def _derive_api_credentials(self) -> bool:
        """自动派生 API 凭据（如果未配置）
        
        Returns:
            True 如果凭据已存在或派生成功，False 如果失败
        """
        # 如果已有凭据，直接返回
        if self.config.api_key and self.config.api_secret and self.config.api_passphrase:
            return True
        
        if not self.config.private_key:
            logger.error("❌ 未配置私钥，无法派生 API 凭据")
            return False
        
        try:
            import time
            import requests
            
            controller_address = self._get_controller_address()
            timestamp = int(time.time())
            signature = self._build_eip712_signature(timestamp)
            
            headers = {
                "Content-Type": "application/json",
                "POLY_ADDRESS": controller_address,
                "POLY_SIGNATURE": signature,
                "POLY_TIMESTAMP": str(timestamp),
                "POLY_NONCE": "0",
            }
            
            # 先尝试派生已有的 API Key
            url = f"{self.clob_url}/auth/derive-api-key"
            resp = requests.get(url, headers=headers, timeout=10)
            
            if resp.status_code == 200:
                data = resp.json()
                self.config.api_key = data["apiKey"]
                self.config.api_secret = data["secret"]
                self.config.api_passphrase = data["passphrase"]
                logger.info(f"✅ API 凭据派生成功")
                return True
            
            # 派生失败，尝试创建新的
            logger.info("📝 未找到已有 API Key，正在创建新的...")
            
            # 重新生成签名（时间戳可能过期）
            timestamp = int(time.time())
            signature = self._build_eip712_signature(timestamp)
            headers["POLY_SIGNATURE"] = signature
            headers["POLY_TIMESTAMP"] = str(timestamp)
            
            url = f"{self.clob_url}/auth/api-key"
            resp = requests.post(url, headers=headers, timeout=10)
            
            if resp.status_code == 200:
                data = resp.json()
                self.config.api_key = data["apiKey"]
                self.config.api_secret = data["secret"]
                self.config.api_passphrase = data["passphrase"]
                logger.info(f"✅ API 凭据创建成功")
                return True
            else:
                logger.error(f"❌ 创建 API 凭据失败: {resp.status_code} - {resp.text}")
                return False
                
        except Exception as e:
            logger.error(f"❌ 派生 API 凭据异常: {e}")
            return False
    
    async def get_balance(self) -> Optional[Dict]:
        """获取 Smart Wallet 账户余额（使用 CLOB API L2 认证）
        
        对于 Magic Link 用户:
        - controller_address: 从私钥派生，用于签名
        - wallet_address: Smart Wallet 地址，资金存放位置
        - API 会自动返回关联的 Smart Wallet 余额
        
        只需配置: private_key, wallet_address
        API 凭据会自动派生
        
        Returns:
            Dict with keys:
            - balance: 可用余额（USDC，已转换为美元）
            - allowances: 授权额度字典
            - smart_wallet: Smart Wallet 地址
            返回 None 如果未配置凭据或请求失败
        """
        if not self.config.wallet_address:
            logger.debug("⚠️ Polymarket Smart Wallet 地址未配置")
            return None
        
        if not self.config.private_key:
            logger.debug("⚠️ Polymarket 私钥未配置")
            return None
        
        # 自动派生 API 凭据
        if not await self._derive_api_credentials():
            logger.error("❌ 无法获取 API 凭据")
            return None
        
        try:
            import time
            
            # 获取控制器地址（从私钥派生，用于 L2 认证签名）
            controller_address = self._get_controller_address()
            
            # 构建请求
            timestamp = int(time.time())
            method = "GET"
            request_path = "/balance-allowance"
            
            # 构建查询参数
            # 对于 Magic Link 用户，使用 signature_type = 1
            # API 会自动返回关联的 Smart Wallet 余额
            params = {
                "asset_type": "COLLATERAL",
                "signature_type": str(self.config.signature_type)
            }
            
            # 构建 HMAC 签名（路径不包含查询参数）
            signature = self._build_hmac_signature(timestamp, method, request_path, "")
            
            # 构建请求头（L2 认证使用控制器地址）
            headers = {
                "Content-Type": "application/json",
                "POLY_ADDRESS": controller_address,
                "POLY_SIGNATURE": signature,
                "POLY_TIMESTAMP": str(timestamp),
                "POLY_API_KEY": self.config.api_key,
                "POLY_PASSPHRASE": self.config.api_passphrase,
            }
            
            url = f"{self.clob_url}{request_path}"
            
            if not self.session:
                self.session = aiohttp.ClientSession()
            
            async with self.session.get(url, headers=headers, params=params) as resp:
                if resp.status == 200:
                    data = await resp.json()
                    
                    # 转换余额（从 wei 到美元，USDC 有 6 位小数）
                    balance = float(data.get('balance', 0)) / 1e6
                    
                    # allowances 是一个字典，包含多个合约的授权额度
                    allowances = data.get('allowances', {})
                    
                    logger.info(f"✅ Polymarket Smart Wallet 余额: ${balance:.2f}")
                    logger.debug(f"   Smart Wallet: {self.config.wallet_address}")
                    logger.debug(f"   Controller: {controller_address}")
                    
                    return {
                        'balance': balance,
                        'allowances': allowances,
                        'smart_wallet': self.config.wallet_address,
                        'controller': controller_address
                    }
                else:
                    error_text = await resp.text()
                    logger.error(f"❌ 获取 Polymarket 余额失败: {resp.status} - {error_text}")
                    return None
                    
        except ImportError as e:
            logger.error(f"⚠️ 缺少依赖库: {e}")
            logger.error("请运行: pip install web3 eth-account")
            return None
        except Exception as e:
            logger.error(f"❌ 获取 Polymarket 余额异常: {e}")
            import traceback
            traceback.print_exc()
            return None
    
    def _get_clob_client(self):
        """获取 py-clob-client 的 ClobClient 实例
        
        Returns:
            ClobClient 实例，失败返回 None
        """
        try:
            from py_clob_client.client import ClobClient
            from py_clob_client.clob_types import ApiCreds
            
            # 确保有 API 凭据
            if not self.config.api_key or not self.config.api_secret or not self.config.api_passphrase:
                logger.error("❌ API 凭据未配置，请先调用 get_balance() 或 _derive_api_credentials()")
                return None
            
            api_creds = ApiCreds(
                api_key=self.config.api_key,
                api_secret=self.config.api_secret,
                api_passphrase=self.config.api_passphrase
            )
            
            # 获取私钥（去掉 0x 前缀）
            private_key = self.config.private_key
            if private_key.startswith('0x'):
                private_key = private_key[2:]
            
            # 创建客户端
            # signature_type: 1 = Magic Link 用户 (EOA signing for Polymarket Proxy Wallet)
            # funder: Smart Wallet 地址（资金来源）
            client = ClobClient(
                host=self.clob_url,
                chain_id=137,  # Polygon
                key=private_key,
                creds=api_creds,
                signature_type=self.config.signature_type,  # 1 for Magic Link
                funder=self.config.wallet_address  # Smart Wallet 地址
            )
            
            return client
            
        except ImportError as e:
            logger.error(f"❌ 缺少 py-clob-client 库: {e}")
            logger.error("请运行: pip install py-clob-client")
            return None
        except Exception as e:
            logger.error(f"❌ 创建 ClobClient 失败: {e}")
            import traceback
            traceback.print_exc()
            return None
    
    async def create_market_order(
        self,
        token_id: str,
        side: str,
        amount: float,
        price: Optional[float] = None,
        tick_size: str = "0.01"
    ) -> Tuple[Optional[Dict], float]:
        """创建市价订单
        
        Args:
            token_id: Token ID（从市场数据中获取）
            side: "buy" 或 "sell"
            amount: 下单金额（USDC）
            price: 可选的价格限制（市价单通常不需要）
            tick_size: 价格精度（"0.1", "0.01", "0.001", "0.0001"）
            
        Returns:
            (订单响应, 下单耗时ms)
            订单响应为 None 表示下单失败
        """
        start_time = time.perf_counter()
        
        try:
            # 确保有 API 凭据
            if not await self._derive_api_credentials():
                logger.error("❌ 无法获取 API 凭据")
                elapsed_ms = (time.perf_counter() - start_time) * 1000
                return None, elapsed_ms
            
            # 获取 ClobClient
            client = self._get_clob_client()
            if not client:
                elapsed_ms = (time.perf_counter() - start_time) * 1000
                return None, elapsed_ms
            
            from py_clob_client.clob_types import MarketOrderArgs, OrderType, PartialCreateOrderOptions
            
            # 构建市价订单参数
            # side: BUY 或 SELL
            side_enum = "BUY" if side.lower() == "buy" else "SELL"
            
            logger.info(f"📤 [Polymarket] 下单请求: {side_enum} ${amount:.2f} @ token={token_id[:16]}...")
            
            # 创建市价订单参数
            order_args = MarketOrderArgs(
                token_id=token_id,
                amount=amount,
                side=side_enum,
                price=price if price is not None else 0.0,  # 市价单用 0 表示不限价
            )
            
            # 设置订单选项
            options = PartialCreateOrderOptions(tick_size=tick_size)
            
            # 1. 创建签名订单
            signed_order = client.create_market_order(order_args, options)
            
            # 2. 提交订单 (FOK = Fill or Kill)
            result = client.post_order(signed_order, OrderType.FOK)
            
            elapsed_ms = (time.perf_counter() - start_time) * 1000
            
            if result and result.get("success"):
                order_id = result.get("orderID", "unknown")
                status = result.get("status", "unknown")
                taking_amount = result.get("takingAmount", "0")
                making_amount = result.get("makingAmount", "0")
                
                logger.info(f"✅ [Polymarket] 下单成功: order_id={order_id}, status={status}, "
                           f"taking={taking_amount}, making={making_amount}, 耗时={elapsed_ms:.2f}ms")
                return result, elapsed_ms
            else:
                error_msg = result.get("errorMsg", "Unknown error") if result else "No response"
                logger.error(f"❌ [Polymarket] 下单失败: {error_msg}, 耗时={elapsed_ms:.2f}ms")
                return result, elapsed_ms
                
        except Exception as e:
            elapsed_ms = (time.perf_counter() - start_time) * 1000
            logger.error(f"❌ [Polymarket] 下单异常: {e}, 耗时={elapsed_ms:.2f}ms")
            import traceback
            traceback.print_exc()
            return None, elapsed_ms
    
    async def get_open_orders(self) -> Optional[List[Dict]]:
        """获取当前挂单列表
        
        Returns:
            订单列表，失败返回 None
        """
        try:
            # 确保有 API 凭据
            if not await self._derive_api_credentials():
                return None
            
            client = self._get_clob_client()
            if not client:
                return None
            
            orders = client.get_open_orders()
            logger.debug(f"✅ [Polymarket] 获取 {len(orders) if orders else 0} 个挂单")
            return orders
            
        except Exception as e:
            logger.error(f"❌ [Polymarket] 获取挂单失败: {e}")
            return None
    
    async def cancel_order(self, order_id: str) -> bool:
        """取消订单
        
        Args:
            order_id: 订单 ID
            
        Returns:
            是否成功取消
        """
        try:
            # 确保有 API 凭据
            if not await self._derive_api_credentials():
                return False
            
            client = self._get_clob_client()
            if not client:
                return False
            
            result = client.cancel_order(order_id)
            
            if result and order_id in result.get("canceled", []):
                logger.info(f"✅ [Polymarket] 订单已取消: {order_id}")
                return True
            else:
                not_canceled = result.get("not_canceled", {}) if result else {}
                logger.error(f"❌ [Polymarket] 取消订单失败: {not_canceled}")
                return False
                
        except Exception as e:
            logger.error(f"❌ [Polymarket] 取消订单异常: {e}")
            return False
    
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
                    # 消息类型统计
                    msg_stats = {"book": 0, "price_change": 0, "other": 0}
                    update_count = 0
                    
                    while True:
                        try:
                            message = await asyncio.wait_for(ws.recv(), timeout=30.0)
                            msg_count += 1
                            
                            # 统计消息类型
                            try:
                                msg_data = json.loads(message)
                                event_type = msg_data.get("event_type", "other")
                                if event_type in msg_stats:
                                    msg_stats[event_type] += 1
                                else:
                                    msg_stats["other"] += 1
                            except:
                                msg_stats["other"] += 1
                            
                            # 新格式可能返回多个 PriceUpdate
                            updates = self._parse_ws_message(message)
                            update_count += len(updates)
                            for update in updates:
                                on_price_update(update)
                            
                            # 每 100 条消息记录统计
                            if msg_count % 100 == 0:
                                log(f"📊 [Polymarket] 消息 #{msg_count} | book:{msg_stats['book']}, price_change:{msg_stats['price_change']}, 价格更新:{update_count}")
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
    
    async def subscribe_tokens(self, token_ids: List[str], on_log: Callable[[str], None] = None) -> bool:
        """热订阅新的 token（不关闭连接）
        
        根据 Polymarket 官方文档，连接后可以通过发送以下消息动态订阅：
        {
            "assets_ids": [...],
            "operation": "subscribe"
        }
        
        Args:
            token_ids: 要订阅的 token ID 列表
            on_log: 日志回调函数
            
        Returns:
            是否成功发送订阅请求
        """
        def log(msg: str):
            logger.info(msg)
            if on_log:
                on_log(msg)
        
        if not self.ws_connection or not self.ws_connected:
            log("⚠️ [Polymarket] WebSocket 未连接，无法热订阅")
            return False
        
        if not token_ids:
            return True
        
        # 去重
        unique_ids = list(set(token_ids))
        
        try:
            subscribe_msg = {
                "assets_ids": unique_ids,
                "operation": "subscribe"
            }
            await self.ws_connection.send(json.dumps(subscribe_msg))
            log(f"✅ [Polymarket] 热订阅 {len(unique_ids)} 个新 token")
            return True
        except Exception as e:
            log(f"❌ [Polymarket] 热订阅失败: {e}")
            return False
    
    def _parse_ws_message(self, text: str) -> List[PriceUpdate]:
        """解析 WebSocket 消息
        
        支持两种格式:
        1. book 消息 (订阅时的初始快照) - 单个 asset
        2. price_change 消息 (新格式) - 包含多个 asset 的 price_changes 数组
        
        注意: WebSocket 可能返回列表格式 [{...}] 而不是单个字典
        
        Returns:
            List[PriceUpdate]: 价格更新列表 (可能为空)
        """
        updates = []
        try:
            raw_data = json.loads(text)
            
            # 处理列表格式
            items = raw_data if isinstance(raw_data, list) else [raw_data]
            
            for data in items:
                if not isinstance(data, dict):
                    continue
                
                update = self._parse_single_message(data)
                if update:
                    updates.extend(update)
                    
        except Exception as e:
            logger.debug(f"解析 Polymarket WS 消息失败: {e}")
        
        return updates
    
    def _parse_single_message(self, data: dict) -> List[PriceUpdate]:
        """解析单个消息字典"""
        updates = []
        try:
            event_type = data.get("event_type", "")
            
            if event_type == "book":
                # book 消息：订阅时的初始订单簿快照
                # 格式: { "event_type": "book", "asset_id": "...", "bids": [...], "asks": [...] }
                # bids 按价格升序排列，最高买价在最后
                # asks 按价格降序排列，最低卖价在最后
                asset_id = data.get("asset_id", "")
                if not asset_id:
                    return updates
                
                bids = data.get("bids", [])
                asks = data.get("asks", [])
                
                yes_bid = None
                yes_ask = None
                
                # Best Bid = 最高买价 (bids 最后一个)
                if bids and len(bids) > 0:
                    try:
                        best_bid = bids[-1]
                        if isinstance(best_bid, dict):
                            yes_bid = float(best_bid.get("price", 0))
                        elif isinstance(best_bid, (list, tuple)) and len(best_bid) > 0:
                            yes_bid = float(best_bid[0])
                    except:
                        pass
                
                # Best Ask = 最低卖价 (asks 最后一个)
                if asks and len(asks) > 0:
                    try:
                        best_ask = asks[-1]
                        if isinstance(best_ask, dict):
                            yes_ask = float(best_ask.get("price", 0))
                        elif isinstance(best_ask, (list, tuple)) and len(best_ask) > 0:
                            yes_ask = float(best_ask[0])
                    except:
                        pass
                
                updates.append(PriceUpdate(
                    platform=Platform.POLYMARKET,
                    market_id=asset_id,  # token_id
                    yes_bid=yes_bid,
                    yes_ask=yes_ask,
                    no_bid=None,
                    no_ask=None,
                    timestamp=datetime.now()
                ))
            
            elif event_type == "price_change":
                # 新格式 price_change 消息：包含 price_changes 数组
                # 格式: { "event_type": "price_change", "market": "...", 
                #         "price_changes": [{ "asset_id": "...", "best_bid": "...", "best_ask": "..." }, ...] }
                price_changes = data.get("price_changes", [])
                
                for change in price_changes:
                    asset_id = change.get("asset_id", "")
                    if not asset_id:
                        continue
                    
                    # 新格式直接提供 best_bid 和 best_ask
                    best_bid = change.get("best_bid")
                    best_ask = change.get("best_ask")
                    
                    yes_bid = None
                    yes_ask = None
                    
                    # 使用 is not None 检查，因为 "0" 或 0 也是有效值
                    if best_bid is not None and best_bid != "":
                        try:
                            yes_bid = float(best_bid)
                        except:
                            pass
                    
                    if best_ask is not None and best_ask != "":
                        try:
                            yes_ask = float(best_ask)
                        except:
                            pass
                    
                    updates.append(PriceUpdate(
                        platform=Platform.POLYMARKET,
                        market_id=asset_id,  # token_id
                        yes_bid=yes_bid,
                        yes_ask=yes_ask,
                        no_bid=None,
                        no_ask=None,
                        timestamp=datetime.now()
                    ))
                    
        except Exception as e:
            logger.debug(f"解析 Polymarket WS 消息失败: {e}")
        
        return updates
