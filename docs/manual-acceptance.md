# octoterm 手动验收清单

前置:`cd clients/web && npm run build`,`cargo run -p octoterm-server`,记下打印的 token,
浏览器打开 `http://127.0.0.1:7683/#token=<token>`。

## 桌面浏览器
- [ ] 列表页显示会话卡片,预览可见
- [ ] New Session → 卡片实时出现;Rename/Kill 生效且另一浏览器标签页同步更新
- [ ] Attach → shell 可交互;`ls --color`、`vim`、`htop` 渲染正常
- [ ] 拉伸窗口 → 终端随 fit 重排,`tput cols` 值同步变化
- [ ] `cat` 一个数 MB 文件 → 页面不卡死,结束后画面正确(resync 生效)
- [ ] 杀掉网络(关 wifi 数秒再恢复)→ 顶部 reconnecting 条出现后消失,会话内容延续
- [ ] 关闭标签页重开 → 会话仍在,attach 恢复现场

## 多端同时 attach 一个会话(默认 window-size = smallest)
- [ ] 大窗口 attach 后,小窗口(另开一个窄浏览器窗口/手机)再 attach → 两边同时缩到
      小的那个尺寸,`tput cols` 一致;大窗口多出来的地方留白居中,画面不错行
- [ ] 拖动其中任一窗口 → 两边同步跟随,没有互相抢尺寸的抖动(vim/htop 不反复重绘)
- [ ] 小窗口那端 detach 或关掉标签页 → 大窗口立即恢复到自己的尺寸
      (直接断网这种半开连接要等心跳超时,最多 90s,见 protocol.md G8)
- [ ] `--window-size largest` / `latest` 启动 → 行为分别变为跟随最大 / 跟随最近操作的一端

## 移动浏览器(iOS Safari / Android Chrome)
- [ ] 列表页可读可点
- [ ] Attach 后软键盘弹出,终端随 visualViewport 缩放,输入回显正常
- [ ] 触摸滚动查看回滚历史
- [ ] 锁屏 30 秒回来 → 自动重连恢复
