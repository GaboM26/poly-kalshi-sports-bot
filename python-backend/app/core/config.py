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
    """Polymarket API 配置"""
    api_key: str = ""
    base_url: str = "https://gamma-api.polymarket.com"
    clob_url: str = "https://clob.polymarket.com"


class SettingsConfig(BaseModel):
    """应用设置"""
    refresh_interval: int = 5
    min_profit_margin: float = 1.0
    default_bet_amount: float = 100.0


class Config(BaseModel):
    """主配置类"""
    kalshi: KalshiConfig
    polymarket: PolymarketConfig
    settings: SettingsConfig

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
