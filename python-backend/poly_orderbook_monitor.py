#!/usr/bin/env python3
"""
Polymarket 订单簿实时监控 TUI
用于分析 Polymarket WebSocket 接口数据是否正常

使用方法:
  python poly_orderbook_monitor.py                                      # 自动获取第一个可用市场
  python poly_orderbook_monitor.py --token TOKEN_A TOKEN_B              # 直接监控指定 token_id
  python poly_orderbook_monitor.py --token TOKEN_A TOKEN_B --names "Heat" "Bulls"  # 指定队伍名称
  python poly_orderbook_monitor.py --search "lakers"                    # 搜索包含关键字的市场
  python poly_orderbook_monitor.py --search "CHI" --debug               # 调试模式：详细输出原始JSON和价格对比
"""

import asyncio
import json
import time
import sys
import requests
import logging
from datetime import datetime
from typing import Dict, List, Optional, Tuple
import websockets
from rich.console import Console
from rich.live import Live
from rich.table import Table
from rich.panel import Panel
from rich.layout import Layout
from rich.text import Text

# 配置日志 - 只写入文件，不输出到控制台
LOG_FILE = "/tmp/poly_monitor.log"

# 先设置根日志级别
logging.getLogger().setLevel(logging.WARNING)

# 配置文件 handler
file_handler = logging.FileHandler(LOG_FILE, mode='w')
file_handler.setLevel(logging.INFO)
file_handler.setFormatter(logging.Formatter('%(asctime)s - %(levelname)s - %(message)s'))

# 创建自己的 logger
logger = logging.getLogger('poly_monitor')
logger.setLevel(logging.INFO)
logger.addHandler(file_handler)
logger.propagate = False  # 不传播到根 logger

# Polymarket 配置
BASE_URL = "https://gamma-api.polymarket.com"
CLOB_URL = "https://clob.polymarket.com"
WS_URL = "wss://ws-subscriptions-clob.polymarket.com/ws/market"

console = Console()


def fetch_nba_markets() -> List[dict]:
    """获取 NBA 市场列表"""
    console.print("[dim]正在获取 NBA 市场...[/dim]")
    logger.info("开始获取 NBA 市场")
    
    try:
        # 获取体育联赛
        sports_url = f"{BASE_URL}/sports"
        resp = requests.get(sports_url)
        sports = resp.json()
        
        # 找到 NBA 联赛
        nba_leagues = [
            s for s in sports 
            if 'NBA' in s.get('sport', '').upper() 
            and 'WNBA' not in s.get('sport', '').upper()
        ]
        
        if not nba_leagues:
            logger.warning("未找到 NBA 联赛")
            return []
        
        markets = []
        
        for league in nba_leagues[:1]:
            series_id = league.get('series')
            
            events_url = f"{BASE_URL}/events"
            params = {
                'series_id': str(series_id),
                'tag_id': '100639',
                'active': 'true',
                'closed': 'false',
                'limit': '20'
            }
            
            resp = requests.get(events_url, params=params)
            api_events = resp.json()
            
            logger.info(f"获取到 {len(api_events)} 个 NBA 事件")
            
            for event in api_events:
                event_title = event.get('title', '')
                event_markets = event.get('markets', [])
                
                for market_data in event_markets:
                    market_id = market_data.get('id')
                    condition_id = market_data.get('conditionId', market_id)
                    question = market_data.get('question', event_title)
                    
                    # 获取 outcomes 和 token IDs
                    outcomes_str = market_data.get('outcomes')
                    prices_str = market_data.get('outcomePrices')
                    tokens_str = market_data.get('clobTokenIds')
                    
                    if not outcomes_str or not tokens_str:
                        continue
                    
                    try:
                        outcomes = json.loads(outcomes_str)
                        tokens = json.loads(tokens_str)
                        prices = json.loads(prices_str) if prices_str else [0.5, 0.5]
                    except:
                        continue
                    
                    # 过滤 Yes/No 格式
                    if any(o.lower() == "yes" for o in outcomes):
                        continue
                    
                    # 只保留全场输赢市场
                    if question != event_title:
                        continue
                    
                    if len(outcomes) == 2 and len(tokens) == 2:
                        markets.append({
                            'condition_id': condition_id,
                            'title': event_title,
                            'question': question,
                            'team_a': outcomes[0],
                            'team_b': outcomes[1],
                            'token_a': tokens[0],
                            'token_b': tokens[1],
                            'price_a': float(prices[0]) if prices else 0.5,
                            'price_b': float(prices[1]) if prices else 0.5,
                        })
        
        return markets
        
    except Exception as e:
        logger.error(f"获取市场失败: {e}")
        console.print(f"[red]获取市场失败: {e}[/red]")
        return []


