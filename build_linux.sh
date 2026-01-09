#!/bin/bash

set -e  # 遇到错误立即退出

echo "🚀 开始构建 Polytaoli Linux x86_64 版本..."

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

if ! rustup target list | grep -q "x86_64-unknown-linux-gnu (installed)"; then
    echo -e "${YELLOW}正在添加 Linux x86_64 目标...${NC}"
    rustup target add x86_64-unknown-linux-gnu
fi
echo -e "${GREEN}✅ 工具检查完成${NC}"

# 3. 交叉编译 Rust 后端
echo -e "${BLUE}⚙️  步骤 3/4: 交叉编译 Rust 程序...${NC}"
cd rust-backend
# 使用 vendored OpenSSL 避免依赖系统 OpenSSL
OPENSSL_STATIC=1 OPENSSL_VENDORED=1 cross build --release --target x86_64-unknown-linux-gnu
cd ..
echo -e "${GREEN}✅ 编译完成${NC}"

# 4. 打包部署文件
echo -e "${BLUE}📦 步骤 4/4: 打包部署文件...${NC}"
rm -rf deploy
mkdir -p deploy

# 复制二进制文件
cp rust-backend/target/x86_64-unknown-linux-gnu/release/polytaoli deploy/

# 复制配置文件
cp rust-backend/config.example.toml deploy/
if [ -f rust-backend/config.toml ]; then
    cp rust-backend/config.toml deploy/config.toml.sample
fi

# 创建启动脚本
cat > deploy/start.sh << 'EOF'
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
EOF

chmod +x deploy/start.sh

# 创建 README
cat > deploy/README.txt << 'EOF'
Polytaoli - 预测市场套利扫描器
================================

部署步骤:
1. 复制 config.example.toml 为 config.toml
2. 编辑 config.toml，填入你的 API 密钥
3. 运行: ./start.sh

配置说明:
- 端口: 默认 8000
- 日志: 保存在 logs/ 目录
- 前端: 访问 http://your-server:8000

停止程序: Ctrl+C 或 kill 进程
EOF

# 打包
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
PACKAGE_NAME="polytaoli-linux-x86_64-${TIMESTAMP}.tar.gz"
tar -czf "$PACKAGE_NAME" deploy/

echo -e "${GREEN}✅ 打包完成!${NC}"
echo ""
echo "📦 部署包: $PACKAGE_NAME"
echo "📁 大小: $(du -h "$PACKAGE_NAME" | cut -f1)"
echo ""
echo "部署到 Linux 服务器:"
echo "  1. 上传: scp $PACKAGE_NAME user@server:/path/"
echo "  2. 解压: tar -xzf $PACKAGE_NAME"
echo "  3. 配置: cd deploy && cp config.example.toml config.toml && nano config.toml"
echo "  4. 启动: ./start.sh"
echo ""
echo -e "${GREEN}🎉 完成!${NC}"
