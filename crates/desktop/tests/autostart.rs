#[cfg(target_os = "macos")]
#[test]
fn the_plist_names_the_executable_and_runs_at_load() {
    let xml = octoterm_desktop::autostart::plist_xml("/Applications/octoterm.app/Contents/MacOS/octoterm-desktop");

    assert!(xml.contains("<key>Label</key>"));
    assert!(xml.contains("com.octoterm.desktop"));
    assert!(xml.contains("/Applications/octoterm.app/Contents/MacOS/octoterm-desktop"));
    assert!(xml.contains("<key>RunAtLoad</key>"));
    assert!(xml.contains("<true/>"));
    // LaunchAgent 只该在登录时拉起一次,不该被当成需要常活的守护
    assert!(!xml.contains("KeepAlive"));
}

#[cfg(target_os = "macos")]
#[test]
fn a_path_with_an_ampersand_is_escaped() {
    let xml = octoterm_desktop::autostart::plist_xml("/Users/a&b/octoterm-desktop");
    assert!(xml.contains("/Users/a&amp;b/octoterm-desktop"), "路径没做 XML 转义:\n{xml}");
    assert!(!xml.contains("a&b"));
}

// 不写「启用→读回→禁用」的往返测试:它会真的改动当前用户的登录项
// (~/Library/LaunchAgents 或 HKCU Run),测试中途 panic 就会把 octoterm 留在
// 开机启动里。`is_enabled` / `set` 的真实读写由 Task 8 的手动验收覆盖。