def fetch_first_market() -> Optional[Tuple[str, str, dict]]:
    """获取第一个可用市场
    
    Returns:
        (token_a, token_b, market_info) 或 None
    """
    markets = fetch_nba_markets()
    
    if not markets:
        return None
    
    market = markets[0]
    console.print(f"  [green]✓[/green] {market['title']}")
    console.print(f"  [dim]{market['team_a']} vs {market['team_b']}[/dim]")
    console.print(f"  [dim]Token A: {market['token_a'][:20]}...[/dim]")
    console.print(f"  [dim]Token B: {market['token_b'][:20]}...[/dim]")
    
    return market['token_a'], market['token_b'], market


def search_market(keyword: str) -> Optional[Tuple[str, str, dict]]:
    """搜索包含关键字的市场"""
    markets = fetch_nba_markets()
    keyword_lower = keyword.lower()
    
    for market in markets:
        if (keyword_lower in market['title'].lower() or 
            keyword_lower in market['team_a'].lower() or 
            keyword_lower in market['team_b'].lower()):
            console.print(f"  [green]✓[/green] {market['title']}")
            console.print(f"  [dim]{market['team_a']} vs {market['team_b']}[/dim]")
            return market['token_a'], market['token_b'], market
    
    console.print(f"[yellow]未找到包含 '{keyword}' 的市场[/yellow]")
    return None


