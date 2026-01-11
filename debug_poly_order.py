#!/usr/bin/env python3
"""
Polymarket 下单调试脚本
用于调试 CHI-MIA 市场的 "Invalid order payload" 错误
"""

import json
import time
from datetime import datetime

# 调试日志路径
DEBUG_LOG_PATH = "/Users/meloner/rustcode/polytaoli/.cursor/debug.log"

def log_debug(hypothesis_id: str, location: str, message: str, data: dict = None):
    """写入调试日志"""
    # #region agent log
    entry = {
        "timestamp": int(time.time() * 1000),
        "sessionId": "debug-session",
        "runId": "poly-debug-1",
        "hypothesisId": hypothesis_id,
        "location": location,
        "message": message,
        "data": data or {}
    }
    with open(DEBUG_LOG_PATH, "a") as f:
        f.write(json.dumps(entry) + "\n")
    # #endregion

# 安装依赖提示
try:
    from py_clob_client.client import ClobClient
    from py_clob_client.clob_types import ApiCreds, OrderArgs, OrderType
    from py_clob_client.constants import POLYGON
except ImportError:
    print("请先安装依赖: pip install py-clob-client")
    exit(1)

# 配置
PRIVATE_KEY = "0x8d3d049e3714bc5fe4bcf6d0e990326445be3381615e6e25648b2e81ef2edf21"
WALLET_ADDRESS = "0x85B634AA874fd6a5E1a80ec9a64fDAbb395201D4"
CLOB_HOST = "https://clob.polymarket.com"

# CHI-MIA 市场的 token (Heat 方 - 失败的订单)
CHI_MIA_TOKEN_ID = "94515776290373751754638142228993059501097351216445649452643423016914071837398"

# PHX-WAS 市场的 token (成功的订单，用于对比)
PHX_WAS_TOKEN_ID = "113640777070257914779167991695197859988168871541269340805216299248113189823953"

