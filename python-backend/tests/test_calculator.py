"""套利计算器测试"""
import pytest
from app.core.calculator import ArbitrageCalculator


def test_calculator_initialization():
    """测试计算器初始化"""
    calc = ArbitrageCalculator(min_profit_margin=1.0, default_bet_amount=100.0)
    assert calc is not None
    assert calc.min_profit_margin == 1.0
    assert calc.default_bet_amount == 100.0


def test_price_validation():
    """测试价格验证"""
    calc = ArbitrageCalculator()
    
    # 有效价格
    assert calc._validate_prices(0.5, 0.5, 0.4, 0.6) == True
    
    # 无效价格（太低）
    assert calc._validate_prices(0.005, 0.5, 0.4, 0.6) == False
    
    # 无效价格（太高）
    assert calc._validate_prices(0.995, 0.5, 0.4, 0.6) == False


# 更多测试待添加...
