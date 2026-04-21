//! InkOS Desktop — Rust + egui 首版，兼容 inkoswin 的小说目录与 `~/.inkos/.env`。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent_routing;
mod aigc;
mod analytics;
mod app;
mod audit33;
mod audit_log;
mod book_rules;
mod chapter_md;
mod config;
mod export;
mod fanfic;
mod fonts;
mod history;
mod import_chapters;
mod inkoswin_prompt;
mod intent;
mod llm;
mod notify;
mod oplog;
mod pipeline;
mod project;
mod rename;
mod runtime;
mod search;
mod state_refresh;
mod state_sync;
mod style;
mod theme;
mod vendors;
mod words;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("InkOS Desktop"),
        ..Default::default()
    };

    eframe::run_native(
        "InkOS Desktop",
        native_options,
        Box::new(|cc| Ok(Box::new(app::InkOsApp::new(cc)))),
    )
}
