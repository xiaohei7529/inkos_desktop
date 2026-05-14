# Changelog

本文件记录面向用户的版本变更摘要；发版时请同步更新并 bump `Cargo.toml` 中的版本号。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)。

## [0.3.0] - 2026-05-14

主题：**Beats First 写作流 + 批量 JSON Delta 刷档 + 写作页 Dashboard**。本版本把写作前置规划、长期记忆刷新、写作页态势感知三件事整体打通，AI 写作时的「注意力分散」和档案刷新的「重而慢」两大痛点同时收敛。

### 新增

- **Beats First Workflow（本章节拍层）**：
  - `ChapterRecord` 新增 `beats: Vec<String>` 字段，并通过 `#[serde(default)]` 完全兼容旧版 `project.json`。
  - 写作页新增「✨ 生成节拍」卡片：根据小说设定 + 前文摘要 + 全书大纲，单次调用 LLM 产出 3-5 条本章短句要点；支持手动增/删/改，所有改动自动随章节落盘。
  - 写作 Prompt（`inkoswin_prompt::build_generation_prompts`）注入 `[本章节拍]` 段，并新增写作硬约束「必须依次推进上方[本章节拍]的每条节拍」，让生成更贴合用户已确认的故事推进点。
- **JSON Delta 批量刷档**：
  - `state_refresh::build_batch_delta_prompts`：与「保存并同步记忆」同构的 system prompt，要求 LLM 输出单个 JSON `{summary, updates[]}`，`action` 支持 `replace` 与 `patch`（后者使用 `===REPLACE_BLOCK===/===WITH===/===END===` 分隔符）。
  - 写作侧 `try_start_state_refresh` 由「N 次全量 Markdown 重写」改为「1 次 spawn_chat + `state_sync::apply_updates` 落盘」，原 `kick_state_refresh_step` 已删除。
  - 自动按最新章节 beats 调用 `filter_state_docs_by_beats` 过滤注入上下文，未涉及的档案不再塞进 prompt（核心档案 `outline.md / novel_brief.md / chapter_summaries.md / book_rules.md` 恒注入）。
  - `chapter_summaries.md` 在刷档开始时**立即在本地重建**，不再消耗一次 LLM 调用。
- **写作页 Dashboard 三件套**：
  - 「📡 当前激活上下文」小组件：在节拍卡片之后显示本次 AI 会读取的状态档案 pill 列表（按 beats 过滤后），hover 显示前 200 字预览；Beats 为空时回退到全量列表。可折叠。
  - 「🪝 伏笔追踪」悬浮窗：工具栏 toggle 打开，自定义大小，列出 `pending_hooks.md` 摘要并对「停滞 / 风险 / 紧迫 / 必须 / 未兑现 / 逾期 / 悬而未决」等关键词高亮加粗；支持一键刷新与跳转到档案页编辑。
  - 流式生成中的「🛑 停止并保存」按钮：立即停止 AI 生成、强制标记 dirty 并把已收到的内容落盘到磁盘，避免「写一半中断 → 内容丢失」。
- **单元测试**：
  - `inkoswin_prompt::beats_parse_*` 系列共 3 条，覆盖节拍解析的常见前缀剥离、5 条上限、忽略空行/围栏。
  - `state_refresh::filter_*` 共 3 条，覆盖空 beats 回退全量、核心档案恒注入、命中关键词时按需注入条件档案。
  - `state_refresh::batch_delta_prompt_*` 共 2 条，覆盖白名单与 `book_rules.md / chapter_summaries.md` 禁止列表、`[本章节拍]` 段注入。

### 优化

- 刷档调用次数由「`REFRESH_ALL_ORDER` 中每个文件 1 次 LLM」改为 **1 次 LLM**；状态条提示统一为「🔄 AI 刷新状态档案中（批量 JSON Delta · N 个目标文件）…」，完成后给出摘要与改动统计。
- `apply_updates` 返回的 `Vec<StateFileChange>` 会逐条写入 `state_sync_log`，便于在写作页底部最近日志直接看到每个文件的 action / 字数变化。
- OpLog 在批量刷档全链路（开始 / 本地重建 / 跳过 story 控制层 / 完成 / 失败 / 跳过 LLM）补齐多条目，方便回溯。
- 「保存并同步记忆」按钮 hover 文案更新为「1 次写作 LLM 调用（批量 JSON Delta）」，准确反映新链路。
- `egui` 借用安全：节拍编辑、激活上下文小组件、伏笔追踪悬浮窗在迭代/可变借用前统一先 `clone` 关键数据快照，避免 `&mut self` 与子借用冲突。

