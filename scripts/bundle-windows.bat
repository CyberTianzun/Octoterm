@echo off
REM Windows 侧不需要 bundle:单个 exe 就是全部产物(图标已嵌进二进制)。
REM 这个脚本只负责把它挪到统一的输出目录,和 macOS 那边对齐。
setlocal
set TARGET=%1
if "%TARGET%"=="" set TARGET=x86_64-pc-windows-msvc
set ROOT=%~dp0..
set BIN=%ROOT%\target\%TARGET%\release\octoterm-desktop.exe
if not exist "%BIN%" (
  echo 找不到 %BIN%,先跑 cargo build --release --target %TARGET% -p octoterm-desktop 1>&2
  exit /b 1
)
if not exist "%ROOT%\target\bundle" mkdir "%ROOT%\target\bundle"
copy /Y "%BIN%" "%ROOT%\target\bundle\octoterm-desktop.exe" >nul
echo %ROOT%\target\bundle\octoterm-desktop.exe
