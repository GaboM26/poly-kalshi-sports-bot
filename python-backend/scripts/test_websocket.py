#!/usr/bin/env python3
"""测试 WebSocket 连接和消息格式"""
import asyncio
import websockets
import json

async def test_websocket():
    uri = "ws://localhost:3000/ws"
    
    print("=" * 80)
    print("测试 WebSocket 连接")
    print("=" * 80)
    
    try:
        async with websockets.connect(uri) as websocket:
            print("✅ WebSocket 连接成功")
            
            # 接收前几条消息
            for i in range(10):
                try:
                    message = await asyncio.wait_for(websocket.recv(), timeout=10)
                    data = json.loads(message)
                    
                    print(f"\n消息 #{i+1}:")
                    print(f"  类型: {data.get('type')}")
                    
                    if data.get('type') == 'connected':
                        print(f"  消息: {data.get('message')}")
                    
                    elif data.get('type') == 'scan_completed':
                        print(f"  Kalshi: {data.get('kalshi_count')}")
                        print(f"  Polymarket: {data.get('polymarket_count')}")
                        print(f"  匹配: {data.get('matched_count')}")
                        print(f"  机会: {data.get('opportunities_count')}")
                    
                    elif data.get('type') == 'opportunity':
                        opp = data.get('data', {})
                        print(f"  事件: {opp.get('kalshi_market', {}).get('event_name')}")
                        print(f"  利润率: {opp.get('profit_margin'):.2f}%")
                        print(f"  预期利润: ${opp.get('expected_profit'):.2f}")
                    
                except asyncio.TimeoutError:
                    print(f"\n等待消息超时")
                    break
            
            print("\n✅ 测试完成")
            
    except Exception as e:
        print(f"❌ 连接失败: {e}")
        print("请确保后端正在运行: python main.py")

if __name__ == "__main__":
    asyncio.run(test_websocket())
