#!/usr/bin/env python3
"""
Kalshi 订单簿实时监控 TUI
用于分析 Kalshi WebSocket 接口数据是否正常

使用方法:
  python kalshi_orderbook_monitor.py                                       # 自动获取第一个可用市场
  python kalshi_orderbook_monitor.py --ticker KXNBAGAME-26JAN06DALSAC-DAL  # 直接监控指定 ticker
"""

import asyncio
import json
import time
import sys
import base64
import requests
import logging
from datetime import datetime
from typing import Dict, List, Optional
import websockets
from rich.console import Console
from rich.live import Live
from rich.table import Table
from rich.panel import Panel
from rich.layout import Layout
from rich.text import Text
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.backends import default_backend

# 配置日志 - 只写入文件，不输出到控制台
LOG_FILE = "/tmp/kalshi_monitor.log"

# 先设置根日志级别
logging.getLogger().setLevel(logging.WARNING)

# 配置文件 handler
file_handler = logging.FileHandler(LOG_FILE, mode='w')
file_handler.setLevel(logging.INFO)
file_handler.setFormatter(logging.Formatter('%(asctime)s - %(levelname)s - %(message)s'))

# 创建自己的 logger
logger = logging.getLogger('kalshi_monitor')
logger.setLevel(logging.INFO)
logger.addHandler(file_handler)
logger.propagate = False  # 不传播到根 logger

# Kalshi 配置
API_KEY = "775ab5b1-d017-437b-9d51-4b62636a6df8"
API_SECRET = """-----BEGIN RSA PRIVATE KEY-----
MIIEogIBAAKCAQEAsJ6PcidUgrVIFc6JVdeWVy63Ti2+g38kfkyNkh62PH+0Md5p
mRoAlMZPis16hXb86PoNI2DVQxvVWlaZhUe3pRvi0C7WnHRNPgE5oJuOmQYTxCsE
dppr3MjW7CVqmCAkmiewFCVNd6M8T4cUJUWVTwAWDC4AX6WzCfGsCuP3MiYh4Q29
/jKxon4HE2jjp+VVnJV4ak5J0Bk9bgYS1rgzC5xUtcMmLUwLktt9pi2iTS1BploP
F4YT+f2pPURjAYWHvLxdCILPMxhzRz7/gwQJjK0oBx79JNtz/6r9jEAxNjWvfsN/
XS8jCPIWCmHFUAB52zu6rBwRINd/YMNJjoJLtwIDAQABAoIBADqpP02T4I7VNQ8B
HT4kr6tfOcS+jsNQDEfilzbL+o0XgTr6+ym9qCnBwBOC8cm4XoHm40w30j1K1k79
6lmJG2a8X1Xz6JXoTutkLsa4Q+FxUklaXE4kNeUL0851EvVZ365PtjzKsvMkhViI
rdk/RjsJ9UiwUrFx8IhB+3lWg/jkgZn5NxLdNqPb6Iql92urObwO/HQpL3SHtBTB
siTsmmTv1ov3HxLU9n2f6/UJF9GIh1uhDLrp/JuHdFMha/BoKMm9IDf+yF5P+7+t
VESHoqFAXCZW8YkE9MRnqiMyF/X41EBnOiRLl6U4nD6iAILBmXXcWGERa0kEu0AT
jGdWyOkCgYEAwreY2S6vp7s/MgEr4+e7CP+6vD6ylpnWkO85S208lqHyW9rlSWVF
j1Lzj30XJiFUkJHoQhSQ3VtiL1UADUOqgyrPN+CNgircrpphOmUv2H9QejhOw6hO
PQtX2Q8qOxsP/dpSl1qBpfF/Atje0GBZVDmnZKXWokLJbuip04iqedMCgYEA6DTR
4AXhFbjgn2NJ4F1yveb/ooGbumOz6ZyFu32t6KECFEiin0i2gd1MLG9h3M2sM5DA
sFnBvc4n8EkuDxWCL3CVzBnYV1ttFMJmTZ2NuuQ79JCq5ZinicJA4hmP6JSIwOfB
QyPDqJ73tYzSCRjiqp5G4zWihEFgMAyqcawv9A0CgYAH9CHtNSf9rPte2odlbtDI
BFInYFRBLJWEJBFuEE86Q/E3aVCWV+moehXy1YQ9jXz7zsny8Q4dzJ7NGia/Y/Uy
GGyQdr6xA3kyNKImFX4H1ON/qo8lFNnlSnJTLnhmI6vajbFz90F2es/aWOt5DYZ4
l0ZMKS4EQMAjpKNTPbDg+wKBgD6vY6jybV0L17evehYTpBIX9hLMolmi6MK7+m4u
82/FZ2ZKZXxSiNJuB05Hk0ekTkRtY1c4I9E1ghqf5sszpz1fS1EZ+Qk8KSpdgHib
e9NkIbtYAoqQt0m9Iv7mn67Nyk0pQ0b4tK0knQJpzZmfGjGtIL3dkM3bSDgwcyLU
tO1RAoGAcPAzBaK9CFZXKfnN/XTFx88NbxR1wFSpBxx5IJtKwVTpq8aOU+Dvzjes
e/VQGYOnvZAAXr4qQrDUXutA5LqwOOHprwasYcYc6YMhuDfS1yroOsDcOkr2bqws
5cVIGehYzyPRyzlwGiQwjCoOFOXDrj7QjHbYMpTWzCoL0ONMbAA=
-----END RSA PRIVATE KEY-----"""