### 兼容性 / 迁移说明

- 旧 `project.json` 加载无需改动，缺失的 `beats` 字段会默认为空 `Vec`；首次保存章节时自动按当前编辑器状态写回。
- `state_refresh::build_state_document_prompts` 与 `sanitize_model_markdown` 暂时保留作为兼容回路，未被默认链路调用。
- 批量 JSON Delta 当前仅写 `story_state/`；`story/author_intent.md` 与 `story/current_focus.md` 暂时跳过（OpLog 会显式记录），下一阶段评估是否纳入。

## [0.2.4] - 2026-05-13

### 新增

- 新建小说目录校验：创建前拒绝已有 `.inkoswin/project.json` / `.inkos/project.json` 的目录，并拒绝非空目录，避免误覆盖已有小说档案。
- 为新建小说目录校验补充单元测试，覆盖空目录、已有项目目录、非空目录三类场景。

### 优化

- 设置页「服务商管理」升级为更清晰的控制台式布局：左侧展示已配置数量、服务商用途与选中状态；右侧按「连接信息」「模型与参数」「验证与保存」分区维护配置。
- 新建小说向导将「题材」纳入必填校验，并明确创建成功后初始化的是本地状态档案模板，不会自动调用 LLM 生成状态档案。
- 新建章节后的状态提示明确说明：章节占位与状态档案模板已就绪，长期记忆会在保存正文后再同步。
- 章节保存后的自动长期记忆刷新若因前置条件不满足而跳过，会在状态栏显示具体原因，减少“已保存但未同步”的误解。
- AI 生成章节前的档案完整性校验统一为「标题、题材、故事核心」三项必填。

### 修复

- 修复章节导入默认分割正则过宽的问题，避免正文行如「第二章正文」被误识别为新章节标题；仍支持「第一章 序幕」「第一章：序幕」「第二章-风暴」等常见标题形式。

## [0.2.3] - 2026-05-12

### 新增

- 项目页「新建小说…」向导：选择目录并填写标题、题材、故事核心等档案后创建项目，自动初始化 `story_state/`（含各状态模板与书籍大纲文件）。
- 状态档案新增 `outline.md`（宏观大纲、分卷、章节细纲模板）；「小说设定 → 状态档案」下拉与「AI 刷新全部」顺序已纳入该文件。
- 设置 → 服务商管理：在「默认模型」旁增加「获取模型」，后台请求 OpenAI 兼容 `GET {Base URL}/models`；若已填写 API Key，则携带 `Authorization: Bearer <API Key>`。返回的模型列表支持关键字筛选，点击列表项可填入「默认模型」。

### 优化

- 章节生成时「连续性档案」拼接顺序调整：`outline.md` 优先（更长字符预算，便于对齐细纲）、并纳入 `book_rules.md` 作为硬约束。
- `chapter_summaries.md` 由系统重建时：超过 100 章的项目将更早章节折叠为「早期章节归档」区段，仅保留最近 100 章的完整摘要块，减轻单文件体积与读取压力。
- 「AI 生成本章 / 生成下一章」：若小说标题或故事核心为空，则提示并跳转到「小说设定」，引导先完善档案再生成。
- 设置页「服务商管理」改为左侧服务商列表 + 右侧内嵌配置面板（基础参数、连接测试、保存与移除），替代原网格卡片 + 居中弹窗编辑。
- 审核页：「审计」「改写」在接口超时或其它原因导致请求失败（结果区含「（请求失败）」或「（请求中断）」）且未更换目标章节时，工具栏显示「重新同步」，可一键重新发起当前模式的 LLM 请求；操作日志分别记录 `AI 审计 · 重新同步` / `AI 改写 · 重新同步`。

### 优化

- 强化章节生成字数治理：生成提示增加目标字数区间约束；保存/生成后可自动注入「归一化 prompt」用于单次纠偏。
- 默认每章目标字数从 2500 调整为 3000，并同步到项目初始化、旧项目兜底与字数预算默认值。
- 对齐「定时写作」与「写作-生成本章」的摘要链路，统一使用同一解析逻辑写入章节摘要。
- 新增「统一章节摘要（60-80 字）」开关（默认开启），减少不同生成链路的摘要风格漂移。

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
