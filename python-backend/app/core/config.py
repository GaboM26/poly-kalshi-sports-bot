"""配置管理模块"""
import toml
from pathlib import Path
from pydantic import BaseModel
from typing import Optional


class KalshiConfig(BaseModel):
    """Kalshi API 配置"""
    api_key: str
    api_secret: str
    base_url: str = "https://api.elections.kalshi.com/trade-api/v2"


class PolymarketConfig(BaseModel):
    """Polymarket API 配置
    
    对于 Magic Link 用户，只需配置:
    - private_key: Controller 私钥（从 https://reveal.magic.link/polymarket 获取）
    - wallet_address: Smart Wallet 地址（从 polymarket.com/settings 查看）
    
    API 凭据会自动派生，无需手动配置
    """
    # 必需配置
    private_key: str = ""  # Controller 私钥
    wallet_address: str = ""  # Smart Wallet 地址
    
    # 可选配置（有默认值）
    base_url: str = "https://gamma-api.polymarket.com"
    clob_url: str = "https://clob.polymarket.com"
    signature_type: int = 1  # 1=Magic Link 用户
    
    # API 凭据（自动派生，无需手动配置）
    api_key: str = ""
    api_secret: str = ""
    api_passphrase: str = ""


class SettingsConfig(BaseModel):
    """应用设置"""
    refresh_interval: int = 5
    min_profit_margin: float = 1.0
    default_bet_amount: float = 100.0


class AuthConfig(BaseModel):
    """认证配置"""
    username: str = "admin"
    password: str = "admin123"
    secret_key: str = "your-secret-key-change-this-in-production"
    token_expire_hours: int = 24


class Config(BaseModel):
    """主配置类"""
    kalshi: KalshiConfig
    polymarket: PolymarketConfig
    settings: SettingsConfig
    auth: AuthConfig = AuthConfig()  # 默认值，兼容旧配置

    @classmethod
    def from_file(cls, config_path: str = "config.toml") -> "Config":
        """从 TOML 文件加载配置"""
        # 从项目根目录（python-backend）查找配置文件
        if Path(config_path).is_absolute():
            path = Path(config_path)
        else:
            # 从当前文件向上两级到 python-backend 目录
            path = Path(__file__).parent.parent.parent / config_path
        
        with open(path, 'r') as f:
            data = toml.load(f)
        return cls(**data)

    def validate_config(self):
        """验证配置"""
        if not self.kalshi.api_key:
            raise ValueError("Kalshi API key 未配置")
        if not self.kalshi.api_secret:
            raise ValueError("Kalshi API secret 未配置")
        return True