BASE_URL = "https://api.elections.kalshi.com/trade-api/v2"
WS_URL = "wss://api.elections.kalshi.com/trade-api/ws/v2"

console = Console()


def fetch_first_market() -> Optional[str]:
    """获取第一个可用的 NBA 市场 ticker"""
    console.print("[dim]正在获取可用市场...[/dim]")
    logger.info("开始获取第一个可用市场")
    
    try:
        # 获取 NBA 事件
        url = f"{BASE_URL}/events"
        params = {"series_ticker": "KXNBAGAME", "status": "open", "limit": 5}
        resp = requests.get(url, params=params)
        data = resp.json()
        
        events = data.get("events", [])
        logger.info(f"获取到 {len(events)} 个事件")
        
        for event in events:
            event_ticker = event.get("event_ticker", "")
            title = event.get("title", "")
            
            # 获取市场
            markets_url = f"{BASE_URL}/markets"
            m_params = {"event_ticker": event_ticker}
            m_resp = requests.get(markets_url, params=m_params)
            m_data = m_resp.json()
            
            markets = m_data.get("markets", [])
            if markets:
                ticker = markets[0].get("ticker", "")
                if ticker:
                    console.print(f"  [green]✓[/green] {ticker}")
                    console.print(f"  [dim]{title}[/dim]")
                    logger.info(f"选择市场: {ticker} ({title})")
                    return ticker
        
        return None
        
    except Exception as e:
        logger.error(f"获取市场失败: {e}")
        console.print(f"[red]获取市场失败: {e}[/red]")
        return None


