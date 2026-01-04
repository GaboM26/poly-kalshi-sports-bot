#!/bin/bash
# 启动脚本 - 模块化版本

echo "🚀 启动预测市场套利扫描器 (模块化版)"
echo "=" 

# 检查虚拟环境
if [ ! -d "venv" ]; then
    echo "❌ 虚拟环境不存在，请先运行: python3 -m venv venv && source venv/bin/activate && pip install -r requirements.txt"
    exit 1
fi

# 激活虚拟环境
source venv/bin/activate

# 检查依赖
python -c "import fastapi" 2>/dev/null
if [ $? -ne 0 ]; then
    echo "📦 安装依赖..."
    pip install -r requirements.txt
fi

# 启动服务器
echo "🌐 启动服务器 http://localhost:3000"
python -m app.main
