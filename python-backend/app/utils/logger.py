"""日志配置模块"""
import logging
import sys
from typing import Optional


def setup_logger(
    name: str = "arbitrage_scanner",
    level: int = logging.INFO,
    format_string: Optional[str] = None
) -> logging.Logger:
    """配置并返回日志记录器
    
    Args:
        name: 日志记录器名称
        level: 日志级别
        format_string: 自定义格式字符串
    
    Returns:
        配置好的日志记录器
    """
    if format_string is None:
        format_string = '%(asctime)s - %(name)s - %(levelname)s - %(message)s'
    
    # 配置根日志记录器
    logging.basicConfig(
        level=level,
        format=format_string,
        handlers=[
            logging.StreamHandler(sys.stdout)
        ]
    )
    
    logger = logging.getLogger(name)
    return logger
