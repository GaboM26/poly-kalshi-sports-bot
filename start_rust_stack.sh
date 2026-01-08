#!/bin/bash

# Polytaoli Rust 后端 + 前端启动脚本

echo "🚀 启动 Polytaoli (Rust 后端版本)"
echo "=================================="

# 检查是否在正确的目录
if [ ! -d "rust-backend" ] || [ ! -d "web" ]; then
    echo "❌ 错误: 请在项目根目录运行此脚本"
    exit 1
fi

# 启动 Rust 后端
echo ""
echo "📦 启动 Rust 后端 (端口 8000)..."
cd rust-backend

# 检查配置文件
if [ ! -f "config.toml" ]; then
    echo "⚠️  警告: config.toml 不存在，从示例文件复制..."
    if [ -f "config.example.toml" ]; then
        cp config.example.toml config.toml
        echo "✅ 已创建 config.toml，请编辑配置文件后重新运行"
        exit 1
    else
        echo "❌ 错误: config.example.toml 也不存在"
        exit 1
    fi
fi

# 在后台启动 Rust 后端
cargo run --release &
RUST_PID=$!
echo "✅ Rust 后端已启动 (PID: $RUST_PID)"

# 等待后端启动
echo "⏳ 等待后端启动..."
sleep 5

# 检查后端是否正常运行
if ! curl -s http://localhost:8000/api/health > /dev/null; then
    echo "❌ 错误: Rust 后端启动失败"
    kill $RUST_PID 2>/dev/null
    exit 1
fi

echo "✅ Rust 后端健康检查通过"

# 启动前端
cd ../web
echo ""
echo "🌐 启动前端 (端口 5173)..."

# 检查 node_modules
if [ ! -d "node_modules" ]; then
    echo "📦 安装前端依赖..."
    npm install
fi

# 启动前端开发服务器
npm run dev &
WEB_PID=$!
echo "✅ 前端已启动 (PID: $WEB_PID)"

echo ""
echo "=================================="
echo "✅ 启动完成！"
echo ""
echo "📊 Rust 后端: http://localhost:8000"
echo "🌐 前端界面: http://localhost:5173"
echo ""
echo "按 Ctrl+C 停止所有服务"
echo "=================================="

# 捕获 Ctrl+C 信号
trap "echo ''; echo '🛑 正在停止服务...'; kill $RUST_PID $WEB_PID 2>/dev/null; exit 0" INT

# 等待
wait
