#!/usr/bin/env python3
"""
获取 Polymarket NBA 赛事的订单簿数据
用于分析订单簿的排序和结构
"""

import requests
import json
from datetime import datetime
from pathlib import Path

# Polymarket API endpoints
GAMMA_API = "https://gamma-api.polymarket.com"
CLOB_API = "https://clob.polymarket.com"

def get_nba_markets():
    """获取所有 NBA 市场"""
    print("📊 获取 NBA 市场列表...")
    
    # 方法1: 通过搜索获取
    url = f"{GAMMA_API}/markets"
    params = {
        "limit": 100,
        "closed": "false",  # 只要未关闭的
    }
    
    try:
        response = requests.get(url, params=params, timeout=10)
        response.raise_for_status()
        all_markets = response.json()
    except Exception as e:
        print(f"   ❌ 获取市场列表失败: {e}")
        return []
    
    # 过滤 NBA 市场
    nba_markets = []
    for market in all_markets:
        question = market.get("question", "").lower()
        description = market.get("description", "").lower()
        tags = [tag.lower() for tag in market.get("tags", [])]
        
        # 检查是否包含 NBA 相关关键词
        if any(keyword in question or keyword in description or keyword in str(tags) 
               for keyword in ["nba", "basketball", "lakers", "warriors", "celtics", "heat"]):
            nba_markets.append(market)
    
    print(f"   找到 {len(nba_markets)} 个 NBA 相关市场")
    
    # 如果没找到，尝试直接使用已知的 token_id（从你的日志中提取）
    if len(nba_markets) == 0:
        print("   ℹ️  使用已知的 NBA token_id 进行测试...")
        # 从你的日志中提取的 token_id
        test_tokens = [
            "94515776290373751754638142228993059501097351216445649452643423016914071837398",  # CHI-MIA
            "16215889044933102237616087156593010997453882627000153373435840826284741185376",  # CHI-MIA
            "103007116798336628661619679985222919791811823436106325887661137036158976283917", # NYK-POR
        ]
        
        # 创建虚拟市场对象用于测试
        for i, token_id in enumerate(test_tokens):
            nba_markets.append({
                "question": f"Test NBA Market {i+1}",
                "id": f"test_{i}",
                "tokens": [{"token_id": token_id, "outcome": "Test"}]
            })
    
    return nba_markets

def get_orderbook(token_id):
    """获取指定 token 的订单簿"""
    url = f"{CLOB_API}/book"
    params = {"token_id": token_id}
    
    try:
        response = requests.get(url, params=params, timeout=10)
        response.raise_for_status()
        return response.json()
    except Exception as e:
        print(f"   ❌ 获取订单簿失败: {e}")
        return None

def analyze_orderbook(orderbook_data, token_id):
    """分析订单簿数据"""
    if not orderbook_data:
        return None
    
    bids = orderbook_data.get("bids", [])
    asks = orderbook_data.get("asks", [])
    
    analysis = {
        "token_id": token_id,
        "timestamp": datetime.now().isoformat(),
        "bids_count": len(bids),
        "asks_count": len(asks),
        "bids": [],
        "asks": [],
    }
    
    # 分析 bids
    if bids:
        for i, bid in enumerate(bids[:5]):  # 只取前5个
            price = float(bid.get("price", 0))
            size = float(bid.get("size", 0))
            analysis["bids"].append({
                "index": i,
                "price": price,
                "size": size,
                "value": price * size
            })
        
        # 检查排序
        prices = [float(b.get("price", 0)) for b in bids]
        is_ascending = all(prices[i] <= prices[i+1] for i in range(len(prices)-1))
        is_descending = all(prices[i] >= prices[i+1] for i in range(len(prices)-1))
        
        analysis["bids_sort"] = "ascending" if is_ascending else ("descending" if is_descending else "unsorted")
        analysis["best_bid"] = {"price": prices[0], "position": "first"} if prices else None
        analysis["worst_bid"] = {"price": prices[-1], "position": "last"} if prices else None
    
    # 分析 asks
    if asks:
        for i, ask in enumerate(asks[:5]):  # 只取前5个
            price = float(ask.get("price", 0))
            size = float(ask.get("size", 0))
            analysis["asks"].append({
                "index": i,
                "price": price,
                "size": size,
                "value": price * size
            })
        
        # 检查排序
        prices = [float(a.get("price", 0)) for a in asks]
        is_ascending = all(prices[i] <= prices[i+1] for i in range(len(prices)-1))
        is_descending = all(prices[i] >= prices[i+1] for i in range(len(prices)-1))
        
        analysis["asks_sort"] = "ascending" if is_ascending else ("descending" if is_descending else "unsorted")
        analysis["best_ask"] = {"price": prices[0], "position": "first"} if prices else None
        analysis["worst_ask"] = {"price": prices[-1], "position": "last"} if prices else None
    
    return analysis

