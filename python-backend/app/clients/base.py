"""基础客户端类"""
import logging
from abc import ABC, abstractmethod
from typing import Optional, List, Tuple
import aiohttp


class BaseAPIClient(ABC):
    """API 客户端基类
    
    提供通用的 HTTP 客户端功能和接口定义
    """
    
    def __init__(self, base_url: str, api_key: Optional[str] = None):
        """初始化客户端
        
        Args:
            base_url: API 基础 URL
            api_key: API 密钥（可选）
        """
        self.base_url = base_url.rstrip('/')
        self.api_key = api_key
        self.session: Optional[aiohttp.ClientSession] = None
        self.logger = logging.getLogger(self.__class__.__name__)
    
    async def _ensure_session(self):
        """确保 HTTP 会话已创建"""
        if self.session is None or self.session.closed:
            self.session = aiohttp.ClientSession()
    
    async def close(self):
        """关闭 HTTP 会话"""
        if self.session and not self.session.closed:
            await self.session.close()
    
    @abstractmethod
    async def get_nba_events_and_markets(self) -> Tuple[List, List]:
        """获取 NBA 事件和市场数据
        
        Returns:
            (events, markets) 元组
        """
        pass
    
    async def _get(self, endpoint: str, **kwargs) -> dict:
        """发送 GET 请求
        
        Args:
            endpoint: API 端点
            **kwargs: 额外的请求参数
        
        Returns:
            响应 JSON 数据
        """
        await self._ensure_session()
        url = f"{self.base_url}/{endpoint.lstrip('/')}"
        
        try:
            async with self.session.get(url, **kwargs) as response:
                response.raise_for_status()
                return await response.json()
        except Exception as e:
            self.logger.error(f"GET 请求失败: {url}, 错误: {e}")
            raise
    
    async def _post(self, endpoint: str, **kwargs) -> dict:
        """发送 POST 请求
        
        Args:
            endpoint: API 端点
            **kwargs: 额外的请求参数
        
        Returns:
            响应 JSON 数据
        """
        await self._ensure_session()
        url = f"{self.base_url}/{endpoint.lstrip('/')}"
        
        try:
            async with self.session.post(url, **kwargs) as response:
                response.raise_for_status()
                return await response.json()
        except Exception as e:
            self.logger.error(f"POST 请求失败: {url}, 错误: {e}")
            raise