class OrderbookMonitor:
    def __init__(self, debug_mode: bool = False):
        # 调试模式
        self.debug_mode = debug_mode
        # 订单簿数据: token_id -> {"bids": [...], "asks": [...]}
        self.orderbooks: Dict[str, dict] = {}
        # 价格缓存: token_id -> {"bid": x, "ask": x}
        self.prices: Dict[str, dict] = {}
        # 监控的 tokens
        self.tokens: List[str] = []
        # token 到队伍名的映射
        self.token_names: Dict[str, str] = {}
        # 市场信息
        self.market_info: Optional[dict] = None
        # 消息统计
        self.msg_count = 0
        self.book_count = 0
        self.price_change_count = 0
        self.update_count = 0
        # 更新历史
        self.update_history = []  # [(time, token_id, type, details)]
        self.start_time = datetime.now()
        # 连接状态
        self.connected = False
        self.last_msg_time = None
        
    def process_message(self, raw_data):
        """处理 WebSocket 消息
        
        注意: Polymarket WebSocket 返回的是列表 [{...}] 而不是单个字典
        """
        self.msg_count += 1
        self.last_msg_time = datetime.now()
        
        # 处理列表格式
        items = raw_data if isinstance(raw_data, list) else [raw_data]
        
        for data in items:
            if not isinstance(data, dict):
                continue
                
            event_type = data.get("event_type", "")
            
            if event_type == "book":
                self.book_count += 1
                logger.info(f"收到 book #{self.book_count}")
                self._process_book(data)
            elif event_type == "price_change":
                self.price_change_count += 1
                if self.price_change_count % 10 == 1:
                    logger.info(f"收到 price_change #{self.price_change_count}")
                self._process_price_change(data)
    
    def _process_book(self, data: dict):
        """处理 book 消息 (订阅时的初始快照)
        
        格式: { "event_type": "book", "asset_id": "...", "bids": [...], "asks": [...], "last_trade_price": "..." }
        bids/asks 格式: [{"price": "0.69", "size": "72"}, ...]
        """
        asset_id = data.get("asset_id", "")
        
        if not asset_id or (self.tokens and asset_id not in self.tokens):
            return
        
        self.update_count += 1
        
        bids = data.get("bids", [])
        asks = data.get("asks", [])
        last_trade = data.get("last_trade_price", "")
        
        # DEBUG 模式：详细输出
        if self.debug_mode:
            team_name = self.token_names.get(asset_id, "Unknown")
            print(f"\n{'='*80}")
            print(f"[BOOK] 📦 {team_name}")
            print(f"{'='*80}")
            print(f"Raw JSON keys: {list(data.keys())}")
            print(f"  Bids count: {len(bids)}")
            print(f"  Asks count: {len(asks)}")
            print(f"  last_trade_price: {last_trade}")
            
            # 分析 bids 排序
            if bids:
                bid_prices = [float(b.get("price", 0)) for b in bids if isinstance(b, dict)]
                print(f"\n  [Bids 分析]")
                print(f"    First 3 bids: {bids[:3]}")
                print(f"    Last 3 bids:  {bids[-3:]}")
                print(f"    First bid price: {bid_prices[0] if bid_prices else 'N/A'}")
                print(f"    Last bid price:  {bid_prices[-1] if bid_prices else 'N/A'}")
                print(f"    排序: {'升序 (ascending)' if bid_prices == sorted(bid_prices) else '降序 (descending)' if bid_prices == sorted(bid_prices, reverse=True) else '无序'}")
                print(f"    Best Bid (最高买价) = Last element: {bid_prices[-1]*100:.2f}¢" if bid_prices else "    No bids")
            
            # 分析 asks 排序
            if asks:
                ask_prices = [float(a.get("price", 0)) for a in asks if isinstance(a, dict)]
                print(f"\n  [Asks 分析]")
                print(f"    First 3 asks: {asks[:3]}")
                print(f"    Last 3 asks:  {asks[-3:]}")
                print(f"    First ask price: {ask_prices[0] if ask_prices else 'N/A'}")
                print(f"    Last ask price:  {ask_prices[-1] if ask_prices else 'N/A'}")
                print(f"    排序: {'升序 (ascending)' if ask_prices == sorted(ask_prices) else '降序 (descending)' if ask_prices == sorted(ask_prices, reverse=True) else '无序'}")
                print(f"    Best Ask (最低卖价) = Last element: {ask_prices[-1]*100:.2f}¢" if ask_prices else "    No asks")
        
        # 存储订单簿
        self.orderbooks[asset_id] = {
            "bids": bids.copy() if bids else [],
            "asks": asks.copy() if asks else []
        }
        
        # 计算价格
        self._calculate_prices(asset_id, bids, asks)
        
        # 记录最后成交价
        if last_trade:
            try:
                self.prices[asset_id]["last"] = float(last_trade)
            except:
                pass
        
        # DEBUG 模式：输出计算结果
        if self.debug_mode:
            prices = self.prices.get(asset_id, {})
            print(f"\n  [计算结果]")
            print(f"    Computed Best Bid: {prices.get('bid', 'N/A')}")
            print(f"    Computed Best Ask: {prices.get('ask', 'N/A')}")
        
        details = f"bids={len(bids)}, asks={len(asks)}"
        if last_trade:
            details += f", last={float(last_trade)*100:.1f}¢"
        self.update_history.append((datetime.now(), asset_id, "BOOK", details))
        
        if len(self.update_history) > 20:
            self.update_history = self.update_history[-20:]
    
    def _process_price_change(self, data: dict):
        """处理 price_change 消息
        
        格式: { "event_type": "price_change", "market": "...", 
                "price_changes": [{ "asset_id": "...", "best_bid": "...", "best_ask": "...", 
                                    "price": "...", "size": "...", "side": "BUY"|"SELL" }, ...] }
        """
        price_changes = data.get("price_changes", [])
        
        for change in price_changes:
            asset_id = change.get("asset_id", "")
            
            if not asset_id or (self.tokens and asset_id not in self.tokens):
                continue
            
            self.update_count += 1
            
            # 获取消息中的值
            msg_best_bid = change.get("best_bid")
            msg_best_ask = change.get("best_ask")
            delta_price = change.get("price")
            delta_size = change.get("size")
            delta_side = change.get("side")
            
            # 获取更新前的本地计算值
            local_bid_before = self.prices.get(asset_id, {}).get("bid")
            local_ask_before = self.prices.get(asset_id, {}).get("ask")
            
            # 应用 delta 更新到本地订单簿
            if delta_price and delta_size and delta_side:
                self._apply_orderbook_delta(asset_id, delta_price, delta_size, delta_side)
            
            # 从本地订单簿重新计算 best_bid/best_ask
            book = self.orderbooks.get(asset_id, {})
            bids = book.get("bids", [])
            asks = book.get("asks", [])
            
            local_bid_computed = None
            local_ask_computed = None
            
            if bids:
                try:
                    bid_prices = [float(b.get("price", 0)) for b in bids if isinstance(b, dict)]
                    if bid_prices:
                        local_bid_computed = max(bid_prices)  # 最高买价
                except:
                    pass
            
            if asks:
                try:
                    ask_prices = [float(a.get("price", 0)) for a in asks if isinstance(a, dict)]
                    if ask_prices:
                        local_ask_computed = min(ask_prices)  # 最低卖价
                except:
                    pass
            
            # DEBUG 模式：详细对比
            if self.debug_mode:
                team_name = self.token_names.get(asset_id, "Unknown")
                print(f"\n[PRICE_CHANGE] #{self.price_change_count} 📊 {team_name}")
                
                # 简化的 change 输出，移除冗长的 asset_id
                change_display = {k: v for k, v in change.items() if k != "asset_id"}
                print(f"  Raw: {json.dumps(change_display)}")
                
                # Delta 更新分析
                if delta_price or delta_size or delta_side:
                    print(f"  [Delta] ✓ price={delta_price}, size={delta_size}, side={delta_side}")
                else:
                    print(f"  [Delta] ✗ 无 (只有 best_bid/best_ask)")
                
                # 价格对比
                msg_bid_cents = float(msg_best_bid)*100 if msg_best_bid else None
                msg_ask_cents = float(msg_best_ask)*100 if msg_best_ask else None
                local_bid_cents = local_bid_computed*100 if local_bid_computed else None
                local_ask_cents = local_ask_computed*100 if local_ask_computed else None
                
                print(f"  [对比] 消息: bid={msg_bid_cents:.2f}¢" if msg_bid_cents else "  [对比] 消息: bid=None", end="")
                print(f", ask={msg_ask_cents:.2f}¢" if msg_ask_cents else ", ask=None")
                print(f"         本地: bid={local_bid_cents:.2f}¢" if local_bid_cents else "         本地: bid=None", end="")
                print(f", ask={local_ask_cents:.2f}¢" if local_ask_cents else ", ask=None")
                
                # 差异检测
                if msg_ask_cents and local_ask_cents:
                    diff = abs(msg_ask_cents - local_ask_cents)
                    if diff > 0.1:
                        print(f"  ⚠️  ASK 不匹配! 消息={msg_ask_cents:.2f}¢ vs 本地={local_ask_cents:.2f}¢ (差={diff:.2f}¢)")
                
                if msg_bid_cents and local_bid_cents:
                    diff = abs(msg_bid_cents - local_bid_cents)
                    if diff > 0.1:
                        print(f"  ⚠️  BID 不匹配! 消息={msg_bid_cents:.2f}¢ vs 本地={local_bid_cents:.2f}¢ (差={diff:.2f}¢)")
            
            # 更新价格缓存 (使用本地计算的值而不是消息中的值)
            if asset_id not in self.prices:
                self.prices[asset_id] = {"bid": None, "ask": None}
            
            # 优先使用本地计算值，如果本地订单簿为空则用消息值
            if local_bid_computed is not None:
                self.prices[asset_id]["bid"] = local_bid_computed
            elif msg_best_bid is not None and msg_best_bid != "":
                try:
                    self.prices[asset_id]["bid"] = float(msg_best_bid)
                except:
                    pass
            
            if local_ask_computed is not None:
                self.prices[asset_id]["ask"] = local_ask_computed
            elif msg_best_ask is not None and msg_best_ask != "":
                try:
                    self.prices[asset_id]["ask"] = float(msg_best_ask)
                except:
                    pass
            
            bid_str = f"{self.prices[asset_id].get('bid', 0)*100:.1f}¢" if self.prices[asset_id].get('bid') else "-"
            ask_str = f"{self.prices[asset_id].get('ask', 0)*100:.1f}¢" if self.prices[asset_id].get('ask') else "-"
            details = f"bid={bid_str}, ask={ask_str}"
            
            self.update_history.append((datetime.now(), asset_id, "PRICE", details))
        
        if len(self.update_history) > 20:
            self.update_history = self.update_history[-20:]
    
    def _apply_orderbook_delta(self, asset_id: str, price_str: str, size_str: str, side: str):
        """应用订单簿增量更新
        
        根据 Polymarket 文档:
        - side=BUY: 更新 bids
        - side=SELL: 更新 asks
        - size=0: 删除该价格层级
        """
        try:
            price = float(price_str)
            size = float(size_str)
        except (ValueError, TypeError):
            return
        
        if asset_id not in self.orderbooks:
            self.orderbooks[asset_id] = {"bids": [], "asks": []}
        
        book = self.orderbooks[asset_id]
        
        if side.upper() == "BUY":
            book_side = book["bids"]
        else:
            book_side = book["asks"]
        
        # 查找该价格层级
        found_idx = None
        for i, entry in enumerate(book_side):
            if isinstance(entry, dict):
                entry_price = float(entry.get("price", 0))
                if abs(entry_price - price) < 0.0001:
                    found_idx = i
                    break
        
        if size <= 0:
            # 删除层级
            if found_idx is not None:
                book_side.pop(found_idx)
        else:
            if found_idx is not None:
                # 更新现有层级
                book_side[found_idx]["size"] = str(size)
            else:
                # 插入新层级
                book_side.append({"price": str(price), "size": str(size)})
    
    def _calculate_prices(self, asset_id: str, bids: list, asks: list):
        """从订单簿计算最佳买卖价
        
        bids/asks 格式: [{"price": "0.69", "size": "72"}, ...]
        bids 按价格升序排列，最后一个是最高买价
        asks 按价格降序排列，最后一个是最低卖价
        """
        if asset_id not in self.prices:
            self.prices[asset_id] = {"bid": None, "ask": None}
        
        # Best Bid = 最高买价 (bids 最后一个)
        if bids and len(bids) > 0:
            try:
                # bids 按价格升序，最后一个是最高买价
                best_bid = bids[-1]
                if isinstance(best_bid, dict):
                    self.prices[asset_id]["bid"] = float(best_bid.get("price", 0))
                elif isinstance(best_bid, (list, tuple)) and len(best_bid) > 0:
                    self.prices[asset_id]["bid"] = float(best_bid[0])
            except:
                pass
        
        # Best Ask = 最低卖价 (asks 最后一个)
        if asks and len(asks) > 0:
            try:
                # asks 按价格降序，最后一个是最低卖价
                best_ask = asks[-1]
                if isinstance(best_ask, dict):
                    self.prices[asset_id]["ask"] = float(best_ask.get("price", 0))
                elif isinstance(best_ask, (list, tuple)) and len(best_ask) > 0:
                    self.prices[asset_id]["ask"] = float(best_ask[0])
            except:
                pass
    
    def create_display(self) -> Layout:
        """创建 TUI 显示布局"""
        layout = Layout()
        
        layout.split_column(
            Layout(name="header", size=3),
            Layout(name="body"),
            Layout(name="footer", size=10)
        )
        
        layout["body"].split_row(
            Layout(name="orderbook", ratio=2),
            Layout(name="stats", ratio=1)
        )
        
        # Header
        runtime = datetime.now() - self.start_time
        status = "[green]已连接[/green]" if self.connected else "[red]未连接[/red]"
        last_update = self.last_msg_time.strftime("%H:%M:%S.%f")[:-3] if self.last_msg_time else "N/A"
        
        header_text = Text()
        header_text.append("Polymarket 订单簿监控 ", style="bold cyan")
        header_text.append(f"| 状态: {status} ", style="")
        header_text.append(f"| 运行时间: {str(runtime).split('.')[0]} ", style="dim")
        header_text.append(f"| 最后更新: {last_update} ", style="dim")
        header_text.append(f"| 日志: {LOG_FILE}", style="dim yellow")
        
        layout["header"].update(Panel(header_text, style="blue"))
        
        # Orderbook
        orderbook_table = self._create_orderbook_table()
        layout["orderbook"].update(Panel(orderbook_table, title="📊 订单簿", border_style="green"))
        
        # Stats
        stats_table = self._create_stats_table()
        layout["stats"].update(Panel(stats_table, title="📈 统计", border_style="yellow"))
        
        # Footer - 更新历史
        history_table = self._create_history_table()
        layout["footer"].update(Panel(history_table, title="📝 最近更新", border_style="magenta"))
        
        return layout
    
    def _create_orderbook_table(self) -> Table:
        """创建订单簿表格"""
        table = Table(show_header=True, header_style="bold")
        table.add_column("队伍", style="cyan", width=20)
        table.add_column("Bid", justify="right", style="green", width=10)
        table.add_column("Ask", justify="right", style="red", width=10)
        table.add_column("Spread", justify="right", style="yellow", width=10)
        table.add_column("Last", justify="right", style="magenta", width=10)
        table.add_column("深度", justify="right", width=10)
        
        # 使用监控的 tokens
        tokens_to_show = self.tokens if self.tokens else list(self.prices.keys())
        
        for token_id in tokens_to_show:
            prices = self.prices.get(token_id, {})
            book = self.orderbooks.get(token_id, {})
            
            bid = prices.get("bid")
            ask = prices.get("ask")
            last = prices.get("last")
            
            bids_depth = len(book.get("bids", []))
            asks_depth = len(book.get("asks", []))
            
            # 计算 spread
            spread = None
            if bid is not None and ask is not None:
                spread = ask - bid
            
            # 获取队伍名
            team_name = self.token_names.get(token_id, token_id[:12] + "...")
            
            table.add_row(
                team_name,
                f"{bid*100:.1f}¢" if bid else "-",
                f"{ask*100:.1f}¢" if ask else "-",
                f"{spread*100:.2f}¢" if spread else "-",
                f"{last*100:.1f}¢" if last else "-",
                f"{bids_depth}/{asks_depth}"
            )
        
        return table
    
    def _create_stats_table(self) -> Table:
        """创建统计表格"""
        table = Table(show_header=False, box=None)
        table.add_column("指标", style="dim")
        table.add_column("值", justify="right", style="bold")
        
        table.add_row("总消息", str(self.msg_count))
        table.add_row("Book", str(self.book_count))
        table.add_row("PriceChange", str(self.price_change_count))
        table.add_row("目标更新", str(self.update_count))
        
        # 计算更新频率
        runtime = (datetime.now() - self.start_time).total_seconds()
        if runtime > 0:
            freq = self.update_count / runtime
            table.add_row("更新频率", f"{freq:.2f}/秒")
        
        # 显示市场信息
        if self.market_info:
            table.add_row("", "")
            table.add_row("市场", self.market_info.get('title', '')[:20])
            price_a = self.market_info.get('price_a', 0)
            price_b = self.market_info.get('price_b', 0)
            table.add_row("初始价格", f"{price_a*100:.0f}¢ / {price_b*100:.0f}¢")
        
        return table
    
    def _create_history_table(self) -> Table:
        """创建更新历史表格"""
        table = Table(show_header=True, header_style="bold dim")
        table.add_column("时间", width=12)
        table.add_column("队伍", width=15)
        table.add_column("类型", width=10)
        table.add_column("详情", width=25)
        
        for ts, token_id, update_type, details in reversed(self.update_history[-8:]):
            time_str = ts.strftime("%H:%M:%S.%f")[:-3]
            team_name = self.token_names.get(token_id, token_id[:10] + "...")
            
            type_style = "yellow" if update_type == "BOOK" else "cyan"
            
            table.add_row(
                time_str,
                team_name,
                f"[{type_style}]{update_type}[/{type_style}]",
                details
            )
        
        return table