def main():
    print("=" * 60)
    print("Polymarket NBA 订单簿数据采集")
    print("=" * 60)
    print()
    
    # 创建输出目录
    output_dir = Path("orderbook_analysis")
    output_dir.mkdir(exist_ok=True)
    
    # 获取 NBA 市场
    markets = get_nba_markets()
    
    if not markets:
        print("❌ 没有找到 NBA 市场")
        return
    
    # 收集所有订单簿数据
    all_orderbooks = []
    all_analysis = []
    
    print(f"\n📥 开始获取订单簿数据...")
    print()
    
    for i, market in enumerate(markets[:10], 1):  # 限制前10个市场
        question = market.get("question", "Unknown")
        market_id = market.get("id", "")
        tokens = market.get("tokens", [])
        
        print(f"{i}. {question}")
        print(f"   Market ID: {market_id}")
        
        for token in tokens:
            token_id = token.get("token_id", "")
            outcome = token.get("outcome", "Unknown")
            
            if not token_id:
                continue
            
            print(f"   📖 获取 {outcome} 的订单簿...")
            
            orderbook = get_orderbook(token_id)
            if orderbook:
                all_orderbooks.append({
                    "market": question,
                    "market_id": market_id,
                    "outcome": outcome,
                    "token_id": token_id,
                    "orderbook": orderbook
                })
                
                # 分析订单簿
                analysis = analyze_orderbook(orderbook, token_id)
                if analysis:
                    analysis["market"] = question
                    analysis["outcome"] = outcome
                    all_analysis.append(analysis)
                    
                    # 打印简要信息
                    if analysis.get("bids"):
                        print(f"      Bids: {analysis['bids_sort']}, "
                              f"first={analysis['bids'][0]['price']:.4f}, "
                              f"last={analysis['bids'][-1]['price']:.4f}")
                    if analysis.get("asks"):
                        print(f"      Asks: {analysis['asks_sort']}, "
                              f"first={analysis['asks'][0]['price']:.4f}, "
                              f"last={analysis['asks'][-1]['price']:.4f}")
        
        print()
    
    # 保存原始数据
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    
    raw_file = output_dir / f"orderbooks_raw_{timestamp}.json"
    with open(raw_file, "w", encoding="utf-8") as f:
        json.dump(all_orderbooks, f, indent=2, ensure_ascii=False)
    print(f"✅ 原始数据已保存: {raw_file}")
    
    # 保存分析结果
    analysis_file = output_dir / f"orderbooks_analysis_{timestamp}.json"
    with open(analysis_file, "w", encoding="utf-8") as f:
        json.dump(all_analysis, f, indent=2, ensure_ascii=False)
    print(f"✅ 分析结果已保存: {analysis_file}")
    
    # 生成总结报告
    print("\n" + "=" * 60)
    print("📊 订单簿排序分析总结")
    print("=" * 60)
    
    bids_sort_summary = {}
    asks_sort_summary = {}
    
    for analysis in all_analysis:
        bids_sort = analysis.get("bids_sort", "unknown")
        asks_sort = analysis.get("asks_sort", "unknown")
        
        bids_sort_summary[bids_sort] = bids_sort_summary.get(bids_sort, 0) + 1
        asks_sort_summary[asks_sort] = asks_sort_summary.get(asks_sort, 0) + 1
    
    print(f"\nBids 排序统计:")
    for sort_type, count in bids_sort_summary.items():
        print(f"  {sort_type}: {count} 个市场")
    
    print(f"\nAsks 排序统计:")
    for sort_type, count in asks_sort_summary.items():
        print(f"  {sort_type}: {count} 个市场")
    
    # 保存总结
    summary = {
        "timestamp": datetime.now().isoformat(),
        "total_markets": len(markets),
        "analyzed_markets": len(all_analysis),
        "bids_sort_summary": bids_sort_summary,
        "asks_sort_summary": asks_sort_summary,
    }
    
    summary_file = output_dir / f"summary_{timestamp}.json"
    with open(summary_file, "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2, ensure_ascii=False)
    print(f"\n✅ 总结报告已保存: {summary_file}")
    
    print("\n" + "=" * 60)
    print("✅ 数据采集完成！")
    print("=" * 60)

if __name__ == "__main__":
    main()
