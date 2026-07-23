#!/usr/bin/env python3
"""
Test the Polymarket order service.
"""
import requests
import json
import time

# Service URL
BASE_URL = "http://127.0.0.1:8001"

def test_health():
    """Test the health check."""
    print("=" * 60)
    print("Test 1: Health check")
    print("=" * 60)
    
    try:
        response = requests.get(f"{BASE_URL}/health", timeout=5)
        print(f"Status code: {response.status_code}")
        print(f"Response: {json.dumps(response.json(), indent=2, ensure_ascii=False)}")
        return response.status_code == 200
    except Exception as e:
        print(f"❌ Health check failed: {e}")
        return False

def test_market_order(token_id, amount=1.0):
    """Test a market order."""
    print("\n" + "=" * 60)
    print(f"Test 2: Market order (amount={amount} USDC)")
    print("=" * 60)
    
    data = {
        "token_id": token_id,
        "side": "buy",
        "amount": amount,
        "order_type": "FAK"
    }
    
    print(f"Request data: {json.dumps(data, indent=2)}")
    
    try:
        start = time.time()
        response = requests.post(
            f"{BASE_URL}/order/market",
            json=data,
            timeout=60  # 60-second timeout
        )
        elapsed = time.time() - start
        
        print(f"\nStatus code: {response.status_code}")
        print(f"Total time: {elapsed:.2f}s")
        print(f"Response: {json.dumps(response.json(), indent=2, ensure_ascii=False)}")
        
        result = response.json()
        if result.get("success"):
            print(f"\n✅ Order placed successfully!")
            print(f"   Order ID: {result.get('order_id', 'N/A')}")
            print(f"   Status: {result.get('status', 'N/A')}")
            print(f"   API latency: {result.get('latency_ms', 'N/A')}ms")
            return True
        else:
            print(f"\n❌ Order placement failed: {result.get('error', 'Unknown error')}")
            return False
            
    except Exception as e:
        print(f"\n❌ Request failed: {e}")
        return False

def test_balance():
    """Test balance retrieval."""
    print("\n" + "=" * 60)
    print("Test 3: Balance retrieval")
    print("=" * 60)
    
    try:
        response = requests.get(f"{BASE_URL}/balance", timeout=10)
        print(f"Status code: {response.status_code}")
        result = response.json()
        
        if result.get("success"):
            balance_data = result.get("balance", {})
            print(f"✅ Balance retrieved successfully:")
            print(f"   {json.dumps(balance_data, indent=2, ensure_ascii=False)}")
            return True
        else:
            print(f"❌ Balance retrieval failed: {result.get('error', 'Unknown error')}")
            return False
            
    except Exception as e:
        print(f"❌ Request failed: {e}")
        return False

if __name__ == "__main__":
    print("\n🚀 Starting Polymarket order service tests\n")
    
    # Test 1: Health check
    health_ok = test_health()
    
    if not health_ok:
        print("\n⚠️  Service is not running or was not initialized correctly")
        print("Start the service first: python3 main.py")
        exit(1)
    
    # Test 2: Balance retrieval
    test_balance()
    
    # Test 3: Market order (requires a token_id)
    print("\n" + "=" * 60)
    print("Tip: provide a token_id to test order placement")
    print("=" * 60)
    print("\nExample usage:")
    print("  python3 test_service.py")
    print("\nOr test order placement manually:")
    print("""
curl -X POST http://127.0.0.1:8001/order/market \\
  -H "Content-Type: application/json" \\
  -d '{
    "token_id": "your_token_id",
    "side": "buy",
    "amount": 1.0,
    "order_type": "FAK"
  }'
""")
    
    print("\n✅ Basic tests complete!")
