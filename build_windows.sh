#!/bin/bash

set -e  # 遇到错误立即退出

echo "🚀 开始构建 Polytaoli Windows x86_64 版本..."

# 颜色定义
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# 1. 构建前端
echo -e "${BLUE}📦 步骤 1/4: 构建前端...${NC}"
cd web
npm run build
cd ..
# 复制前端文件到 rust-backend/static 目录（用于嵌入到二进制文件）
rm -rf rust-backend/static
cp -r web/dist rust-backend/static
echo -e "${GREEN}✅ 前端构建完成${NC}"

# 2. 检查并安装交叉编译工具
echo -e "${BLUE}🔧 步骤 2/4: 检查交叉编译工具...${NC}"
if ! command -v cross &> /dev/null; then
    echo -e "${YELLOW}正在安装 cross...${NC}"
    cargo install cross --git https://github.com/cross-rs/cross
fi

if ! rustup target list | grep -q "x86_64-pc-windows-gnu (installed)"; then
    echo -e "${YELLOW}正在添加 Windows x86_64 目标...${NC}"
    rustup target add x86_64-pc-windows-gnu
fi
echo -e "${GREEN}✅ 工具检查完成${NC}"

# 3. 交叉编译 Rust 后端
echo -e "${BLUE}⚙️  步骤 3/4: 交叉编译 Rust 程序...${NC}"
cd rust-backend
# 使用 vendored OpenSSL 避免依赖系统 OpenSSL
OPENSSL_STATIC=1 OPENSSL_VENDORED=1 cross build --release --target x86_64-pc-windows-gnu
cd ..
echo -e "${GREEN}✅ 编译完成${NC}"

# 4. 打包部署文件
echo -e "${BLUE}📦 步骤 4/4: 打包部署文件...${NC}"
rm -rf deploy-windows
mkdir -p deploy-windows

# 复制二进制文件
cp rust-backend/target/x86_64-pc-windows-gnu/release/polytaoli.exe deploy-windows/

# 复制配置文件
cp rust-backend/config.example.toml deploy-windows/
if [ -f rust-backend/config.toml ]; then
    cp rust-backend/config.toml deploy-windows/config.toml.sample
fi

# 创建启动脚本（Windows batch）
cat > deploy-windows/start.bat << 'EOF'
@echo off
REM 检查配置文件
if not exist config.toml (
    echo 错误: 未找到 config.toml
    echo 请复制 config.example.toml 为 config.toml 并配置
    pause
    exit /b 1
)

REM 创建日志目录
if not exist logs mkdir logs

REM 启动程序
echo 启动 Polytaoli...
polytaoli.exe
pause
EOF

# 创建 PowerShell 启动脚本（更现代）
cat > deploy-windows/start.ps1 << 'EOF'
# 检查配置文件
if (-not (Test-Path "config.toml")) {
    Write-Host "错误: 未找到 config.toml" -ForegroundColor Red
    Write-Host "请复制 config.example.toml 为 config.toml 并配置"
    Read-Host "按回车键退出"
    exit 1
}

# 创建日志目录
if (-not (Test-Path "logs")) {
    New-Item -ItemType Directory -Path "logs" | Out-Null
}

# 启动程序
Write-Host "启动 Polytaoli..." -ForegroundColor Green
.\polytaoli.exe
EOF

# 创建 README
cat > deploy-windows/README.txt << 'EOF'
Polytaoli - 预测市场套利扫描器 (Windows 版本)
================================================

部署步骤:
1. 复制 config.example.toml 为 config.toml
2. 编辑 config.toml，填入你的 API 密钥
3. 运行启动脚本:
   - 双击 start.bat (命令提示符)
   - 或右键 start.ps1 -> 使用 PowerShell 运行

配置说明:
- 端口: 默认 8000
- 日志: 保存在 logs\ 目录
- 前端: 访问 http://localhost:8000

停止程序: 关闭命令行窗口或 Ctrl+C

系统要求:
- Windows 10/11 或 Windows Server 2016+
- x86_64 架构
EOF

# 打包成 zip
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
PACKAGE_NAME="polytaoli-windows-x86_64-${TIMESTAMP}.zip"

# 检查是否有 zip 命令
if command -v zip &> /dev/null; then
    zip -r "$PACKAGE_NAME" deploy-windows/
else
    echo -e "${YELLOW}未找到 zip 命令，使用 tar 打包...${NC}"
    tar -czf "${PACKAGE_NAME%.zip}.tar.gz" deploy-windows/
    PACKAGE_NAME="${PACKAGE_NAME%.zip}.tar.gz"
fi

echo -e "${GREEN}✅ 打包完成!${NC}"
echo ""
echo "📦 部署包: $PACKAGE_NAME"
echo "📁 大小: $(du -h "$PACKAGE_NAME" | cut -f1)"
echo ""
echo "部署到 Windows 服务器:"
echo "  1. 解压文件"
echo "  2. 进入 deploy-windows 目录"
echo "  3. 复制 config.example.toml 为 config.toml 并编辑"
echo "  4. 双击 start.bat 或运行 start.ps1"
echo ""
echo -e "${GREEN}🎉 完成!${NC}"
