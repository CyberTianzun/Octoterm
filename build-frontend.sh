#!/usr/bin/env bash
# 构建 web 客户端到 clients/web/dist/ —— octoterm-server 直接从那个目录发静态文件。
#
# 与 build-frontend.bat 等价,改动请同步两边。
set -euo pipefail

# 相对脚本自身定位,而不是相对当前目录:在仓库任何位置执行都对
cd "$(dirname "$0")/clients/web"

npm install
npm run build
