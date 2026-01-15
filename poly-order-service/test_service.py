#!/usr/bin/env python3
"""
测试 Polymarket 下单服务
"""
import requests
import json
import time

# 服务地址
BASE_URL = "http://127.0.0.1:8001"

def test_health():
    """测试健康检查"""
    print("=" * 60)
    print("测试 1: 健康检查")
    print("=" * 60)
    
    try:
        response = requests.get(f"{BASE_URL}/health", timeout=5)
        print(f"状态码: {response.status_code}")
        print(f"响应: {json.dumps(response.json(), indent=2, ensure_ascii=False)}")
        return response.status_code == 200
    except Exception as e:
        print(f"❌ 健康检查失败: {e}")
        return False

def test_market_order(token_id, amount=1.0):
    """测试市价单"""
    print("\n" + "=" * 60)
    print(f"测试 2: 市价单 (amount={amount} USDC)")
    print("=" * 60)
    
    data = {
        "token_id": token_id,
        "side": "buy",
        "amount": amount,
        "order_type": "FAK"
    }
    
    print(f"请求数据: {json.dumps(data, indent=2)}")
    
    try:
        start = time.time()
        response = requests.post(
            f"{BASE_URL}/order/market",
            json=data,
            timeout=60  # 60 秒超时
        )
        elapsed = time.time() - start
        
        print(f"\n状态码: {response.status_code}")
        print(f"总耗时: {elapsed:.2f}s")
        print(f"响应: {json.dumps(response.json(), indent=2, ensure_ascii=False)}")
        
        result = response.json()
        if result.get("success"):
            print(f"\n✅ 下单成功!")
            print(f"   订单ID: {result.get('order_id', 'N/A')}")
            print(f"   状态: {result.get('status', 'N/A')}")
            print(f"   API延迟: {result.get('latency_ms', 'N/A')}ms")
            return True
        else:
            print(f"\n❌ 下单失败: {result.get('error', 'Unknown error')}")
            return False
            
    except Exception as e:
        print(f"\n❌ 请求失败: {e}")
        return False

def test_balance():
    """测试余额查询"""
    print("\n" + "=" * 60)
    print("测试 3: 余额查询")
    print("=" * 60)
    
    try:
        response = requests.get(f"{BASE_URL}/balance", timeout=10)
        print(f"状态码: {response.status_code}")
        result = response.json()
        
        if result.get("success"):
            balance_data = result.get("balance", {})
            print(f"✅ 余额查询成功:")
            print(f"   {json.dumps(balance_data, indent=2, ensure_ascii=False)}")
            return True
        else:
            print(f"❌ 余额查询失败: {result.get('error', 'Unknown error')}")
            return False
            
    except Exception as e:
        print(f"❌ 请求失败: {e}")
        return False

if __name__ == "__main__":
    print("\n🚀 开始测试 Polymarket 下单服务\n")
    
    # 测试 1: 健康检查
    health_ok = test_health()
    
    if not health_ok:
        print("\n⚠️  服务未运行或未正常初始化")
        print("请先启动服务: python3 main.py")
        exit(1)
    
    # 测试 2: 余额查询
    test_balance()
    
    # 测试 3: 市价单（需要提供 token_id）
    print("\n" + "=" * 60)
    print("提示: 如需测试下单，请提供 token_id")
    print("=" * 60)
    print("\n示例用法:")
    print("  python3 test_service.py")
    print("\n或者手动测试下单:")
    print("""
curl -X POST http://127.0.0.1:8001/order/market \\
  -H "Content-Type: application/json" \\
  -d '{
    "token_id": "你的token_id",
    "side": "buy",
    "amount": 1.0,
    "order_type": "FAK"
  }'
""")
    
    print("\n✅ 基础测试完成!")