def main():
    print("=" * 60)
    print("Polymarket 下单调试脚本")
    print("=" * 60)
    print(f"时间: {datetime.now()}")
    print(f"钱包: {WALLET_ADDRESS}")
    print()
    
    # #region agent log
    log_debug("H1", "main:start", "脚本启动", {"wallet": WALLET_ADDRESS})
    # #endregion
    
    # 1. 初始化客户端
    print("[1] 初始化 CLOB 客户端...")
    try:
        client = ClobClient(
            host=CLOB_HOST,
            chain_id=POLYGON,
            key=PRIVATE_KEY,
            signature_type=1,  # Magic Link
            funder=WALLET_ADDRESS
        )
        print("    ✅ 客户端初始化成功")
        
        # #region agent log
        log_debug("H1", "main:client_init", "客户端初始化成功", {"host": CLOB_HOST, "chain_id": POLYGON})
        # #endregion
    except Exception as e:
        print(f"    ❌ 客户端初始化失败: {e}")
        log_debug("H1", "main:client_init_error", "客户端初始化失败", {"error": str(e)})
        return
    
    # 2. 获取 API 凭据
    print("\n[2] 获取/派生 API 凭据...")
    try:
        # 尝试不同的方法名
        if hasattr(client, 'create_or_derive_api_creds'):
            creds = client.create_or_derive_api_creds()
        elif hasattr(client, 'derive_api_creds'):
            creds = client.derive_api_creds()
        elif hasattr(client, 'create_api_creds'):
            creds = client.create_api_creds()
        else:
            # 列出所有可用方法
            methods = [m for m in dir(client) if not m.startswith('_') and 'cred' in m.lower()]
            print(f"    可用的 creds 相关方法: {methods}")
            all_methods = [m for m in dir(client) if not m.startswith('_')]
            print(f"    所有公共方法: {all_methods}")
            raise Exception("找不到凭据派生方法")
        
        client.set_api_creds(creds)
        print(f"    ✅ API Key: {creds.api_key[:20]}...")
        
        # #region agent log
        log_debug("H1", "main:api_creds", "API凭据获取成功", {"api_key_prefix": creds.api_key[:20]})
        # #endregion
    except Exception as e:
        print(f"    ❌ 获取凭据失败: {e}")
        log_debug("H1", "main:api_creds_error", "获取凭据失败", {"error": str(e)})
        return
    
    # 3. 查询市场信息
    print("\n[3] 查询市场信息...")
    for name, token_id in [("CHI-MIA (失败)", CHI_MIA_TOKEN_ID), ("PHX-WAS (成功)", PHX_WAS_TOKEN_ID)]:
        print(f"\n    --- {name} ---")
        try:
            # 获取 tick_size
            tick_size = client.get_tick_size(token_id)
            print(f"    tick_size: {tick_size}")
            
            # 获取 neg_risk
            neg_risk = client.get_neg_risk(token_id)
            print(f"    neg_risk: {neg_risk}")
            
            # 获取订单簿
            book = client.get_order_book(token_id)
            best_bid = book.bids[0] if book.bids else None
            best_ask = book.asks[0] if book.asks else None
            print(f"    best_bid (bids[0]): {best_bid}")
            print(f"    best_ask (asks[0]): {best_ask}")
            
            # 打印前5个 asks 来查看排序顺序
            print(f"    前5个 asks (检查排序):")
            for i, ask in enumerate(book.asks[:5]):
                print(f"      asks[{i}]: price={ask.price}, size={ask.size}")
            
            # 打印前5个 bids
            print(f"    前5个 bids (检查排序):")
            for i, bid in enumerate(book.bids[:5]):
                print(f"      bids[{i}]: price={bid.price}, size={bid.size}")
            
            # #region agent log
            log_debug("H3", f"main:market_info:{name[:7]}", "市场信息", {
                "token_id": token_id[:20] + "...",
                "tick_size": tick_size,
                "neg_risk": neg_risk,
                "best_bid": str(best_bid),
                "best_ask": str(best_ask),
                "bids_count": len(book.bids),
                "asks_count": len(book.asks),
                "asks_first_5": [(a.price, a.size) for a in book.asks[:5]],
                "bids_first_5": [(b.price, b.size) for b in book.bids[:5]]
            })
            # #endregion
            
        except Exception as e:
            print(f"    ❌ 查询失败: {e}")
            log_debug("H3", f"main:market_info_error:{name[:7]}", "查询市场失败", {"error": str(e)})
    
    # 4. 尝试向 CHI-MIA 下单 (1 USD)
    print("\n" + "=" * 60)
    print("[4] 尝试向 CHI-MIA 市场下单 1 USD...")
    print("=" * 60)
    
    try:
        # 获取当前价格
        book = client.get_order_book(CHI_MIA_TOKEN_ID)
        if not book.asks:
            print("    ❌ 订单簿没有卖单")
            log_debug("H3", "main:no_asks", "订单簿没有卖单", {})
            return
            
        best_ask_price = float(book.asks[0].price)
        print(f"    当前最优卖价: {best_ask_price}")
        
        # 计算购买数量 (1 USD / price)
        amount = 1.0  # 1 USD
        
        # #region agent log
        log_debug("H2", "main:order_params", "订单参数", {
            "token_id": CHI_MIA_TOKEN_ID[:20] + "...",
            "side": "BUY",
            "amount": amount,
            "price": best_ask_price
        })
        # #endregion
        
        print(f"    下单参数: side=BUY, amount={amount} USD")
        print()
        
        # 检测可用的下单方法
        order_methods = [m for m in dir(client) if 'order' in m.lower() and not m.startswith('_')]
        print(f"    可用的 order 方法: {order_methods}")
        
        # #region agent log
        log_debug("H1", "main:before_order", "准备下单", {"order_methods": order_methods})
        # #endregion
        
        # 尝试使用 create_order + post_order 组合 (限价单方式)
        print("    调用 create_order + post_order...")
        
        # 计算要买多少份 (size = amount / price)
        # Polymarket 限制: maker amount 最多 2 位小数, taker amount (size) 最多 4 位小数
        raw_size = amount / best_ask_price
        size = round(raw_size, 4)  # 四舍五入到 4 位小数
        print(f"    计算: {amount} USD / {best_ask_price} = {raw_size:.8f} -> 四舍五入 -> {size} 份")
        
        # #region agent log
        log_debug("H5", "main:limit_order_calc", "限价单计算", {
            "amount_usd": amount,
            "price": best_ask_price,
            "size": size
        })
        # #endregion
        
        signed_order = client.create_order(
            OrderArgs(
                token_id=CHI_MIA_TOKEN_ID,
                price=best_ask_price,
                side="BUY",
                size=size,
            )
        )
        
        # #region agent log
        log_debug("H2", "main:signed_order", "签名订单", {"order": str(signed_order)[:200]})
        # #endregion
        
        print(f"    签名订单: {str(signed_order)[:100]}...")
        
        response = client.post_order(signed_order, OrderType.FOK)
        
        print(f"    ✅ 下单成功!")
        print(f"    响应: {json.dumps(response, indent=2)}")
        
        # #region agent log
        log_debug("H1", "main:order_success", "下单成功", {"response": response})
        # #endregion
        
    except Exception as e:
        error_str = str(e)
        print(f"    ❌ 下单失败: {error_str}")
        
        # #region agent log
        log_debug("H1", "main:order_error", "下单失败", {"error": error_str, "error_type": type(e).__name__})
        # #endregion
        
        # 尝试更多调试信息
        print("\n    --- 额外调试 ---")
        
        # 尝试用 FAK 订单类型
        print("    尝试使用 FAK (Fill and Kill) 订单类型...")
        try:
            signed_order = client.create_order(
                OrderArgs(
                    token_id=CHI_MIA_TOKEN_ID,
                    price=best_ask_price,
                    side="BUY",
                    size=size,
                )
            )
            response = client.post_order(signed_order, OrderType.FAK)
            print(f"    ✅ FAK 下单成功!")
            print(f"    响应: {json.dumps(response, indent=2)}")
            
            # #region agent log
            log_debug("H2", "main:fak_success", "FAK下单成功", {"response": response})
            # #endregion
            
        except Exception as e2:
            print(f"    ❌ FAK 也失败: {e2}")
            
            # #region agent log
            log_debug("H2", "main:fak_error", "FAK下单也失败", {"error": str(e2)})
            # #endregion
    
    # 5. 对比：尝试向 PHX-WAS 下单 (作为参考)
    print("\n" + "=" * 60)
    print("[5] 对比：尝试向 PHX-WAS 市场下单 1 USD...")
    print("=" * 60)
    
    try:
        book = client.get_order_book(PHX_WAS_TOKEN_ID)
        if not book.asks:
            print("    ❌ 订单簿没有卖单")
            return
            
        best_ask_price = float(book.asks[0].price)
        print(f"    当前最优卖价: {best_ask_price}")
        
        amount = 1.0
        raw_size = amount / best_ask_price
        size = round(raw_size, 4)  # 四舍五入到 4 位小数
        print(f"    计算: {amount} USD / {best_ask_price} = {raw_size:.8f} -> 四舍五入 -> {size} 份")
        
        # #region agent log
        log_debug("H4", "main:phx_order_params", "PHX订单参数", {
            "token_id": PHX_WAS_TOKEN_ID[:20] + "...",
            "side": "BUY",
            "amount": amount,
            "price": best_ask_price,
            "size": size
        })
        # #endregion
        
        signed_order = client.create_order(
            OrderArgs(
                token_id=PHX_WAS_TOKEN_ID,
                price=best_ask_price,
                side="BUY",
                size=size,
            )
        )
        response = client.post_order(signed_order, OrderType.FAK)
        
        print(f"    ✅ 下单成功!")
        print(f"    响应: {json.dumps(response, indent=2)}")
        
        # #region agent log
        log_debug("H4", "main:phx_success", "PHX下单成功", {"response": response})
        # #endregion
        
    except Exception as e:
        print(f"    ❌ 下单失败: {e}")
        
        # #region agent log
        log_debug("H4", "main:phx_error", "PHX下单失败", {"error": str(e)})
        # #endregion
    
    print("\n" + "=" * 60)
    print("调试完成")
    print("=" * 60)

if __name__ == "__main__":
    main()
