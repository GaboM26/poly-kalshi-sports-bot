"""辅助函数"""
from datetime import datetime
from typing import Any, Dict


def serialize_datetime(obj: Any) -> Any:
    """序列化 datetime 对象为 ISO 格式字符串"""
    if isinstance(obj, datetime):
        return obj.isoformat()
    return obj


def safe_float(value: Any, default: float = 0.0) -> float:
    """安全地转换为浮点数"""
    try:
        return float(value)
    except (TypeError, ValueError):
        return default


def normalize_team_name(name: str) -> str:
    """标准化队伍名称"""
    return name.strip().upper()