class OrderbookMonitor:
    def __init__(self):
        # 订单簿数据: ticker -> {"yes": [...], "no": [...]}
        self.orderbooks: Dict[str, dict] = {}
        # 价格缓存: ticker -> {"yes_bid": x, "no_bid": x, "yes_ask": x, "no_ask": x}
        self.prices: Dict[str, dict] = {}
        # 监控的 tickers
        self.tickers: List[str] = []
        # 消息统计
        self.msg_count = 0
        self.snapshot_count = 0
        self.delta_count = 0
        self.update_count = 0
        # 更新历史
        self.update_history = []  # [(time, ticker, type, details)]
        self.start_time = datetime.now()
        # 连接状态
        self.connected = False
        self.last_msg_time = None
        
    def _sign_request(self, timestamp_ms: int, method: str, path: str) -> str:
        """生成 Kalshi API 签名 - 使用 PSS padding"""
        message = f"{timestamp_ms}{method}{path}"
        
        private_key = serialization.load_pem_private_key(
            API_SECRET.encode(),
            password=None,
            backend=default_backend()
        )
        
        # 使用 PSS padding (与主程序保持一致)
        signature = private_key.sign(
            message.encode(),
            padding.PSS(
                mgf=padding.MGF1(hashes.SHA256()),
                salt_length=padding.PSS.MAX_LENGTH
            ),
            hashes.SHA256()
        )
        
        return base64.b64encode(signature).decode()
    
    def process_message(self, data: dict):
        """处理 WebSocket 消息"""
        self.msg_count += 1
        self.last_msg_time = datetime.now()
        
        msg_type = data.get("type", "")
        
        if msg_type == "orderbook_snapshot":
            self.snapshot_count += 1
            logger.info(f"收到 snapshot #{self.snapshot_count}")
            self._process_orderbook(data, is_snapshot=True)
        elif msg_type == "orderbook_delta":
            self.delta_count += 1
            # 每 10 次记录一次
            if self.delta_count % 10 == 1:
                logger.info(f"收到 delta #{self.delta_count}")
            self._process_orderbook(data, is_snapshot=False)
    
    def _process_orderbook(self, data: dict, is_snapshot: bool):
        """处理订单簿消息
        
        Snapshot 消息格式:
            {"yes": [[price, qty], ...], "no": [[price, qty], ...]}
        
        Delta 消息格式:
            {"price": 31, "delta": 125, "side": "yes", "ts": "..."}
        """
        msg = data.get("msg", {})
        ticker = msg.get("market_ticker", "")
        
        if not ticker or (self.tickers and ticker not in self.tickers):
            return
        
        self.update_count += 1
        
        # 初始化订单簿
        if ticker not in self.orderbooks:
            self.orderbooks[ticker] = {"yes": [], "no": []}
        
        if is_snapshot:
            # Snapshot: 直接替换整个订单簿
            yes_data = msg.get("yes", [])
            no_data = msg.get("no", [])
            self.orderbooks[ticker]["yes"] = yes_data.copy()
            self.orderbooks[ticker]["no"] = no_data.copy()
            details = f"yes={len(yes_data)}, no={len(no_data)}"
        else:
            # Delta: 单个价格变化
            price = msg.get("price")  # 价格 (美分)
            delta = msg.get("delta")  # 数量变化 (正数增加，负数减少)
            side = msg.get("side")    # "yes" 或 "no"
            
            if price is not None and delta is not None and side:
                self._apply_delta(ticker, side, price, delta)
                details = f"{side}@{price}¢ Δ{delta:+d}"
            else:
                details = "无效 delta"
        
        # 计算价格
        self._calculate_prices(ticker)
        
        # 记录更新历史
        update_type = "SNAPSHOT" if is_snapshot else "DELTA"
        self.update_history.append((datetime.now(), ticker, update_type, details))
        
        # 只保留最近 20 条
        if len(self.update_history) > 20:
            self.update_history = self.update_history[-20:]
    
    def _apply_delta(self, ticker: str, side: str, price: int, delta: int):
        """应用单个 delta 更新到订单簿
        
        Args:
            ticker: 市场 ticker
            side: "yes" 或 "no"
            price: 价格 (美分)
            delta: 数量变化 (正数增加，负数减少)
        """
        book = self.orderbooks[ticker][side]
        
        # 查找该价格
        found_idx = -1
        for i, entry in enumerate(book):
            if entry[0] == price:
                found_idx = i
                break
        
        if found_idx >= 0:
            # 找到了，更新数量
            new_qty = book[found_idx][1] + delta
            if new_qty <= 0:
                # 数量为0或负数，删除该档位
                book.pop(found_idx)
            else:
                book[found_idx][1] = new_qty
        else:
            # 没找到，如果是正数 delta 则添加
            if delta > 0:
                book.append([price, delta])
                # 按价格排序
                book.sort(key=lambda x: x[0])
    
    def _calculate_prices(self, ticker: str):
        """计算最佳买卖价"""
        book = self.orderbooks.get(ticker, {})
        yes_data = book.get("yes", [])
        no_data = book.get("no", [])
        
        # 初始化价格
        if ticker not in self.prices:
            self.prices[ticker] = {
                "yes_bid": None, "yes_ask": None,
                "no_bid": None, "no_ask": None
            }
        
        # Best Bid = 最高买价 (列表最后一个)
        if yes_data:
            self.prices[ticker]["yes_bid"] = yes_data[-1][0] / 100.0
        if no_data:
            self.prices[ticker]["no_bid"] = no_data[-1][0] / 100.0
        
        # 计算 Ask 价格
        yes_bid = self.prices[ticker].get("yes_bid")
        no_bid = self.prices[ticker].get("no_bid")
        
        if no_bid is not None:
            self.prices[ticker]["yes_ask"] = 1.0 - no_bid
        if yes_bid is not None:
            self.prices[ticker]["no_ask"] = 1.0 - yes_bid
    
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
        header_text.append("Kalshi 订单簿监控 ", style="bold cyan")
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
        table.add_column("市场", style="cyan", width=12)
        table.add_column("Yes Bid", justify="right", style="green", width=8)
        table.add_column("Yes Ask", justify="right", style="red", width=8)
        table.add_column("No Bid", justify="right", style="green", width=8)
        table.add_column("No Ask", justify="right", style="red", width=8)
        table.add_column("深度", justify="right", width=8)
        
        # 使用实际收到数据的 tickers，或者监控的 tickers
        tickers_to_show = self.tickers if self.tickers else list(self.orderbooks.keys())
        
        for ticker in tickers_to_show:
            prices = self.prices.get(ticker, {})
            book = self.orderbooks.get(ticker, {})
            
            yes_bid = prices.get("yes_bid")
            yes_ask = prices.get("yes_ask")
            no_bid = prices.get("no_bid")
            no_ask = prices.get("no_ask")
            
            yes_depth = len(book.get("yes", []))
            no_depth = len(book.get("no", []))
            
            # 格式化显示 - 只显示队伍名
            parts = ticker.split("-")
            team = parts[-1] if len(parts) > 1 else ticker[:10]
            
            table.add_row(
                team,
                f"{yes_bid*100:.0f}¢" if yes_bid else "-",
                f"{yes_ask*100:.0f}¢" if yes_ask else "-",
                f"{no_bid*100:.0f}¢" if no_bid else "-",
                f"{no_ask*100:.0f}¢" if no_ask else "-",
                f"{yes_depth}/{no_depth}"
            )
        
        return table
    
    def _create_stats_table(self) -> Table:
        """创建统计表格"""
        table = Table(show_header=False, box=None)
        table.add_column("指标", style="dim")
        table.add_column("值", justify="right", style="bold")
        
        table.add_row("总消息", str(self.msg_count))
        table.add_row("Snapshot", str(self.snapshot_count))
        table.add_row("Delta", str(self.delta_count))
        table.add_row("目标更新", str(self.update_count))
        
        # 计算更新频率
        runtime = (datetime.now() - self.start_time).total_seconds()
        if runtime > 0:
            freq = self.update_count / runtime
            table.add_row("更新频率", f"{freq:.2f}/秒")
        
        return table
    
    def _create_history_table(self) -> Table:
        """创建更新历史表格"""
        table = Table(show_header=True, header_style="bold dim")
        table.add_column("时间", width=12)
        table.add_column("市场", width=8)
        table.add_column("类型", width=10)
        table.add_column("详情", width=20)
        
        for ts, ticker, update_type, details in reversed(self.update_history[-8:]):
            time_str = ts.strftime("%H:%M:%S.%f")[:-3]
            team = ticker.split("-")[-1]
            
            type_style = "yellow" if update_type == "SNAPSHOT" else "cyan"
            
            table.add_row(
                time_str,
                team,
                f"[{type_style}]{update_type}[/{type_style}]",
                details
            )
        
        return table


