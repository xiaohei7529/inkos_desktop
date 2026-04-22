//! 应用版本与构建元信息（`CARGO_PKG_VERSION` + `build.rs` 注入的 Git 短 SHA）。

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_SHA: &str = env!("INKOS_GIT_SHA");

/// 公开仓库地址（发行版与源码）。
pub const REPOSITORY_URL: &str = "https://github.com/xiaohei7529/inkos_desktop";

/// 底部状态栏与「关于」页展示的简短标签。
pub fn version_label() -> String {
    format!("InkOS Desktop · v{APP_VERSION} · {GIT_SHA}")
}
