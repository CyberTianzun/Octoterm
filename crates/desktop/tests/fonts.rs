//! 中文字体确实被挂上去了。这一条纯 CPU:egui::Context 不需要窗口也不需要 GPU,
//! 所以「界面上的中文会不会变成豆腐块」这件事不用靠肉眼看。

use octoterm_desktop::fonts;

/// 设置界面里出现过的字,挑几个常用的。
const SAMPLE: &str = "监听地址访问令牌开机自启保存并应用取消已生效个会话未受影响";

/// 跑一帧,把字体真正建出来(`ctx.fonts_mut` 在第一帧之前用不了)。
///
/// `FullOutput::textures_delta` 必须显式 clear:它的 Drop 里有 debug_assert,
/// 平时由渲染器消费掉,这里没有渲染器。
fn warm_up(ctx: &egui::Context) {
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
}

#[test]
fn the_bundled_default_fonts_have_no_chinese_at_all() {
    // 这条是上面那条的前提:如果哪天 egui 自带了中文,fonts.rs 就没必要存在了。
    let ctx = egui::Context::default();
    warm_up(&ctx);
    let has = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), "设置"));
    assert!(!has, "egui 默认字体居然有汉字了,fonts.rs 可以删了");
}

#[test]
#[cfg(any(target_os = "macos", windows))]
fn a_system_chinese_font_is_found_on_the_target_platforms() {
    let found = fonts::load();
    assert!(found.is_some(), "目标平台上必须能找到一份中文字体");
}

#[test]
#[cfg(any(target_os = "macos", windows))]
fn installing_makes_the_settings_labels_renderable() {
    let ctx = egui::Context::default();
    fonts::install(&ctx);
    warm_up(&ctx);

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let id = egui::FontId::new(14.0, family.clone());
        let has = ctx.fonts_mut(|f| f.has_glyphs(&id, SAMPLE));
        assert!(has, "{family:?} 里缺字形:界面会显示成方块");
    }
}

#[test]
#[cfg(any(target_os = "macos", windows))]
fn latin_text_still_comes_from_the_default_font() {
    // 中文字体是最低优先级的 fallback,不该把 ASCII 也接管过去 —— 真接管了这条
    // 测不出来(两边都有字形),但至少保证装了它之后英文数字没丢。
    let ctx = egui::Context::default();
    fonts::install(&ctx);
    warm_up(&ctx);
    let has = ctx.fonts_mut(|f| {
        f.has_glyphs(&egui::FontId::proportional(14.0), "127.0.0.1:7683 config.toml")
    });
    assert!(has);
}
