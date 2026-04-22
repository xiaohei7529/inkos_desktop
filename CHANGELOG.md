# Changelog

本文件记录面向用户的版本变更摘要；发版时请同步更新并 bump `Cargo.toml` 中的版本号。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)。

## [0.2.1] - 2026-04-22

### 新增

- 长期记忆刷新扩展至 `story/author_intent.md` 与 `story/current_focus.md`，并纳入刷新上下文。
- 定时写作在章节自动保存后，会触发与手动保存一致的状态档案自动刷新链路。
- 第 1 章完成后可自动追加同步刷新 `book_rules.md`，避免硬约束与正文进度脱节。

### 优化

- 「AI 刷新状态档案」的提示文案与行为对齐，支持 `story/` 控制层文件刷新。
- 刷新写盘路径支持按文件类型自动分流到 `story_state/` 与 `story/`。

## [0.2.0] - 2026-04-22

### 新增

- 底部状态栏显示 `版本号 + Git 短提交`；侧栏「帮助 → 关于」页展示本更新日志与 GitHub 链接。
- GitHub Actions：推送 `v*` 标签时在 Windows 上构建 `release` 并上传 `inkos_desktop-windows-x64-<tag>.zip` 到 Release。

### 文档

- README 增加「版本与发行」：CHANGELOG、`Cargo.toml` 对齐、打 tag、推送与 Actions 权限说明。

## [0.1.0] - 2026-04-21

### 新增

- 首版 InkOS Desktop：Rust + egui 写作工作台，兼容 inkoswin 目录与配置。
- 左侧导航：项目、写作、审核、助手、小说设定、操作日志、工具箱、设置。
- 多服务商与 Agent 路由、流式 LLM、状态档案与 book_rules 等能力。

### 说明

- 应用内「关于」页展示本日志；底部状态栏显示 `版本号 + Git 短提交`，便于区分每次构建。
