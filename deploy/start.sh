#!/bin/bash

# 存储所有进程 PID
PIDS=""

# 捕获 Ctrl+C 信号
trap "echo ''; echo '🛑 正在停止服务...'; kill $PIDS 2>/dev/null; exit 0" INT

# 确保可执行权限
chmod +x polytaoli

# 检查配置文件
if [ ! -f config.toml ]; then
    echo "❌ 错误: 未找到 config.toml"
    echo "请复制 config.example.toml 为 config.toml 并配置"
    exit 1
fi

# 创建日志目录
mkdir -p logs

# 启动 Python 下单服务
echo "🐍 启动 Python 下单服务 (端口 8001)..."
cd poly-order-service

# 检查 Python 配置
if [ ! -f config.toml ]; then
    echo "⚠️  警告: poly-order-service/config.toml 不存在"
    if [ -f config.toml.sample ]; then
        echo "请复制 config.toml.sample 为 config.toml 并配置"
    fi
    echo "Python 下单服务将无法启动"
else
    # 检查 Python 虚拟环境
    if [ ! -d ".venv" ]; then
        echo "📦 创建 Python 虚拟环境..."
        python3 -m venv .venv
        source .venv/bin/activate
        pip install -r requirements.txt
    else
        source .venv/bin/activate
    fi
    
    # 启动 Python 服务
    python main.py &
    PYTHON_PID=$!
    PIDS="$PYTHON_PID"
    echo "✅ Python 下单服务已启动 (PID: $PYTHON_PID)"
    
    # 等待 Python 服务启动
    sleep 3
fi

cd ..

# 启动 Rust 后端
echo "🚀 启动 Rust 后端 (端口 8000)..."
./polytaoli &
RUST_PID=$!
PIDS="$PIDS $RUST_PID"
echo "✅ Rust 后端已启动 (PID: $RUST_PID)"

echo ""
echo "=================================="
echo "✅ 启动完成！"
echo ""
echo "🐍 Python 下单服务: http://localhost:8001"
echo "📊 Rust 后端: http://localhost:8000"
echo ""
echo "按 Ctrl+C 停止所有服务"
echo "=================================="

# 等待
wait
