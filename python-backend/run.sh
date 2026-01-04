#!/bin/bash

# 启动 Python 后端服务器

echo "🚀 启动预测市场套利扫描器 (Python 版本)"

# 检查虚拟环境
if [ ! -d "venv" ]; then
    echo "📦 创建虚拟环境..."
    python3 -m venv venv
fi

# 激活虚拟环境
echo "🔧 激活虚拟环境..."
source venv/bin/activate

# 安装依赖
echo "📥 安装依赖..."
pip install -r requirements.txt

# 启动服务器
echo "🌐 启动服务器..."
python main.py
