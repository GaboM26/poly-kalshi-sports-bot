#!/bin/bash

# 测试日志配置脚本

echo "🧪 测试 Rust 后端日志配置"
echo "=================================="

# 清理旧日志
echo "📝 清理旧日志文件..."
rm -rf logs/
mkdir -p logs/

# 编译项目
echo "🔨 编译项目..."
cargo build --release

if [ $? -ne 0 ]; then
    echo "❌ 编译失败"
    exit 1
fi

echo "✅ 编译成功"
echo ""

# 启动服务器（后台运行）
echo "🚀 启动服务器..."
cargo run --release &
SERVER_PID=$!

echo "✅ 服务器已启动 (PID: $SERVER_PID)"
echo ""

# 等待服务器启动
echo "⏳ 等待服务器启动..."
sleep 5

# 检查日志文件是否创建
echo "📁 检查日志文件..."
if [ -f "logs/polytaoli.log" ]; then
    echo "✅ 日志文件已创建: logs/polytaoli.log"
else
    echo "❌ 日志文件未创建"
    kill $SERVER_PID 2>/dev/null
    exit 1
fi

echo ""
echo "📊 控制台日志示例（最近 10 行）:"
echo "=================================="
# 这里显示的是我们捕获的输出，实际运行时会在终端看到

echo ""
echo "📄 文件日志示例（最近 20 行）:"
echo "=================================="
tail -20 logs/polytaoli.log

echo ""
echo "📈 日志统计:"
echo "=================================="
echo "总行数: $(wc -l < logs/polytaoli.log)"
echo "INFO 日志: $(grep -c "INFO" logs/polytaoli.log)"
echo "DEBUG 日志: $(grep -c "DEBUG" logs/polytaoli.log)"
echo "WARN 日志: $(grep -c "WARN" logs/polytaoli.log)"
echo "ERROR 日志: $(grep -c "ERROR" logs/polytaoli.log)"

echo ""
echo "📂 日志文件大小:"
ls -lh logs/

echo ""
echo "✅ 日志测试完成！"
echo ""
echo "💡 提示:"
echo "  - 控制台只显示 INFO 及以上级别的日志"
echo "  - 文件包含所有 DEBUG 及以上级别的日志"
echo "  - 查看实时日志: tail -f logs/polytaoli.log"
echo "  - 搜索日志: grep '关键词' logs/polytaoli.log"
echo ""
echo "🛑 按 Ctrl+C 停止服务器"

# 等待用户中断
trap "echo ''; echo '🛑 正在停止服务器...'; kill $SERVER_PID 2>/dev/null; exit 0" INT
wait