async def main():
    logger.info("="*60)
    logger.info("Kalshi 订单簿监控启动")
    logger.info(f"日志文件: {LOG_FILE}")
    logger.info("="*60)
    
    # 解析命令行参数
    args = sys.argv[1:]
    ticker = None
    
    if args:
        if args[0] == "--help" or args[0] == "-h":
            console.print(__doc__)
            return
        elif args[0] == "--ticker":
            if len(args) < 2:
                console.print("[red]错误: --ticker 需要指定 ticker 名称[/red]")
                return
            ticker = args[1].upper()  # 自动转换为大写
            logger.info(f"使用指定 ticker: {ticker}")
            console.print(f"[dim]监控指定 ticker:[/dim]")
            console.print(f"  [green]✓[/green] {ticker}")
    
    # 如果没有指定 ticker，自动获取第一个可用市场
    if not ticker:
        ticker = fetch_first_market()
    
    if not ticker:
        console.print("[red]未找到可用市场，退出[/red]")
        return
    
    tickers = [ticker]  # 只监控一个市场
    console.print(f"\n[bold cyan]开始监控...[/bold cyan]")
    console.print(f"[dim]日志文件: {LOG_FILE}[/dim]\n")
    
    monitor = OrderbookMonitor()
    monitor.tickers = tickers  # 保存 tickers 到 monitor
    
    async def connect_and_monitor():
        retry_count = 0
        while True:
            try:
                retry_count += 1
                # 生成认证签名
                timestamp_ms = int(time.time() * 1000)
                path = "/trade-api/ws/v2"
                logger.info(f"连接 WebSocket (尝试 #{retry_count})...")
                signature = monitor._sign_request(timestamp_ms, "GET", path)
                
                headers = {
                    "KALSHI-ACCESS-KEY": API_KEY,
                    "KALSHI-ACCESS-SIGNATURE": signature,
                    "KALSHI-ACCESS-TIMESTAMP": str(timestamp_ms),
                }
                
                async with websockets.connect(WS_URL, extra_headers=headers, open_timeout=30) as ws:
                    monitor.connected = True
                    logger.info("✅ WebSocket 连接成功")
                    
                    # 订阅市场
                    subscribe_msg = {
                        "id": 1,
                        "cmd": "subscribe",
                        "params": {
                            "channels": ["orderbook_delta"],
                            "market_ticker": ticker
                        }
                    }
                    logger.info(f"订阅市场: {ticker}")
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
    
    # 启动 TUI 显示
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
