//! 设置窗口的绘制。只画界面、只返回用户意图,不碰文件、不碰 supervisor ——
//! 副作用全部由 app.rs 执行(保存流程本身在 [`crate::settings::save`] 里),
//! UI 层因此保持无状态、无 IO,不需要测。
//!
//! **[`draw`] 同一帧里可能被调用多次**:调用方 `EguiWindow::redraw_ui` 收的是
//! `FnMut`,而界面里用到的 `egui::Grid` 每次开窗第一帧必然 `request_discard`、
//! 让整帧重跑一趟。所以这里只允许改 [`View`] 里的表单文本和返回 [`Outcome`],
//! 一次性的动作(保存、写文件、重启 HTTP 层)必须由调用方在闭包外做一次。

use crate::settings::state::Form;

pub use crate::settings::state::Message;

/// 用户在这一帧里表达的意图。同一帧最多认一个:后点的覆盖先点的,
/// 而实际上一帧内不可能同时点中两个按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    None,
    Save,
    Cancel,
    OpenConfigFile,
    RegenerateToken,
}

/// 设置窗口的全部可见状态。窗口开着的时候由 app.rs 持有,关窗时一起丢掉。
pub struct View {
    pub form: Form,
    /// 只读展示,v1 不提供修改。
    pub window_size: String,
    /// (名称, 命令) —— 只读列表。
    pub launchers: Vec<(String, String)>,
    /// 启动时的致命状况(配置解析失败 / 没监听上),常驻在窗口最顶上,直到状况
    /// 被修好为止。和 `message` 不是一回事:后者只描述最近一次保存的结果。
    pub banner: Option<String>,
    pub message: Option<Message>,
}

const OK_COLOR: egui::Color32 = egui::Color32::from_rgb(60, 140, 60);
const ERR_COLOR: egui::Color32 = egui::Color32::from_rgb(180, 60, 60);

pub fn draw(ui: &mut egui::Ui, view: &mut View) -> Outcome {
    let mut outcome = Outcome::None;

    egui::CentralPanel::default().show(ui, |ui| {
        // 整个窗口内容包一层竖直滚动:配置解析失败 + 监听失败叠加时,`banner`
        // 能长到 9 行左右(toml::de::Error 的 Display 自带 6 行位置指示片段),
        // 460×460 的窗口现在勉强装得下,但设置窗口正是「起不来时唯一的出路」——
        // 装不下就等于没有出路。这层 ScrollArea 是很便宜的保险,常态下(没有
        // banner)内容够短,不会触发滚动,视觉上没有变化。
        //
        // 布局选择:滚动区域包含**全部**内容,包括底部的「取消 / 保存并应用」
        // 按钮 —— 没有另外用 `TopBottomPanel::bottom` 把按钮钉在底部。banner
        // 很长时用户可能要先滚到底才能点到保存按钮,这是刻意接受的权衡:这个
        // 窗口是 460×460 的小对话框,按钮钉底需要额外拆一层 Panel、把当前
        // 「一个 CentralPanel 里从上画到下」的简单结构复杂化,换来的只是免去
        // 一次滚动动作,不值得。
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);

            if let Some(banner) = &view.banner {
                ui.colored_label(ERR_COLOR, banner.as_str());
                ui.add_space(6.0);
            }

            // 表单一被改动就把上一次的保存结果抹掉:保存成功后接着改端口,绿色的
            // 「已生效 · …」还挂在那里,说的已经不是当前表单里这套值了。
            // 纯赋值,不违反「同一帧可能跑两趟」的约束。
            let mut edited = false;

            egui::Grid::new("settings")
                .num_columns(2)
                .spacing([12.0, 10.0])
                .show(ui, |ui| {
                    ui.label("监听地址");
                    ui.horizontal(|ui| {
                        edited |= ui
                            .add(
                                egui::TextEdit::singleline(&mut view.form.host)
                                    .desired_width(140.0),
                            )
                            .changed();
                        ui.label(":");
                        edited |= ui
                            .add(
                                egui::TextEdit::singleline(&mut view.form.port).desired_width(60.0),
                            )
                            .changed();
                    });
                    ui.end_row();

                    ui.label("访问 token");
                    ui.horizontal(|ui| {
                        edited |= ui
                            .add(
                                egui::TextEdit::singleline(&mut view.form.token)
                                    .desired_width(180.0),
                            )
                            .changed();
                        if ui.button("重新生成").clicked() {
                            outcome = Outcome::RegenerateToken;
                        }
                    });
                    ui.end_row();

                    ui.label("");
                    ui.small(
                        "留空表示不固定:不写进 config.toml,每次启动随机生成。\n\
                     本次运行沿用当前 token;若服务未在监听,保存时当场生成一个新的。",
                    );
                    ui.end_row();

                    ui.label("会话尺寸策略");
                    ui.horizontal(|ui| {
                        ui.label(view.window_size.as_str());
                        ui.small("(在 config.toml 中修改)");
                    });
                    ui.end_row();

                    ui.label("开机自启");
                    edited |= ui.checkbox(&mut view.form.autostart, "").changed();
                    ui.end_row();
                });

            ui.add_space(8.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("启动项");
                if ui.button("打开 config.toml").clicked() {
                    outcome = Outcome::OpenConfigFile;
                }
            });
            egui::ScrollArea::vertical()
                .max_height(110.0)
                .show(ui, |ui| {
                    if view.launchers.is_empty() {
                        ui.small("(只有内置项)");
                    }
                    for (name, command) in &view.launchers {
                        ui.horizontal(|ui| {
                            ui.label(name.as_str());
                            ui.small(command.as_str());
                        });
                    }
                });

            // 「重新生成」也算改动:token 已经不是保存时那个了。
            if edited || outcome == Outcome::RegenerateToken {
                view.message = None;
            }

            ui.add_space(8.0);
            if let Some(msg) = &view.message {
                match msg {
                    Message::Ok(t) => ui.colored_label(OK_COLOR, t.as_str()),
                    Message::Err(t) => ui.colored_label(ERR_COLOR, t.as_str()),
                };
            }

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("取消").clicked() {
                    outcome = Outcome::Cancel;
                }
                if ui.button("保存并应用").clicked() {
                    outcome = Outcome::Save;
                }
            });
        });
    });

    outcome
}
