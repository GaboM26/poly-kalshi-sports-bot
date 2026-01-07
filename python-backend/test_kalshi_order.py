#!/opt/anaconda3/bin/python
"""Kalshi 市价订单测试脚本

使用方法:
    python test_kalshi_order.py <ticker> [--side yes|no] [--action buy|sell] [--count N]

示例:
    # 买入 1 个 yes 合约
    python test_kalshi_order.py KXNBAGAME-26JAN07CLELAL-CLE --side yes --action buy
    
    # 买入 2 个 no 合约
    python test_kalshi_order.py KXNBAGAME-26JAN07CLELAL-CLE --side no --action buy --count 2
"""
import asyncio
import argparse
import logging
import sys
from pathlib import Path

# 添加项目路径
sys.path.insert(0, str(Path(__file__).parent))

from app.core.config import Config
from app.clients.kalshi import KalshiClient

# 配置日志
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(levelname)s - %(message)s'
)
logger = logging.getLogger(__name__)


async def test_order(ticker: str, side: str, action: str, count: int):
    """执行测试下单"""
    
    # 加载配置
    config = Config.from_file()
    
    print("=" * 60)
    print("Kalshi 市价订单测试")
    print("=" * 60)
    print(f"市场 Ticker: {ticker}")
    print(f"方向 (side): {side}")
    print(f"操作 (action): {action}")
    print(f"数量 (count): {count}")
    print("=" * 60)
    
    async with KalshiClient(config.kalshi) as client:
        # 先测试连接
        print("\n1. 测试 API 连接...")
        if not await client.login():
            print("❌ API 连接失败，请检查配置")
            return
        print("✅ API 连接成功")
        
        # 获取余额
        print("\n2. 获取账户余额...")
        balance_data = await client.get_balance()
        if balance_data:
            balance_cents = balance_data.get("balance", 0)
            portfolio_value = balance_data.get("portfolio_value", 0)
            print(f"✅ 可用余额: ${balance_cents / 100:.2f}")
            print(f"   持仓价值: ${portfolio_value / 100:.2f}")
        else:
            print("⚠️ 无法获取余额")
        
        # 执行下单
        print("\n3. 执行市价下单...")
        print(f"   请求: {action.upper()} {count}x {side.upper()} @ {ticker}")
        
        result, elapsed_ms = await client.create_market_order(
            ticker=ticker,
            side=side,
            action=action,
            count=count
        )
        
        print("\n" + "=" * 60)
        print("下单结果")
        print("=" * 60)
        
        if result:
            order = result.get("order", {})
            print(f"✅ 下单成功!")
            print(f"   订单 ID: {order.get('order_id', 'N/A')}")
            print(f"   状态: {order.get('status', 'N/A')}")
            print(f"   成交数量: {order.get('fill_count', 0)}")
            print(f"   剩余数量: {order.get('remaining_count', 0)}")
            
            # 费用信息
            taker_fees = order.get('taker_fees', 0)
            taker_fill_cost = order.get('taker_fill_cost', 0)
            if taker_fees or taker_fill_cost:
                print(f"   Taker 费用: ${taker_fees / 100:.2f}")
                print(f"   Taker 成交成本: ${taker_fill_cost / 100:.2f}")
            
            # 时间信息
            created_time = order.get('created_time', 'N/A')
            print(f"   创建时间: {created_time}")
        else:
            print("❌ 下单失败!")
        
        print(f"\n⏱️  下单耗时: {elapsed_ms:.2f} ms")
        print("=" * 60)


def main():
    parser = argparse.ArgumentParser(
        description="Kalshi 市价订单测试脚本",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
示例:
    python test_kalshi_order.py KXNBAGAME-26JAN07CLELAL-CLE --side yes --action buy
    python test_kalshi_order.py KXNBAGAME-26JAN07CLELAL-CLE --side no --action buy --count 2
        """
    )
    
    parser.add_argument(
        "ticker",
        help="市场 ticker (如 KXNBAGAME-26JAN07CLELAL-CLE)"
    )
    parser.add_argument(
        "--side",
        choices=["yes", "no"],
        default="yes",
        help="下单方向: yes 或 no (默认: yes)"
    )
    parser.add_argument(
        "--action",
        choices=["buy", "sell"],
        default="buy",
        help="操作类型: buy 或 sell (默认: buy)"
    )
    parser.add_argument(
        "--count",
        type=int,
        default=1,
        help="合约数量 (默认: 1，约 10 美分)"
    )
    
    args = parser.parse_args()
    
    # 验证 count
    if args.count < 1:
        print("错误: count 必须 >= 1")
        sys.exit(1)
    
    # 运行测试
    asyncio.run(test_order(
        ticker=args.ticker,
        side=args.side,
        action=args.action,
        count=args.count
    ))


if __name__ == "__main__":
    main()
