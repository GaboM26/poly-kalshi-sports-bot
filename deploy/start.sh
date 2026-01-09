#!/bin/bash
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

# 启动程序
echo "🚀 启动 Polytaoli..."
./polytaoli
