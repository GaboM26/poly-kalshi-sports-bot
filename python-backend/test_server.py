#!/usr/bin/env python3
"""测试服务器功能"""
import asyncio
import logging
from config import Config
from kalshi_client import KalshiClient
from polymarket_client import PolymarketClient
from matcher import EventMatcher
from calculator import ArbitrageCalculator

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


async def test_clients():
    """测试客户端功能"""
    logger.info("=" * 80)
    logger.info("开始测试客户端")
    logger.info("=" * 80)
    
    # 加载配置
    config = Config.from_file("../config.toml")
    logger.info("✅ 配置加载成功")
    
    # 初始化变量
    kalshi_markets = []
    polymarket_markets = []
    
    # 测试 Kalshi
    logger.info("\n" + "=" * 80)
    logger.info("测试 Kalshi 客户端")
    logger.info("=" * 80)
    
    kalshi_client = KalshiClient(config.kalshi)
    success = await kalshi_client.login()
    
    if success:
        logger.info("✅ Kalshi 登录成功")
        kalshi_markets = await kalshi_client.get_nba_markets()
        logger.info(f"✅ 获取到 {len(kalshi_markets)} 个 Kalshi 市场")
        
        if kalshi_markets:
            logger.info(f"\n示例市场:")
            for market in kalshi_markets[:3]:
                logger.info(f"  - {market.event_name}: {market.team_a} ({market.price_a:.2f}) vs {market.team_b} ({market.price_b:.2f})")
    else:
        logger.error("❌ Kalshi 登录失败，跳过 Kalshi 测试")
    
    # 测试 Polymarket
    logger.info("\n" + "=" * 80)
    logger.info("测试 Polymarket 客户端")
    logger.info("=" * 80)
    
    polymarket_client = PolymarketClient(config.polymarket)
    await polymarket_client.initialize_team_mappings()
    polymarket_markets = await polymarket_client.get_nba_markets()
    logger.info(f"✅ 获取到 {len(polymarket_markets)} 个 Polymarket 市场")
    
    if polymarket_markets:
        logger.info(f"\n示例市场:")
        for market in polymarket_markets[:3]:
            logger.info(f"  - {market.event_name}: {market.team_a} ({market.price_a:.2f}) vs {market.team_b} ({market.price_b:.2f})")
    
    # 测试匹配
    logger.info("\n" + "=" * 80)
    logger.info("测试赛事匹配")
    logger.info("=" * 80)
    
    matcher = EventMatcher()
    matched_events = matcher.match_events(kalshi_markets, polymarket_markets)
    logger.info(f"✅ 匹配到 {len(matched_events)} 个赛事")
    
    if matched_events:
        logger.info(f"\n匹配示例:")
        for event in matched_events[:3]:
            logger.info(f"  - {event.event_name} (置信度: {event.confidence:.2f})")
            if event.kalshi_market:
                logger.info(f"    Kalshi: {event.kalshi_market.team_a} vs {event.kalshi_market.team_b}")
            if event.polymarket_market:
                logger.info(f"    Polymarket: {event.polymarket_market.team_a} vs {event.polymarket_market.team_b}")
    
    # 测试套利计算
    logger.info("\n" + "=" * 80)
    logger.info("测试套利计算")
    logger.info("=" * 80)
    
    calculator = ArbitrageCalculator(
        min_profit_margin=config.settings.min_profit_margin,
        default_bet_amount=config.settings.default_bet_amount
    )
    opportunities = calculator.calculate_arbitrage(matched_events)
    logger.info(f"✅ 发现 {len(opportunities)} 个套利机会")
    
    if opportunities:
        logger.info(f"\n套利机会:")
        for opp in opportunities[:5]:
            logger.info(f"  - {opp.event_name}")
            logger.info(f"    利润率: {opp.profit_margin:.2f}%")
            logger.info(f"    预期利润: ${opp.expected_profit:.2f}")
            logger.info(f"    Kalshi 下注: {opp.kalshi_bet_team} @ {opp.kalshi_bet_price:.2f}")
            logger.info(f"    Polymarket 下注: {opp.polymarket_bet_team} @ {opp.polymarket_bet_price:.2f}")
    
    # 清理
    if kalshi_client.session:
        await kalshi_client.session.close()
    if polymarket_client.session:
        await polymarket_client.session.close()
    
    logger.info("\n" + "=" * 80)
    logger.info("测试完成")
    logger.info("=" * 80)


if __name__ == "__main__":
    asyncio.run(test_clients())
