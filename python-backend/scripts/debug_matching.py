#!/usr/bin/env python3
"""调试匹配问题"""
import asyncio
import logging
from config import Config
from kalshi_client import KalshiClient
from polymarket_client import PolymarketClient
from matcher import EventMatcher

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

async def debug_matching():
    config = Config.from_file("../config.toml")
    
    # 获取市场数据
    kalshi_client = KalshiClient(config.kalshi)
    await kalshi_client.login()
    kalshi_markets = await kalshi_client.get_nba_markets()
    
    polymarket_client = PolymarketClient(config.polymarket)
    await polymarket_client.initialize_team_mappings()
    polymarket_markets = await polymarket_client.get_nba_markets()
    
    print("=" * 80)
    print(f"Kalshi 市场示例 (共 {len(kalshi_markets)} 个):")
    print("=" * 80)
    for market in kalshi_markets[:5]:
        print(f"  ID: {market.market_id}")
        print(f"  事件名: {market.event_name}")
        print(f"  队伍 A: {market.team_a}")
        print(f"  队伍 B: {market.team_b}")
        print(f"  价格: {market.price_a:.2f} / {market.price_b:.2f}")
        print()
    
    print("=" * 80)
    print(f"Polymarket 市场示例 (共 {len(polymarket_markets)} 个):")
    print("=" * 80)
    for market in polymarket_markets[:5]:
        print(f"  ID: {market.market_id}")
        print(f"  事件名: {market.event_name}")
        print(f"  队伍 A: {market.team_a}")
        print(f"  队伍 B: {market.team_b}")
        print(f"  价格: {market.price_a:.2f} / {market.price_b:.2f}")
        print()
    
    # 测试匹配
    print("=" * 80)
    print("测试匹配:")
    print("=" * 80)
    
    matcher = EventMatcher()
    
    # 手动测试几个匹配
    if kalshi_markets and polymarket_markets:
        k_market = kalshi_markets[0]
        print(f"\nKalshi 市场 1: {k_market.team_a} vs {k_market.team_b}")
        
        for p_market in polymarket_markets[:10]:
            score_a_a = matcher.calculate_similarity(k_market.team_a, p_market.team_a)
            score_b_b = matcher.calculate_similarity(k_market.team_b, p_market.team_b)
            score1 = (score_a_a + score_b_b) / 2
            
            score_a_b = matcher.calculate_similarity(k_market.team_a, p_market.team_b)
            score_b_a = matcher.calculate_similarity(k_market.team_b, p_market.team_a)
            score2 = (score_a_b + score_b_a) / 2
            
            score = max(score1, score2)
            
            if score > 0.3:
                print(f"  Polymarket: {p_market.team_a} vs {p_market.team_b}")
                print(f"    相似度: {score:.2f} (方案1: {score1:.2f}, 方案2: {score2:.2f})")
    
    # 清理
    if kalshi_client.session:
        await kalshi_client.session.close()
    if polymarket_client.session:
        await polymarket_client.session.close()

if __name__ == "__main__":
    asyncio.run(debug_matching())
