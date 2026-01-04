#!/usr/bin/env python3
"""
演示脚本 - 展示如何使用 Python 后端 API
"""
import asyncio
import aiohttp
import json
from datetime import datetime


async def demo_api():
    """演示 API 调用"""
    base_url = "http://localhost:3000"
    
    print("=" * 80)
    print("预测市场套利扫描器 - API 演示")
    print("=" * 80)
    print()
    
    async with aiohttp.ClientSession() as session:
        # 1. 检查服务器状态
        print("1️⃣  检查服务器状态...")
        try:
            async with session.get(f"{base_url}/") as resp:
                if resp.status == 200:
                    data = await resp.json()
                    print(f"   ✅ 服务器运行中: {data.get('message')}")
                    print(f"   版本: {data.get('version')}")
                else:
                    print(f"   ❌ 服务器响应异常: {resp.status}")
                    return
        except Exception as e:
            print(f"   ❌ 无法连接到服务器: {e}")
            print(f"   请先启动服务器: python main.py")
            return
        
        print()
        
        # 2. 获取系统统计
        print("2️⃣  获取系统统计信息...")
        try:
            async with session.get(f"{base_url}/api/stats") as resp:
                if resp.status == 200:
                    stats = await resp.json()
                    print(f"   📊 Kalshi 市场数: {stats.get('total_kalshi_markets', 0)}")
                    print(f"   📊 Polymarket 市场数: {stats.get('total_polymarket_markets', 0)}")
                    print(f"   🎯 匹配的赛事数: {stats.get('matched_events', 0)}")
                    print(f"   💰 套利机会数: {stats.get('arbitrage_opportunities', 0)}")
                    if stats.get('last_update'):
                        print(f"   🕐 最后更新: {stats.get('last_update')}")
                else:
                    print(f"   ❌ 获取统计失败: {resp.status}")
        except Exception as e:
            print(f"   ❌ 获取统计异常: {e}")
        
        print()
        
        # 3. 获取套利机会
        print("3️⃣  获取套利机会...")
        try:
            async with session.get(f"{base_url}/api/opportunities") as resp:
                if resp.status == 200:
                    opportunities = await resp.json()
                    
                    if not opportunities:
                        print(f"   ℹ️  当前没有发现套利机会")
                        print(f"   提示: 可以降低 config.toml 中的 min_profit_margin 来发现更多机会")
                    else:
                        print(f"   🎉 发现 {len(opportunities)} 个套利机会！")
                        print()
                        
                        for i, opp in enumerate(opportunities[:5], 1):
                            print(f"   机会 #{i}:")
                            print(f"   ├─ 赛事: {opp.get('event_name')}")
                            print(f"   ├─ 利润率: {opp.get('profit_margin', 0):.2f}%")
                            print(f"   ├─ 预期利润: ${opp.get('expected_profit', 0):.2f}")
                            print(f"   ├─ 下注金额: ${opp.get('bet_amount', 0):.2f}")
                            print(f"   ├─ Kalshi 下注: {opp.get('kalshi_bet_team')} @ {opp.get('kalshi_bet_price', 0):.2f}")
                            print(f"   └─ Polymarket 下注: {opp.get('polymarket_bet_team')} @ {opp.get('polymarket_bet_price', 0):.2f}")
                            print()
                else:
                    print(f"   ❌ 获取套利机会失败: {resp.status}")
        except Exception as e:
            print(f"   ❌ 获取套利机会异常: {e}")
        
        print()
        
        # 4. WebSocket 演示
        print("4️⃣  WebSocket 实时推送演示...")
        print("   提示: 按 Ctrl+C 停止")
        print()
        
        try:
            import websockets
            
            ws_url = "ws://localhost:3000/ws"
            async with websockets.connect(ws_url) as websocket:
                print(f"   ✅ WebSocket 已连接")
                
                # 接收几条消息
                for i in range(3):
                    message = await asyncio.wait_for(
                        websocket.recv(), 
                        timeout=10
                    )
                    data = json.loads(message)
                    
                    msg_type = data.get('type')
                    if msg_type == 'connected':
                        print(f"   📡 {data.get('message')}")
                    elif msg_type == 'update':
                        stats = data.get('stats', {})
                        opps = data.get('opportunities', [])
                        print(f"   📊 更新: {stats.get('total_polymarket_markets', 0)} 个市场, {len(opps)} 个套利机会")
                    
                    if i < 2:
                        await asyncio.sleep(1)
                
                print(f"   ✅ WebSocket 演示完成")
                
        except ImportError:
            print(f"   ℹ️  需要安装 websockets 库: pip install websockets")
        except asyncio.TimeoutError:
            print(f"   ⏱️  等待消息超时")
        except Exception as e:
            print(f"   ❌ WebSocket 连接失败: {e}")
    
    print()
    print("=" * 80)
    print("演示完成！")
    print("=" * 80)
    print()
    print("下一步:")
    print("  - 访问 http://localhost:3000 查看 API")
    print("  - 访问 http://localhost:3000/api/stats 查看统计")
    print("  - 访问 http://localhost:3000/api/opportunities 查看套利机会")
    print("  - 启动前端: cd ../web && npm run dev")
    print()


if __name__ == "__main__":
    try:
        asyncio.run(demo_api())
    except KeyboardInterrupt:
        print("\n\n👋 演示已停止")
