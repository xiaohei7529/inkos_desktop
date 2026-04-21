# InkOS Desktop（Rust + egui）

与 `inkoswin` 使用相同的小说目录布局、`.inkoswin/project.json`、`chapters/*.md`、`story_state/*.md` 以及 `~/.inkos/.env`、小说目录 `.env`、`~/.inkoswin/settings.json`。

## 本版已实现

1. **项目**：选择小说目录、最近列表、启动时加载默认目录。
2. **章节**：新建下一章、选择章节、编辑标题/状态/正文、保存到 `NNN.md`；右侧 **CommonMark 预览**（`egui_commonmark`，约 0.28s 防抖）。
3. **配置**：编辑并保存全局 `~/.inkos/.env` 与当前小说 `.env`；展示合并后的生效字段（API Key 脱敏）。
4. **小说设定**：编辑 `project.json` 中的元数据并保存；保存时会刷新 `story_state/chapter_summaries.md` 并保证其它状态文件存在（与 `inkoswin` 逻辑对齐）。

## 未实现（后续迭代）

- LLM 调用（生成章节、书名设定等）
- 状态档案单独编辑 Tab、写作助手对话
- 自动定时生成、打开外部编辑器等

## 运行

需要本机已安装 [Rust](https://rustup.rs/)（Windows 上通常还需 VS Build Tools 以链接 egui 默认的 `glow` 后端）。

```powershell
cd D:\laragon\www\inkos\inkos_desktop
cargo run --release
```

## 技术栈

- `eframe` / `egui` 0.34
- `egui_commonmark`：Markdown 预览
- `serde` / `serde_json`：`project.json`、`settings.json`
- `rfd`：系统文件夹选择
- `chrono`、`dirs`、`dunce`、`anyhow`
