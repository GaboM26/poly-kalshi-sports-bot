#!/bin/bash

# 启动 Python 后端服务器（简化版）

cd "$(dirname "$0")"

echo "🚀 启动预测市场套利扫描器 (Python 版本)"

# 检查虚拟环境
if [ ! -d "venv" ]; then
    echo "❌ 虚拟环境不存在，请先运行: python3 -m venv venv && source venv/bin/activate && pip install -r requirements.txt"
    exit 1
fi

# 激活虚拟环境并启动服务器
source venv/bin/activate
echo "🌐 启动服务器在 http://localhost:3000"
python main.py
