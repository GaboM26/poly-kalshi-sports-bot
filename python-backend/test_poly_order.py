#!/opt/anaconda3/bin/python
"""Polymarket 市价订单测试脚本

使用方法:
    python test_poly_order.py <token_id> [--side buy|sell] [--amount N]

示例:
    # 买入 $1 的合约
    python test_poly_order.py 12345...abc --side buy --amount 1
    
    # 卖出 $2 的合约
    python test_poly_order.py 12345...abc --side sell --amount 2
"""
import asyncio
import argparse
import logging
import sys
from pathlib import Path

# 添加项目路径
sys.path.insert(0, str(Path(__file__).parent))

from app.core.config import Config
from app.clients.polymarket import PolymarketClient

# 配置日志
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)


async def test_order(token_id: str, side: str, amount: float):
    """执行测试下单"""
    
    # 加载配置
    config = Config.from_file()
    
    print("=" * 60)
    print("Polymarket 市价订单测试")
    print("=" * 60)
    print(f"Token ID: {token_id[:32]}..." if len(token_id) > 32 else f"Token ID: {token_id}")
    print(f"方向 (side): {side}")
    print(f"金额 (amount): ${amount:.2f} USDC")
    print("=" * 60)
    
    async with PolymarketClient(config.polymarket) as client:
        # 1. 测试连接 - 获取余额
        print("\n1. 测试 API 连接...")
        balance_data = await client.get_balance()
        
        if not balance_data:
            print("❌ API 连接失败，请检查配置")
            return
        
        print("✅ API 连接成功")
        balance = balance_data.get("balance", 0)
        print(f"   可用余额: ${balance:.2f} USDC")
        print(f"   Smart Wallet: {balance_data.get('smart_wallet', 'N/A')}")
        print(f"   Controller: {balance_data.get('controller', 'N/A')}")
        
        # 2. 检查余额是否足够
        if side.lower() == "buy" and balance < amount:
            print(f"\n⚠️ 警告: 余额 (${balance:.2f}) 可能不足以支付 ${amount:.2f}")
        
        # 3. 执行下单
        print("\n2. 执行市价下单...")
        print(f"   请求: {side.upper()} ${amount:.2f} @ token={token_id[:16]}...")
        
        result, elapsed_ms = await client.create_market_order(
            token_id=token_id,
            side=side,
            amount=amount
        )
        
        print("\n" + "=" * 60)
        print("下单结果")
        print("=" * 60)
        
        if result:
            success = result.get("success", False)
            if success:
                print(f"✅ 下单成功!")
                print(f"   订单 ID: {result.get('orderID', 'N/A')}")
                print(f"   状态: {result.get('status', 'N/A')}")
                print(f"   Taking Amount: {result.get('takingAmount', 'N/A')}")
                print(f"   Making Amount: {result.get('makingAmount', 'N/A')}")
                
                # 交易哈希
                tx_hashes = result.get("transactionsHashes", [])
                if tx_hashes:
                    print(f"   交易哈希: {tx_hashes}")
            else:
                print(f"❌ 下单失败!")
                print(f"   错误信息: {result.get('errorMsg', 'Unknown error')}")
        else:
            print("❌ 下单失败! (无响应)")
        
        print(f"\n⏱️  下单耗时: {elapsed_ms:.2f} ms")
        print("=" * 60)
        
        # 4. 再次获取余额
        print("\n3. 获取最新余额...")
        new_balance_data = await client.get_balance()
        if new_balance_data:
            new_balance = new_balance_data.get("balance", 0)
            print(f"   当前余额: ${new_balance:.2f} USDC")
            diff = new_balance - balance
            if abs(diff) > 0.001:
                print(f"   余额变化: {'+' if diff > 0 else ''}{diff:.2f} USDC")


def main():
    parser = argparse.ArgumentParser(
        description="Polymarket 市价订单测试脚本",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
    python test_poly_order.py 12345...abc --side buy --amount 1
    python test_poly_order.py 12345...abc --side sell --amount 2
        """
    )
    
    parser.add_argument(
        "token_id",
        help="Token ID (从市场数据中获取)"
    )
    parser.add_argument(
        "--side",
        choices=["buy", "sell"],
        default="buy",
        help="下单方向: buy 或 sell (默认: buy)"
    )
    parser.add_argument(
        "--amount",
        type=float,
        default=1.0,
        help="下单金额 USDC (默认: 1.0)"
    )
    
    args = parser.parse_args()
    
    # 验证 amount
    if args.amount <= 0:
        print("错误: amount 必须 > 0")
        sys.exit(1)
    
    # 运行测试
    asyncio.run(test_order(
        token_id=args.token_id,
        side=args.side,
        amount=args.amount
    ))


if __name__ == "__main__":
    main()
