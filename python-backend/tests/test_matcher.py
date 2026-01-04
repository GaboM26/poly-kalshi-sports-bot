"""匹配器测试"""
import pytest
from app.core.matcher import EventMatcher
from app.core.models import KalshiEvent, PolymarketEvent


def test_event_matcher_initialization():
    """测试匹配器初始化"""
    matcher = EventMatcher()
    assert matcher is not None
    assert matcher.time_tolerance_hours == 24


def test_normalize_team_name():
    """测试队伍名称标准化"""
    from app.utils.helpers import normalize_team_name
    
    assert normalize_team_name("lakers") == "LAKERS"
    assert normalize_team_name(" Lakers ") == "LAKERS"
    assert normalize_team_name("LAL") == "LAL"


# 更多测试待添加...
