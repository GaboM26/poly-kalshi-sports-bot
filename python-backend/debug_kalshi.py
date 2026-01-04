#!/usr/bin/env python3
"""调试 Kalshi API"""
import asyncio
import aiohttp
import json
from config import Config

async def debug_kalshi():
    config = Config.from_file("../config.toml")
    
    base_url = config.kalshi.base_url
    token = config.kalshi.api_key
    
    headers = {
        "Content-Type": "application/json",
        "Authorization": f"Bearer {token}"
    }
    
    async with aiohttp.ClientSession() as session:
        # 1. 测试连接
        print("=" * 80)
        print("1. 测试 Kalshi 连接")
        print("=" * 80)
        url = f"{base_url}/exchange/status"
        async with session.get(url, headers=headers) as resp:
            print(f"状态码: {resp.status}")
            if resp.status == 200:
                data = await resp.json()
                print(f"响应: {json.dumps(data, indent=2)}")
        
        # 2. 获取所有系列
        print("\n" + "=" * 80)
        print("2. 获取所有系列 (Series)")
        print("=" * 80)
        url = f"{base_url}/series"
        async with session.get(url, headers=headers) as resp:
            print(f"状态码: {resp.status}")
            if resp.status == 200:
                data = await resp.json()
                series_list = data.get("series", [])
                print(f"找到 {len(series_list)} 个系列")
                
                # 查找 NBA 相关的系列
                nba_series = [s for s in series_list if 'NBA' in s.get('title', '').upper()]
                print(f"\nNBA 相关系列:")
                for s in nba_series:
                    print(f"  - {s.get('title')} (ticker: {s.get('ticker')})")
        
        # 3. 尝试不同的参数获取赛事
        print("\n" + "=" * 80)
        print("3. 尝试获取 NBA 赛事")
        print("=" * 80)
        
        # 尝试 1: series_ticker=NBA
        print("\n尝试 1: series_ticker=NBA")
        url = f"{base_url}/events"
        params = {"series_ticker": "NBA", "status": "open", "limit": 10}
        async with session.get(url, params=params, headers=headers) as resp:
            print(f"状态码: {resp.status}")
            if resp.status == 200:
                data = await resp.json()
                events = data.get("events", [])
                print(f"找到 {len(events)} 个赛事")
                for e in events[:3]:
                    print(f"  - {e.get('title')} (ticker: {e.get('event_ticker')})")
            else:
                text = await resp.text()
                print(f"错误: {text}")
        
        # 尝试 2: 不带 series_ticker
        print("\n尝试 2: 不带 series_ticker，只用 status")
        params = {"status": "open", "limit": 10}
        async with session.get(url, params=params, headers=headers) as resp:
            print(f"状态码: {resp.status}")
            if resp.status == 200:
                data = await resp.json()
                events = data.get("events", [])
                print(f"找到 {len(events)} 个赛事")
                for e in events[:3]:
                    print(f"  - {e.get('title')} (series: {e.get('series_ticker')})")
        
        # 尝试 3: 搜索 basketball
        print("\n尝试 3: 搜索 basketball")
        url = f"{base_url}/markets"
        params = {"limit": 10, "status": "open"}
        async with session.get(url, params=params, headers=headers) as resp:
            print(f"状态码: {resp.status}")
            if resp.status == 200:
                data = await resp.json()
                markets = data.get("markets", [])
                print(f"找到 {len(markets)} 个市场")
                
                # 查找 NBA 相关
                nba_markets = [m for m in markets if 'NBA' in m.get('title', '').upper() or 'basketball' in m.get('title', '').lower()]
                print(f"NBA 相关市场: {len(nba_markets)}")
                for m in nba_markets[:3]:
                    print(f"  - {m.get('title')}")

if __name__ == "__main__":
    asyncio.run(debug_kalshi())
