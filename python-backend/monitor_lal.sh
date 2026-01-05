#!/bin/bash
# 监控 LAL-NOP 比赛的脚本

cd /Users/meloner/rustcode/polytaoli/python-backend
source venv/bin/activate

echo "开始监控 LAL-NOP 比赛..."
echo "日志文件: /tmp/kalshi_monitor.log"
echo ""

python kalshi_orderbook_monitor.py --ticker KXNBAGAME-26JAN06LALNOP-LAL KXNBAGAME-26JAN06LALNOP-NOP
