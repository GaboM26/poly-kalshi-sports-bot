"""套利计算引擎

套利原理:
- 如果两个平台对同一队伍获胜的定价不同，可能存在套利机会
- 套利条件: 两个平台的价格之和 < 1 (隐含概率总和 < 100%)

价格语义（统一为 Yes/No 视角）:
- Kalshi MEM Yes: 买 MEM 获胜
- Kalshi MEM No: 买 MEM 不获胜
- Poly MEM Yes: 买 MEM 获胜 (= prices[0] 如果 MEM 是 team_a)
- Poly MEM No: 买 MEM 不获胜 (= prices[1] = LAL 获胜)

Kalshi Trading Fees:
- fees = round up(0.07 x C x P x (1-P))
- P = 合约价格（美元），例如 50 cents = 0.5
- C = 交易的合约数量
- round up = 向上取整到下一个美分
"""
import logging
import math
from typing import Optional
from datetime import datetime
from .models import KalshiMarket, PolymarketMarket, ArbitrageOpportunity

logger = logging.getLogger(__name__)


class ArbitrageCalculator:
    """套利计算器"""
    
    # Kalshi Trading Fee 费率
    KALSHI_TRADING_FEE_RATE = 0.07
    
    def __init__(self, min_profit_margin: float = 1.0, default_bet_amount: float = 100.0):
        self.min_profit_margin = min_profit_margin
        self.default_bet_amount = default_bet_amount
    
    def _calculate_kalshi_trading_fee(self, contracts: float, price: float) -> float:
        """计算 Kalshi Trading Fee
        
        公式: fees = round up(0.07 x C x P x (1-P))
        
        Args:
            contracts: 合约数量 C
            price: 合约价格 P（美元，例如 0.45 = 45 cents）
            
        Returns:
            费用金额（美元），向上取整到美分
        """
        if contracts <= 0 or price <= 0 or price >= 1:
            return 0.0
        
        # 计算原始费用
        raw_fee = self.KALSHI_TRADING_FEE_RATE * contracts * price * (1 - price)
        
        # 向上取整到美分 (round up to next cent)
        fee = math.ceil(raw_fee * 100) / 100
        
        return fee
    
    def calculate_single(
        self,
        event_name: str,
        team_name: str,
        kalshi_market: KalshiMarket,
        kalshi_yes_price: float,
        kalshi_no_price: float,
        polymarket_market: PolymarketMarket,
        polymarket_yes_price: float,
        polymarket_no_price: float
    ) -> Optional[ArbitrageOpportunity]:
        """计算单个配对市场的套利机会
        
        所有价格都已经统一为同一队伍的视角:
        - kalshi_yes_price: Kalshi 上买 "该队伍获胜" 的价格
        - kalshi_no_price: Kalshi 上买 "该队伍不获胜" 的价格
        - polymarket_yes_price: Poly 上买 "该队伍获胜" 的价格
        - polymarket_no_price: Poly 上买 "该队伍不获胜" 的价格
        """
        # 验证价格有效性
        if not self._validate_prices(kalshi_yes_price, kalshi_no_price, 
                                      polymarket_yes_price, polymarket_no_price):
            return None
        
        best_opportunity = None
        
        # 策略 1: Kalshi Yes + Polymarket No
        # 在 Kalshi 买该队获胜，在 Poly 买该队不获胜（即对手获胜）
        opp1 = self._calculate_strategy(
            event_name=event_name,
            team_name=team_name,
            kalshi_market=kalshi_market,
            kalshi_price=kalshi_yes_price,
            kalshi_side="yes",
            kalshi_yes_price=kalshi_yes_price,
            kalshi_no_price=kalshi_no_price,
            polymarket_market=polymarket_market,
            polymarket_price=polymarket_no_price,
            polymarket_side="no",
            polymarket_yes_price=polymarket_yes_price,
            polymarket_no_price=polymarket_no_price
        )
        
        if opp1 and (not best_opportunity or opp1.profit_margin > best_opportunity.profit_margin):
            best_opportunity = opp1
        
        # 策略 2: Kalshi No + Polymarket Yes
        # 在 Kalshi 买该队不获胜，在 Poly 买该队获胜
        opp2 = self._calculate_strategy(
            event_name=event_name,
            team_name=team_name,
            kalshi_market=kalshi_market,
            kalshi_price=kalshi_no_price,
            kalshi_side="no",
            kalshi_yes_price=kalshi_yes_price,
            kalshi_no_price=kalshi_no_price,
            polymarket_market=polymarket_market,
            polymarket_price=polymarket_yes_price,
            polymarket_side="yes",
            polymarket_yes_price=polymarket_yes_price,
            polymarket_no_price=polymarket_no_price
        )
        
        if opp2 and (not best_opportunity or opp2.profit_margin > best_opportunity.profit_margin):
            best_opportunity = opp2
        
        return best_opportunity
    
    def _validate_prices(
        self,
        k_yes: float,
        k_no: float,
        p_yes: float,
        p_no: float
    ) -> bool:
        """验证价格有效性"""
        for price in [k_yes, k_no, p_yes, p_no]:
            if price <= 0.01 or price >= 0.99:
                return False
        return True
    
    def _calculate_strategy(
        self,
        event_name: str,
        team_name: str,
        kalshi_market: KalshiMarket,
        kalshi_price: float,
        kalshi_side: str,
        kalshi_yes_price: float,
        kalshi_no_price: float,
        polymarket_market: PolymarketMarket,
        polymarket_price: float,
        polymarket_side: str,
        polymarket_yes_price: float,
        polymarket_no_price: float
    ) -> Optional[ArbitrageOpportunity]:
        """计算单个策略的套利（含 Kalshi Trading Fee）"""
        
        # 计算隐含概率总和
        implied_prob_sum = kalshi_price + polymarket_price
        
        # 如果总和 >= 1，没有套利机会
        if implied_prob_sum >= 1.0:
            return None
        
        # 计算最优下注金额
        total_bet = self.default_bet_amount
        guaranteed_return = total_bet / implied_prob_sum
        
        kalshi_bet = guaranteed_return * kalshi_price
        polymarket_bet = guaranteed_return * polymarket_price
        
        # 计算 Kalshi 合约数量和交易费用
        # 合约数量 = 下注金额 / 合约价格
        kalshi_contracts = kalshi_bet / kalshi_price if kalshi_price > 0 else 0
        kalshi_fee = self._calculate_kalshi_trading_fee(kalshi_contracts, kalshi_price)
        
        # 计算扣除费用后的预期利润
        gross_profit = guaranteed_return - total_bet
        expected_profit = gross_profit - kalshi_fee
        
        # 计算扣除费用后的利润率
        profit_margin = (expected_profit / total_bet) * 100.0 if total_bet > 0 else 0.0
        
        # 检查是否满足最小利润率（扣除费用后）
        if profit_margin < self.min_profit_margin:
            return None
        
        logger.debug(f"💰 套利机会: {event_name} - {team_name}")
        logger.debug(f"   Kalshi {kalshi_side}: {kalshi_price:.2f}, Poly {polymarket_side}: {polymarket_price:.2f}")
        logger.debug(f"   合约数: {kalshi_contracts:.0f}, Kalshi费用: ${kalshi_fee:.2f}")
        logger.debug(f"   利润率: {profit_margin:.2f}%, 预期利润: ${expected_profit:.2f} (毛利: ${gross_profit:.2f})")
        
        return ArbitrageOpportunity(
            event_name=event_name,
            team_name=team_name,
            kalshi_market_id=kalshi_market.market_id,
            kalshi_price=kalshi_price,
            kalshi_side=kalshi_side,
            kalshi_bet=kalshi_bet,
            kalshi_yes_price=kalshi_yes_price,
            kalshi_no_price=kalshi_no_price,
            kalshi_contracts=kalshi_contracts,
            kalshi_fee=kalshi_fee,
            polymarket_market_id=polymarket_market.market_id,
            polymarket_price=polymarket_price,
            polymarket_side=polymarket_side,
            polymarket_bet=polymarket_bet,
            polymarket_yes_price=polymarket_yes_price,
            polymarket_no_price=polymarket_no_price,
            total_bet=total_bet,
            profit_margin=profit_margin,
            expected_profit=expected_profit,
            gross_profit=gross_profit,
            timestamp=datetime.now(),
            start_time=kalshi_market.start_time  # 从 Kalshi 市场获取开始时间
        )
