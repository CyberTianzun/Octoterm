@echo off
REM 构建 web 客户端到 clients\web\dist\ —— octoterm-server 直接从那个目录发静态文件。
REM
REM 与 build-frontend.sh 等价,改动请同步两边。
setlocal

REM %~dp0 是脚本所在目录(自带结尾反斜杠),相对它定位而不是相对当前目录;
REM setlocal + /d 保证切盘也能切,且不会把工作目录泄漏回调用方的 shell。
cd /d "%~dp0clients\web" || exit /b 1

REM 必须加 call:Windows 上 npm 是 npm.cmd(批处理文件),.bat 里直接调用另一个
REM 批处理会**移交控制权且永不返回**,后面的命令一行都不会执行 —— 于是 build
REM 被静默跳过,脚本还报成功。
call npm install || exit /b 1
call npm run build || exit /b 1
