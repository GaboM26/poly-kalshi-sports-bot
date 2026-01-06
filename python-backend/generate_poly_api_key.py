"""
生成 Polymarket API 凭据

根据官方文档: https://docs.polymarket.com/quickstart/first-order
使用私钥派生 API 凭据（API Key, Secret, Passphrase）

使用方法:
    python generate_poly_api_key.py <your_private_key>

示例:
    python generate_poly_api_key.py 0x1234567890abcdef...
    python generate_poly_api_key.py 1234567890abcdef...  (不带0x前缀也可以)
"""

import asyncio
import sys
from py_clob_client.client import ClobClient
from py_clob_client.clob_types import ApiCreds
from eth_account import Account

# 配置
HOST = "https://clob.polymarket.com"
CHAIN_ID = 137  # Polygon mainnet

# 签名类型说明:
# 0 = EOA (使用独立的 EOA 钱包)
# 1 = POLY_PROXY (通过 Polymarket.com 账户 - Magic Link/Google 登录)
# 2 = GNOSIS_SAFE (通过 Polymarket.com 账户 - 浏览器钱包连接)
SIGNATURE_TYPE = 0  # 默认使用 EOA


def generate_api_credentials(private_key: str):
    """生成 Polymarket API 凭据"""
    
    # 确保私钥格式正确
    if not private_key.startswith("0x"):
        private_key = "0x" + private_key
    
    print("=" * 60)
    print("🔐 Polymarket API 凭据生成工具")
    print("=" * 60)
    
    try:
        # 2. 从私钥创建账户
        account = Account.from_key(private_key)
        wallet_address = account.address
        print(f"\n✅ 钱包地址: {wallet_address}")
        
        # 3. 初始化客户端
        print(f"\n🔌 连接到 Polymarket CLOB...")
        print(f"   Host: {HOST}")
        print(f"   Chain ID: {CHAIN_ID}")
        
        client = ClobClient(
            host=HOST,
            chain_id=CHAIN_ID,
            key=private_key,
            signature_type=SIGNATURE_TYPE,
            funder=wallet_address  # 对于 EOA，funder 就是钱包地址
        )
        
        # 4. 派生 API 凭据
        print(f"\n🔑 正在派生 API 凭据...")
        print(f"   签名类型: {SIGNATURE_TYPE} (EOA)")
        
        # 调用 create_or_derive_api_creds (同步方法)
        creds = client.create_or_derive_api_creds()
        
        if not creds:
            print("❌ 错误: 无法生成 API 凭据")
            return
        
        # 5. 显示生成的凭据
        print("\n" + "=" * 60)
        print("✅ API 凭据生成成功!")
        print("=" * 60)
        print(f"\nAPI Key:     {creds.api_key}")
        print(f"API Secret:  {creds.api_secret}")
        print(f"Passphrase:  {creds.api_passphrase}")
        
        # 6. 生成配置文件格式
        print("\n" + "=" * 60)
        print("📝 添加到 config.toml 的配置:")
        print("=" * 60)
        print(f"""
[polymarket]
# CLOB API 凭据（使用 generate_poly_api_key.py 生成）
api_key = "{creds.api_key}"
api_secret = "{creds.api_secret}"
api_passphrase = "{creds.api_passphrase}"
base_url = "https://gamma-api.polymarket.com"
clob_url = "https://clob.polymarket.com"
# 钱包地址（用于获取账户余额和交易）
wallet_address = "{wallet_address}"
# 钱包私钥（必需！用于 Level 2 认证和余额查询）
private_key = "{private_key}"
# 签名类型: 0=EOA钱包, 1=Polymarket账户(Magic Link), 2=Polymarket账户(浏览器钱包)
signature_type = {SIGNATURE_TYPE}
""")
        
        # 7. 测试凭据
        print("\n" + "=" * 60)
        print("🧪 测试 API 凭据...")
        print("=" * 60)
        
        # 重新初始化客户端使用完整凭据
        client_with_creds = ClobClient(
            host=HOST,
            chain_id=CHAIN_ID,
            key=private_key,
            creds=ApiCreds(
                api_key=creds.api_key,
                api_secret=creds.api_secret,
                api_passphrase=creds.api_passphrase
            ),
            signature_type=SIGNATURE_TYPE,
            funder=wallet_address
        )
        
        # 测试获取余额（需要异步）
        async def test_balance():
            try:
                balance_allowance = await client_with_creds.get_balance_allowance()
                print(f"✅ API 凭据有效!")
                print(f"   余额: ${float(balance_allowance.balance) / 1e6:.2f} USDC")
                print(f"   可用: ${float(balance_allowance.allowance) / 1e6:.2f} USDC")
            except Exception as e:
                print(f"⚠️  无法获取余额（可能需要充值）: {e}")
        
        # 运行异步测试
        asyncio.run(test_balance())
        
        print("\n" + "=" * 60)
        print("✅ 完成!")
        print("=" * 60)
        print("\n请将上述配置添加到 config.toml 文件中")
        print("注意: 请妥善保管这些凭据，不要泄露给他人!")
        
    except Exception as e:
        print(f"\n❌ 错误: {e}")
        import traceback
        traceback.print_exc()


if __name__ == "__main__":
    # 检查依赖
    try:
        import py_clob_client
        import eth_account
    except ImportError:
        print("❌ 缺少依赖库，请先安装:")
        print("\n  pip install py-clob-client eth-account")
        exit(1)
    
    # 检查命令行参数
    if len(sys.argv) < 2:
        print("=" * 60)
        print("🔐 Polymarket API 凭据生成工具")
        print("=" * 60)
        print("\n❌ 错误: 缺少私钥参数")
        print("\n使用方法:")
        print("  python generate_poly_api_key.py <your_private_key>")
        print("\n示例:")
        print("  python generate_poly_api_key.py 0x1234567890abcdef...")
        print("  python generate_poly_api_key.py 1234567890abcdef...  (不带0x前缀也可以)")
        print("\n⚠️  安全提示:")
        print("  - 私钥是敏感信息，请妥善保管")
        print("  - 建议在私密环境下运行此脚本")
        print("  - 运行后可以清除终端历史: history -c")
        exit(1)
    
    # 获取私钥
    private_key = sys.argv[1]
    
    # 运行（现在是同步函数）
    generate_api_credentials(private_key)
