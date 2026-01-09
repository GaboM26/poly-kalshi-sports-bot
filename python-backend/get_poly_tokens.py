#!/usr/bin/env python3
"""
获取 Polymarket NBA 市场的 token IDs
用于 poly_orderbook_monitor.py --token 参数
"""

import requests
import json

BASE_URL = "https://gamma-api.polymarket.com"

def get_nba_markets():
    """获取所有 NBA 市场"""
    print("正在获取 NBA 市场...")
    
    # 获取体育联赛
    sports = requests.get(f"{BASE_URL}/sports").json()
    
    nba_leagues = [
        s for s in sports 
        if 'NBA' in s.get('sport', '').upper() 
        and 'WNBA' not in s.get('sport', '').upper()
    ]
    
    if not nba_leagues:
        print("未找到 NBA 联赛")
        return []
    
    markets = []
    
    for league in nba_leagues[:1]:
        series_id = league.get('series')
        
        params = {
            'series_id': str(series_id),
            'tag_id': '100639',
            'active': 'true',
            'closed': 'false',
            'limit': '50'
        }
        
        events = requests.get(f"{BASE_URL}/events", params=params).json()
        
        for event in events:
            title = event.get('title', '')
            
            for market in event.get('markets', []):
                outcomes_str = market.get('outcomes')
                tokens_str = market.get('clobTokenIds')
                prices_str = market.get('outcomePrices')
                question = market.get('question', title)
                
                if not outcomes_str or not tokens_str:
                    continue
                
                try:
                    outcomes = json.loads(outcomes_str)
                    tokens = json.loads(tokens_str)
                    prices = json.loads(prices_str) if prices_str else [0.5, 0.5]
                except:
                    continue
                
                # 跳过 Yes/No 市场
                if any(o.lower() == "yes" for o in outcomes):
                    continue
                
                # 只保留全场输赢市场
                if question != title:
                    continue
                
                if len(outcomes) == 2 and len(tokens) == 2:
                    markets.append({
                        'title': title,
                        'team_a': outcomes[0],
                        'team_b': outcomes[1],
                        'token_a': tokens[0],
                        'token_b': tokens[1],
                        'price_a': float(prices[0]),
                        'price_b': float(prices[1]),
                    })
    
    return markets


def main():
    markets = get_nba_markets()
    
    if not markets:
        print("没有找到市场")
        return
    
    print(f"\n找到 {len(markets)} 个 NBA 市场:\n")
    print("=" * 100)
    
    for i, m in enumerate(markets):
        print(f"\n[{i+1}] {m['title']}")
        print(f"    {m['team_a']}: {m['price_a']*100:.0f}¢  vs  {m['team_b']}: {m['price_b']*100:.0f}¢")
        
        # 生成带名称的命令
        print(f"\n    python poly_orderbook_monitor.py --token {m['token_a']} {m['token_b']} --names \"{m['team_a']}\" \"{m['team_b']}\" --debug")
    
    print("\n" + "=" * 100)


if __name__ == "__main__":
    main()
