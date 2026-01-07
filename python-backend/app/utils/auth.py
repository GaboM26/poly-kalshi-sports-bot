"""认证工具模块"""
from datetime import datetime, timedelta
from typing import Optional
from jose import JWTError, jwt
from passlib.context import CryptContext
from app.core.config import AuthConfig

# 密码加密上下文
pwd_context = CryptContext(schemes=["bcrypt"], deprecated="auto")


def verify_password(plain_password: str, hashed_password: str) -> bool:
    """验证密码"""
    return pwd_context.verify(plain_password, hashed_password)


def get_password_hash(password: str) -> str:
    """获取密码哈希"""
    return pwd_context.hash(password)


def create_access_token(data: dict, auth_config: AuthConfig) -> str:
    """创建访问令牌
    
    Args:
        data: 要编码的数据（通常包含 sub: username）
        auth_config: 认证配置
        
    Returns:
        JWT token 字符串
    """
    to_encode = data.copy()
    expire = datetime.utcnow() + timedelta(hours=auth_config.token_expire_hours)
    to_encode.update({"exp": expire})
    
    encoded_jwt = jwt.encode(
        to_encode, 
        auth_config.secret_key, 
        algorithm="HS256"
    )
    return encoded_jwt


def verify_token(token: str, auth_config: AuthConfig) -> Optional[str]:
    """验证令牌
    
    Args:
        token: JWT token
        auth_config: 认证配置
        
    Returns:
        用户名，如果验证失败返回 None
    """
    try:
        payload = jwt.decode(
            token, 
            auth_config.secret_key, 
            algorithms=["HS256"]
        )
        username: str = payload.get("sub")
        if username is None:
            return None
        return username
    except JWTError:
        return None
