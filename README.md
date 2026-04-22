# InkOS Desktop（Rust + egui）

> 本项目是 [Narcooo/inkos](https://github.com/Narcooo/inkos) 的桌面端 Rust/egui 二次实现，向其作者与社区致敬：
> 上游 `Narcooo/inkos` 是基于 TypeScript / Node.js 的「自动化小说写作 AI Agent」CLI + Web Studio + TUI 套件，本仓库在桌面工作台形态下复刻其核心心智模型——多 Agent 写作管线、连续性审计、状态档案治理、文风指纹、AIGC 检测、字数治理、同人导入、多模型路由、守护通知等，并在数据布局上保持与 [`inkoswin`](https://github.com/Narcooo/inkoswin) 旧目录结构兼容。
>
> **特别感谢** Narcooo 与 InkOS 社区对开源生态的持续输出，本项目一切设计灵感、术语命名与 prompt 思路均承袭自上游。

## 与上游 Narcooo/inkos 的功能对照

下表展示当前桌面端已经覆盖、尚未对齐、或在桌面形态下做了等价替换的功能。✅ 已实现 · 🟡 部分实现 · ⏳ 计划中。

| 上游能力 | 本项目对应 | 状态 |
|---|---|---|
| 项目目录 / `inkos.json` / `.env` | 兼容 `inkoswin` 项目目录、`~/.inkos/.env`、小说级 `.env` | ✅ |
| 章节 Markdown / `chapters/NNN.md` | 章节编辑器 + CommonMark 预览 + 自动备份 | ✅ |
| 7 个真相文件（current_state / particle_ledger / pending_hooks / chapter_summaries / subplot_board / emotional_arcs / character_matrix） | 9 个状态档案（增加 `book_rules.md` 与 `novel_brief.md`） | ✅ |
| 写作管线 `plan / compose / draft / audit / revise` | 五段式管线 + per-chapter `story/runtime/` 产物（intent.md / context.json / rule-stack.yaml / trace.json） | ✅ |
| 字数治理（目标 ± 容差 + 1 次纠偏） | 章节 UI 字数治理控件 + 单次归一化 | ✅ |
| 33 维连续性 + 去 AI 味审计 | 33 维 + 去 AI 味维度的内置 prompt 模板 | ✅ |
| `style analyze / import` 文风指纹 | 文风分析（句长 / 词频 / 节奏）+ `story/style_fingerprint.json` | ✅ |
| AIGC 检测 `inkos detect` | 启发式 + LLM 评分混合检测，单章 / 全书 | ✅ |
| 数据分析 `inkos analytics` | 审计通过率、字数 / token 排名、问题词云 | ✅ |
| `import chapters` 已有文本批量导入 | 单文件按「第X章」拆分 + 断点续导 | ✅ |
| 全书导出 TXT / Markdown / EPUB | `导出/` 目录 + 手写 EPUB3 容器 | ✅ |
| 全书实体改名 `把 A 改成 B` | 全章节 + 状态档案扫描替换 + 自动快照 | ✅ |
| 多模型路由（按 Agent 维度覆盖） | `agent_routing.json` + Settings 表格 | ✅ |
| 守护进程通知（Telegram / 飞书 / 企业微信 / Webhook） | `notify.rs` + 事件过滤 + HMAC 签名 | ✅ |
| 同人创作 `fanfic init` | 同人书向导（canon / au / ooc / cp） | ✅ |
| 长期作者意图 `author_intent.md` / `current_focus.md` | `story/author_intent.md` + `story/current_focus.md` 控制层 | ✅ |
| 全文搜索 | 「搜索」页：跨章节 + 状态档案 | ✅ |
| 章节版本快照与回滚 | `章节历史/` + 版本对比弹窗 | ✅ |
| 守护进程 `inkos up / down` | 「定时写作」+ 链式审计 + 通知钩子 | ✅ |
| 连续批量写 `--count N` | 「定时写作」批量栏 | ✅ |
| OpenClaw / TUI 共享交互内核 | 暂未对齐（桌面 GUI 形态替代） | ⏳ |
| Web Studio（Vite + React + Hono） | 暂不复刻；本项目即为「原生桌面 Studio」 | ⏳ |
| SQLite 时序记忆 | 当前以 `state_sync` 落 markdown / `analytics.json` 替代 | ⏳ |

## 本版功能总览

### 项目与配置

- 选择小说目录、最近列表、启动时自动加载默认目录。
- 兼容 `inkoswin` 的 `chapters/`、`story_state/*.md`、`.inkoswin/project.json`、`~/.inkos/.env`、小说级 `.env`、`~/.inkoswin/settings.json`。
- 多服务商管理（OpenAI / Anthropic / DeepSeek / Moonshot / MiniMax / 百炼 / 智谱 / SiliconFlow / PPIO / OpenRouter / Ollama / 自定义），按服务商维度填 Base URL / Model / API Key / Temperature / Max Tokens / Thinking Budget。
- **多 Agent 路由**：对 writer / auditor / planner / composer / observer / reviser / radar 七类 Agent 可分别覆盖 vendor + model；未配置自动回落至默认 writing / review vendor。

### 写作管线（与上游 Narcooo/inkos 对齐）

- 五段式原子动作：**plan → compose → draft → audit → revise**。
- 每章产出 runtime 文件树：`story/runtime/chapter-XXXX.intent.md`、`context.json`、`rule-stack.yaml`、`trace.json`。
- **字数治理**：目标字数 ± 容差区间，超界后单次纠偏归一化（追加压缩/扩写指令），不会硬截断正文。
- **33 维审计**：内置 33 个维度 + 「去 AI 味」专项维度的审计 prompt 模板，自动识别高频词、句式单调、过度总结。
- **作者意图 / 当前焦点**：`story/author_intent.md`（长期目标）+ `story/current_focus.md`（近期 1-3 章关注点），自动注入到管线 prompt。

### 内容流转

- **导入章节**：单个文本按「第X章」/ 自定义 split 正则拆分，支持断点续导（已有章节自动跳过）。
- **导出全书**：TXT / Markdown / EPUB（手写 EPUB3 容器，含 OPF/NCX，章节按编号有序），落盘到 `导出/<书名>-<日期>.<ext>`。
- **同人书向导**：从 `source.txt` 初始化，支持 canon / au / ooc / cp 四种模式，对应不同的 `author_intent.md` 模板与同人专属审计开关。
- **全书实体改名**：`把林烬改成张三`，扫描全部章节 + 状态档案，替换前自动生成快照与状态备份。
- **全文搜索**：「搜索」页跨章节 + 状态档案。

### 文风 / AIGC / 分析

- **文风指纹**：选择参考文本 → 分析（句长分布、词频疲劳、标点节奏、段落长度）→ 写入 `story/style_fingerprint.json`，被 writer / reviser prompt 引用。
- **AIGC 检测**：启发式（高频词检测、句式重复度、过度总结、连接词密度）+ 可选 LLM 评分。单章 / 全书统计。
- **数据分析**：审计通过率、章节字数 / token / 耗时排名、最常出现的审计问题。

### 写作工作台

- 章节列表 + 状态切换（草稿 / 写作中 / 待审核 / 已完成 / 已归档 / 已生成）。
- 编辑器 + CommonMark 实时预览（约 0.28s 防抖）。
- 摘要独立编辑，保存时回写 `project.json` 与 `chapter_summaries.md`。
- 章节历史快照与版本对比 + 一键恢复。
- **伏笔提醒**：监控 `pending_hooks.md` 变更，写作页顶部弹卡片提示。

### 审核与状态档案同步

- 「审核」模式（基于状态档案 + 项目背景的全维度审计）与「改写」模式（结合最近一次审计意见做整章改写）。
- AI 改写替换原文后自动链式触发 **状态档案同步**：让审计 LLM 输出 JSON delta（replace / patch），再写回 `story_state/*.md`，破坏性写入前自动备份到 `状态档案备份/<时间戳>/`。
- 审计 / 改写记录持久化到 `审计记录/log.jsonl`，可一键载入历史结果。

### 写作助手

- 与「写作 LLM」对话，自动注入项目背景 + 全部状态档案。
- 默认坚持 `book_rules.md` 中的硬约束（personalityLock / behavioralConstraints / prohibitions / forbidden）。

### 定时写作 + 通知

- 间隔（分钟级）自动执行：`先审计上一章 → 写下一章` 链式工作流。
- 支持连续写 N 章（批量栏）。
- 守护事件 → Telegram / 飞书 / 企业微信 / 通用 Webhook 推送（HMAC-SHA256 签名 + 事件类型过滤）。

### 操作日志

- 每个项目独立的 `操作日志/YYYY-MM-DD.jsonl`，记录所有关键事件（打开项目、保存章节、AI 审计、AI 改写、替换原文、状态同步、定时写作、导入导出、改名 …）。

## 数据布局

```
<小说根目录>/
├── chapters/NNN.md                 # 章节正文（标题 + 正文）
├── story_state/                    # 9 个真相文件（与 inkoswin 兼容）
│   ├── book_rules.md               # 硬约束（YAML frontmatter）
│   ├── novel_brief.md
│   ├── current_state.md
│   ├── particle_ledger.md
│   ├── pending_hooks.md
│   ├── chapter_summaries.md        # 系统自动生成
│   ├── subplot_board.md
│   ├── emotional_arcs.md
│   └── character_matrix.md
├── story/                          # 与 Narcooo/inkos 对齐的扩展层
│   ├── author_intent.md            # 长期作者意图
│   ├── current_focus.md            # 近期 1-3 章关注点
│   ├── style_fingerprint.json      # 文风指纹
│   ├── analytics.json              # 数据分析缓存
│   ├── aigc_report.json            # AIGC 检测结果
│   └── runtime/
│       ├── chapter-0001.intent.md
│       ├── chapter-0001.context.json
│       ├── chapter-0001.rule-stack.yaml
│       └── chapter-0001.trace.json
├── 章节历史/第NNN章/<时间戳>.md     # 自动备份与版本快照
├── 审计记录/log.jsonl              # 审计 / 改写历史
├── 操作日志/YYYY-MM-DD.jsonl       # 全局操作日志
├── 状态档案备份/<时间戳>/          # 状态档案变更前快照
├── 导出/<书名>-<日期>.{txt,md,epub}
└── .inkoswin/
    ├── project.json
    └── agent_routing.json          # 多 Agent 路由表
```

全局：
- `~/.inkos/.env`：全局 LLM 默认配置（与上游 inkos / inkoswin 共享）。
- `~/.inkoswin/settings.json`：桌面端偏好（最近项目、活跃服务商、流式开关、定时写作钩子）。

## 版本与发行（GitHub Release）

应用内底部状态栏与「帮助 → 关于」页显示：`Cargo.toml` 中的版本号 + 当前构建的 Git 短提交（由 `build.rs` 在编译时写入）。

### 发版前准备

1. 在 [`CHANGELOG.md`](CHANGELOG.md) 顶部增加 `## [x.y.z] - 日期` 小节，用简短条目写清用户可见的变更。
2. 将 [`Cargo.toml`](Cargo.toml) 里 `[package] version` 改为与发版号一致的 `x.y.z`（与 Git tag 去掉前缀 `v` 后一致）。
3. 提交上述改动，例如：`git commit -am "chore: release v0.2.0"`。

### 打标签并触发自动构建

本仓库已配置 [`.github/workflows/release.yml`](.github/workflows/release.yml)：向 GitHub **推送** 匹配 `v*` 的 tag 时，会在 `windows-latest` 上执行 `cargo build --release`，并将 `inkos_desktop.exe` 打成 zip 上传到 **同一 tag** 对应的 GitHub Release。

```powershell
git tag v0.2.0
git push origin v0.2.0
```

- tag 名建议与版本一致，例如版本 `0.2.0` 对应 tag `v0.2.0`。
- 若 Release 未出现附件，请到仓库 **Settings → Actions → General**，将 **Workflow permissions** 设为可写 `contents`（或使用组织策略允许 `GITHUB_TOKEN` 写 Release）。

### 手动发版（不用 Actions 时）

本地 `cargo build --release` 后，在 GitHub 网页 **Releases → Draft a new release** 中选择 tag、上传 zip 即可。

## 运行

需要本机已安装 [Rust](https://rustup.rs/)（Windows 上通常还需 VS Build Tools 以链接 egui 默认的 `glow` 后端）。

```powershell
cd D:\laragon\www\inkos\inkos_desktop
cargo run --release
```

## 技术栈

- `eframe` / `egui` 0.34 — GUI 主框架
- `egui_commonmark` — Markdown 预览
- `serde` / `serde_json` — `project.json`、`settings.json`、runtime artifacts
- `ureq` — OpenAI 兼容 chat（含 SSE 流式）
- `rfd` — 系统文件夹选择
- `chrono`、`dirs`、`dunce`、`anyhow` — 系统辅助
- `regex`、`zip`（EPUB 容器）、`hmac` / `sha2`（Webhook 签名）— 新增子系统

## 致谢与许可

- 灵感来源 / 心智模型完全来自 [Narcooo/inkos](https://github.com/Narcooo/inkos)（AGPL-3.0），本项目以 `MIT OR Apache-2.0` 双许可形式独立实现，遵循 AGPL 上游 prompt / 设计模式的「思想与方法」可被自由参考的精神。
- 上游 [`inkoswin`](https://github.com/Narcooo/inkoswin) 桌面参考实现也对本项目的目录布局有直接影响，特此致谢。
- 如需投入生产或商用 InkOS 心智模型，**优先推荐使用上游 `Narcooo/inkos`**——这是原作者持续迭代、社区最活跃的版本。
