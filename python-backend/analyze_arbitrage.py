#!/usr/bin/env python3
"""分析套利机会的详细情况"""
import asyncio
import logging
from config import Config
from kalshi_client import KalshiClient
from polymarket_client import PolymarketClient
from matcher import EventMatcher
from calculator import ArbitrageCalculator

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


async def analyze():
    config = Config.from_file("../config.toml")
    
    # 获取市场数据
    kalshi_client = KalshiClient(config.kalshi)
    await kalshi_client.login()
    kalshi_markets = await kalshi_client.get_nba_markets()
    
    polymarket_client = PolymarketClient(config.polymarket)
    await polymarket_client.initialize_team_mappings()
    polymarket_markets = await polymarket_client.get_nba_markets()
    
    print("=" * 80)
    print("数据统计")
    print("=" * 80)
    print(f"Kalshi 市场: {len(kalshi_markets)} 个")
    print(f"Polymarket 市场: {len(polymarket_markets)} 个")
    
    # 分析 Kalshi 市场结构
    print("\n" + "=" * 80)
    print("Kalshi 市场分析")
    print("=" * 80)
    
    # 按赛事分组
    kalshi_by_event = {}
    for market in kalshi_markets:
        event = market.event_name
        if event not in kalshi_by_event:
            kalshi_by_event[event] = []
        kalshi_by_event[event].append(market)
    
    print(f"Kalshi 赛事数: {len(kalshi_by_event)}")
    print(f"示例赛事:")
    for event, markets in list(kalshi_by_event.items())[:3]:
        print(f"\n  赛事: {event}")
        print(f"  市场数: {len(markets)}")
        for m in markets:
            print(f"    - {m.market_id}: {m.team_a} ({m.price_a:.2f}) vs {m.team_b} ({m.price_b:.2f})")
    
    # 分析 Polymarket 市场结构
    print("\n" + "=" * 80)
    print("Polymarket 市场分析")
    print("=" * 80)
    
    polymarket_by_event = {}
    for market in polymarket_markets:
        event = market.event_name
        if event not in polymarket_by_event:
            polymarket_by_event[event] = []
        polymarket_by_event[event].append(market)
    
    print(f"Polymarket 赛事数: {len(polymarket_by_event)}")
    print(f"示例赛事:")
    for event, markets in list(polymarket_by_event.items())[:3]:
        print(f"\n  赛事: {event}")
        print(f"  市场数: {len(markets)}")
        for m in markets:
            print(f"    - {m.market_id}: {m.team_a} ({m.price_a:.2f}) vs {m.team_b} ({m.price_b:.2f})")
    
    # 匹配分析
    print("\n" + "=" * 80)
    print("匹配分析")
    print("=" * 80)
    
    matcher = EventMatcher()
    matched_events = matcher.match_events(kalshi_markets, polymarket_markets)
    
    print(f"匹配的赛事: {len(matched_events)}")
    print(f"\n匹配详情:")
    for event in matched_events[:5]:
        print(f"\n  赛事: {event.event_name} (置信度: {event.confidence:.2f})")
        if event.kalshi_market:
            print(f"    Kalshi: {event.kalshi_market.team_a} vs {event.kalshi_market.team_b}")
            print(f"            价格: {event.kalshi_market.price_a:.2f} / {event.kalshi_market.price_b:.2f}")
        if event.polymarket_market:
            print(f"    Polymarket: {event.polymarket_market.team_a} vs {event.polymarket_market.team_b}")
            print(f"                价格: {event.polymarket_market.price_a:.2f} / {event.polymarket_market.price_b:.2f}")
    
    # 套利计算分析
    print("\n" + "=" * 80)
    print("套利计算分析")
    print("=" * 80)
    
    calculator = ArbitrageCalculator(
        min_profit_margin=config.settings.min_profit_margin,
        default_bet_amount=config.settings.default_bet_amount
    )
    opportunities = calculator.calculate_arbitrage(matched_events)
    
    print(f"总套利机会: {len(opportunities)}")
    print(f"\n为什么有 {len(opportunities)} 个机会？")
    print(f"  - 匹配的赛事: {len(matched_events)}")
    print(f"  - 每个赛事尝试 4 种组合:")
    print(f"    1. Kalshi A + Polymarket B")
    print(f"    2. Kalshi A + Polymarket A")
    print(f"    3. Kalshi B + Polymarket A")
    print(f"    4. Kalshi B + Polymarket B")
    print(f"  - 理论最大: {len(matched_events)} × 4 = {len(matched_events) * 4}")
    print(f"  - 实际发现: {len(opportunities)} (过滤掉了不满足条件的)")
    
    # 分析套利机会的分布
    print(f"\n套利机会分布:")
    profit_ranges = {
        "0-1%": 0,
        "1-5%": 0,
        "5-10%": 0,
        "10-50%": 0,
        "50%+": 0
    }
    
    for opp in opportunities:
        margin = opp.profit_margin
        if margin < 1:
            profit_ranges["0-1%"] += 1
        elif margin < 5:
            profit_ranges["1-5%"] += 1
        elif margin < 10:
            profit_ranges["5-10%"] += 1
        elif margin < 50:
            profit_ranges["10-50%"] += 1
        else:
            profit_ranges["50%+"] += 1
    
    for range_name, count in profit_ranges.items():
        print(f"  {range_name}: {count} 个")
    
    # 显示前10个套利机会
    print(f"\n前 10 个套利机会:")
    for i, opp in enumerate(opportunities[:10], 1):
        print(f"\n{i}. {opp.event_name}")
        print(f"   利润率: {opp.profit_margin:.2f}%")
        print(f"   预期利润: ${opp.expected_profit:.2f}")
        print(f"   Kalshi: {opp.kalshi_bet_team} @ {opp.kalshi_bet_price:.2f}")
        print(f"   Polymarket: {opp.polymarket_bet_team} @ {opp.polymarket_bet_price:.2f}")
        print(f"   概率和: {opp.kalshi_bet_price + opp.polymarket_bet_price:.2f}")
        
        # 检查是否合理
        if opp.kalshi_bet_team == opp.polymarket_bet_team:
            print(f"   ⚠️  警告: 下注同一队伍！")
        if opp.kalshi_bet_price + opp.polymarket_bet_price >= 1:
            print(f"   ⚠️  警告: 概率和 >= 1，不应该是套利机会！")
    
    # 清理
    if kalshi_client.session:
        await kalshi_client.session.close()
    if polymarket_client.session:
        await polymarket_client.session.close()


if __name__ == "__main__":
    asyncio.run(analyze())