async def main():
    logger.info("="*60)
    logger.info("Polymarket 订单簿监控启动")
    logger.info(f"日志文件: {LOG_FILE}")
    logger.info("="*60)
    
    # 解析命令行参数
    args = sys.argv[1:]
    token_a = None
    token_b = None
    market_info = None
    debug_mode = False
    custom_names = []
    
    # 检查是否有 --debug 标志
    if "--debug" in args:
        debug_mode = True
        args = [a for a in args if a != "--debug"]
        console.print("[bold yellow]🔍 调试模式已启用 - 将输出详细的原始JSON和价格对比[/bold yellow]\n")
    
    # 检查是否有 --names 参数
    if "--names" in args:
        names_idx = args.index("--names")
        # 获取 --names 后面的名称
        remaining = args[names_idx + 1:]
        for name in remaining:
            if name.startswith("--"):
                break
            custom_names.append(name)
        # 移除 --names 和其参数
        args = args[:names_idx]
    
    if args:
        if args[0] == "--help" or args[0] == "-h":
            console.print(__doc__)
            return
        elif args[0] == "--token":
            if len(args) < 2:
                console.print("[red]错误: --token 需要指定 token_id[/red]")
                return
            token_a = args[1]
            token_b = args[2] if len(args) > 2 else None
            logger.info(f"使用指定 token: {token_a}")
            console.print(f"[dim]监控指定 token:[/dim]")
            console.print(f"  [green]✓[/green] {token_a[:30]}...")
            if token_b:
                console.print(f"  [green]✓[/green] {token_b[:30]}...")
        elif args[0] == "--search":
            if len(args) < 2:
                console.print("[red]错误: --search 需要指定关键字[/red]")
                return
            result = search_market(args[1])
            if result:
                token_a, token_b, market_info = result
            else:
                return
    
    # 如果没有指定 token，自动获取第一个可用市场
    if not token_a:
        result = fetch_first_market()
        if result:
            token_a, token_b, market_info = result
    
    if not token_a:
        console.print("[red]未找到可用市场，退出[/red]")
        return
    
    # 构建 token 列表
    tokens = [token_a]
    if token_b:
        tokens.append(token_b)
    
    console.print(f"\n[bold cyan]开始监控 {len(tokens)} 个 token...[/bold cyan]")
    if debug_mode:
        console.print(f"[dim]Token A: {token_a}[/dim]")
        console.print(f"[dim]Token B: {token_b}[/dim]")
    console.print(f"[dim]日志文件: {LOG_FILE}[/dim]\n")
    
    monitor = OrderbookMonitor(debug_mode=debug_mode)
    monitor.tokens = tokens
    monitor.market_info = market_info
    
    # 设置 token 名称映射
    if custom_names:
        # 使用用户自定义名称
        monitor.token_names[token_a] = custom_names[0] if len(custom_names) > 0 else 'Team A'
        if token_b:
            monitor.token_names[token_b] = custom_names[1] if len(custom_names) > 1 else 'Team B'
    elif market_info:
        monitor.token_names[token_a] = market_info.get('team_a', 'Team A')
        if token_b:
            monitor.token_names[token_b] = market_info.get('team_b', 'Team B')
    else:
        # 直接指定 token 时，使用简单名称
        monitor.token_names[token_a] = 'Team A'
        if token_b:
            monitor.token_names[token_b] = 'Team B'
    
    async def connect_and_monitor():
        retry_count = 0
        while True:
            try:
                retry_count += 1
                logger.info(f"连接 WebSocket (尝试 #{retry_count})...")
                
                async with websockets.connect(WS_URL, open_timeout=30) as ws:
                    monitor.connected = True
                    logger.info("✅ WebSocket 连接成功")
                    
                    # 等待连接确认
                    try:
                        msg = await asyncio.wait_for(ws.recv(), timeout=10)
                        data = json.loads(msg)
                        if data.get("event_type") == "connected":
                            logger.info("✅ 连接已确认")
                    except asyncio.TimeoutError:
                        logger.warning("⚠️ 未收到连接确认，继续...")
                    
                    # 订阅市场
                    subscribe_msg = {
                        "assets_ids": tokens,
                        "type": "market"
                    }
                    logger.info(f"订阅 tokens: {tokens}")
                    await ws.send(json.dumps(subscribe_msg))
                    
                    # 接收消息
                    while True:
                        message = await ws.recv()
                        data = json.loads(message)
                        monitor.process_message(data)
                        
            except websockets.ConnectionClosed as e:
                monitor.connected = False
                logger.warning(f"WebSocket 连接关闭: {e}, 5秒后重连...")
                await asyncio.sleep(5)
            except Exception as e:
                monitor.connected = False
                logger.error(f"WebSocket 错误: {type(e).__name__}: {e}")
                await asyncio.sleep(5)
    
    # 启动 WebSocket 连接
    ws_task = asyncio.create_task(connect_and_monitor())
    
    # 等待一秒让连接建立
    await asyncio.sleep(1)
    
    if debug_mode:
        # 调试模式：直接输出到控制台，不使用 TUI
        console.print("[bold green]✅ 调试模式运行中，按 Ctrl+C 退出[/bold green]")
        console.print("[dim]等待 WebSocket 消息...[/dim]\n")
        try:
            while True:
                await asyncio.sleep(1)
        except KeyboardInterrupt:
            console.print("\n[yellow]正在关闭...[/yellow]")
            ws_task.cancel()
    else:
        # 正常模式：启动 TUI 显示
        with Live(monitor.create_display(), refresh_per_second=4, console=console) as live:
            try:
                while True:
                    await asyncio.sleep(0.25)
                    live.update(monitor.create_display())
            except KeyboardInterrupt:
                console.print("\n[yellow]正在关闭...[/yellow]")
                ws_task.cancel()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        console.print("\n[yellow]已退出[/yellow]")
