//! 对齐 `inkoswin.story_generator.build_generation_prompts` 与 `parse_generation_output` 的
//! 章节生成 prompt 构造 / 输出解析。
//!
//! 通过 port 原 Python 版本保证「生成本章 / 生成下一章」的结果与 inkoswin 一致：
//!
//! - 历史章节摘要：最近 12 章 `chapter.summary | make_summary(content, 100)`
//! - 最近章节正文节选：最近 3 章 × 1200 字
//! - 连续性档案：固定顺序 + 分级限长（novel_brief 2200 / chapter_summaries 1200 / 其它 800）
//! - 当前章节草稿：trim 1600
//! - 输出格式：`标题：/摘要：/正文：`

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::chapter_md::{make_summary, DEFAULT_CHAPTER_SUMMARY_LIMIT};
use crate::project::{ChapterRecord, NovelProject};

/// 与 Python `ChapterGenerationResult` 对齐。
pub struct ChapterGenerationResult {
    pub title: String,
    pub content: String,
    pub summary: String,
    pub raw_text: String,
}

/// 连续性档案有序 + 限长策略（对齐 `_build_state_context_text`）。
/// outline.md 优先注入，防止续写跑偏；book_rules.md 作为硬约束兜底。
const ORDERED_STATE_FILES: &[&str] = &[
    "outline.md",
    "novel_brief.md",
    "current_state.md",
    "pending_hooks.md",
    "subplot_board.md",
    "emotional_arcs.md",
    "character_matrix.md",
    "particle_ledger.md",
    "chapter_summaries.md",
    "book_rules.md",
];

fn state_title(filename: &str) -> &'static str {
    match filename {
        "outline.md" => "书籍大纲与细纲",
        "novel_brief.md" => "小说简报",
        "current_state.md" => "当前状态",
        "pending_hooks.md" => "待回收伏笔",
        "subplot_board.md" => "副线与情节板",
        "emotional_arcs.md" => "情感与关系弧",
        "character_matrix.md" => "角色矩阵",
        "particle_ledger.md" => "粒子账本",
        "chapter_summaries.md" => "章节摘要档",
        "book_rules.md" => "硬约束",
        _ => "状态档案",
    }
}

fn limit_for_state(filename: &str) -> usize {
    match filename {
        "outline.md" => 3000,
        "chapter_summaries.md" => 1200,
        "novel_brief.md" => 2200,
        _ => 800,
    }
}

fn trim_text(text: &str, limit: usize) -> String {
    let stripped = text.trim();
    if stripped.chars().count() <= limit {
        return stripped.to_string();
    }
    let cut: String = stripped.chars().take(limit).collect();
    format!("{}...", cut.trim_end())
}

fn build_state_context_text(state_documents: &HashMap<String, String>) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for filename in ORDERED_STATE_FILES {
        let Some(raw) = state_documents.get(*filename) else {
            continue;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let title = state_title(filename);
        let limit = limit_for_state(filename);
        blocks.push(format!(
            "[{filename} | {title}]\n{}",
            trim_text(trimmed, limit)
        ));
    }
    if blocks.is_empty() {
        "暂无状态档案。".to_string()
    } else {
        blocks.join("\n\n")
    }
}

/// 通用叙事工程基础层（题材无关）：实体追踪、信息增量、空间衔接。
pub const NARRATIVE_ENGINE_CORE: &str = r#"
### 叙事工程基础（全题材通用）

1. **信息增量 (Information Increment)**：
   - 每一章必须至少揭露一个新信息、完成一个可验证的小目标，或明显推进一条既有悬念；禁止整章原地打转的重复描写。

2. **实体追踪 (Entity Tracking)**：
   - 对已出现的关键道具/线索须标明物理状态（在手中、衣袋、车内、已丢弃等）；引入新道具时说明如何进入场景。

3. **零跳跃衔接 (Seamless Transition)**：
   - 禁止“推开门后已在街上”类空间跳跃；地点/时间变化须写出过渡动作与感官变化。

4. **Show, Don't Tell**：
   - 用具体动作、对话与感官细节承载情绪与世界观，避免作者口吻的解释性旁白。
"#;

/// 兼容旧名：已由 [`NARRATIVE_ENGINE_CORE`] + 项目级文风包替代题材硬编码。
pub const COMMON_WRITING_GUIDE: &str = NARRATIVE_ENGINE_CORE;

/// 与 [`NARRATIVE_ENGINE_CORE`] 同义，保留旧名兼容。
pub const ATMOSPHERE_STYLING: &str = NARRATIVE_ENGINE_CORE;

/// 从 `NovelProject` 组装 `[核心叙事强制约束]` 块（POV / 密度 / 文风包）。
pub fn core_narrative_constraints_section(project: &NovelProject) -> String {
    format!(
        "[核心叙事强制约束]（最高优先级，须严格遵守）\n\
         {}\n\
         {}\n\
         {}\n\
         【信息增量】本章须推进至少一个具体目标或揭露一个新信息，禁止无意义的空转铺陈。\n\
         【实体追踪】关键道具/线索须写明物理位置与状态；与 continuity 档案冲突时以档案为准。\n\n",
        project.pov.to_prompt_instruction(),
        project.density.to_prompt_instruction(),
        project.style_preset.to_prompt_instruction(),
    )
}

/// 第一章专属：悬念与锚点（与 [`CHAPTER_ONE_STRATEGY`] 互补，保留 JSON 输出提醒）。
pub const PROLOGUE_SENSE_GUIDE: &str = r#"
### 第一章专属创作指令：悬念与锚点

1. **叙事压制 (Information Suppression)**：
   - 严禁在第一章解释世界观底层逻辑、超能力来源或反派最终目的。
   - 遵循“只给谜面，不给谜底”原则。如果主角发现了异样，第一章只描写异样的恐怖/怪异，不许描写“为什么会这样”。

2. **悬念捕捉 (Hook Extraction)**：
   - 在本章结束时，你必须识别出本章埋下的、尚未解决的 3-5 个悬念（Hooks）。
   - 每条悬念须能对应：[悬念名称]、[线索片段]、[潜在指向]。

3. **感官描写入门**：
   - 强制使用“局部特写”代替“全景叙述”。不要写“我来到了殡仪馆”，要写“脚下老旧的木地板发出的嘎吱声，在浓雾中传得很远，仿佛有什么东西在暗处数着我的步子”。

4. **输出格式限制**：
   - 正文（标题/摘要/正文）结束后，可追加闭合的 ```json 代码围栏同步 pending_hooks（详见输出格式第 5 条）；程序亦会在写完后自动审计提取悬念。
"#;

/// 第一章特化：反大纲化与悬念构建（通用题材，不限诡异/都市/言情）。
pub const CHAPTER_ONE_STRATEGY: &str = r#"
### 第一章特化创作指南：反大纲化与悬念构建

1. **信息遮蔽原则 (Information Shadowing)**：
   - 严禁在第一章解释世界观的全貌。
   - 所有的设定必须通过“异常现象”引出，而不是通过“内心独白”解释。
   - 示例：不要写“这是一个邪神入侵的世界”，要写“街角的雕像在林夜转头时，似乎裂开了一道缝，溢出了腐臭的粘液”。

2. **微观张力 (Micro-Tension)**：
   - 每 300 字必须出现一个“不确定性”。不要让主角顺利完成动作。
   - 即使是喝咖啡，也要描写“咖啡表面的倒影扭曲成了一张陌生的脸”，通过这种不间断的微小干扰防止剧情平铺直叙。

3. **留白与钩子 (The Hook Extraction)**：
   - 在本章结尾，请埋下可被后续回收的【悬念钩子】。
   - 每个钩子必须包含：一个未被解释的动作、一个语焉不详的道具、或者一个身份不明的角色。

4. **拒绝逻辑闭环**：
   - 第一章的任务不是“解决问题”，而是“展示危机”。
   - 结尾必须停留在一个“不得不进行下一步”的紧迫点上，且该点必须产生至少两个逻辑方向的未知。
"#;

/// 第 2 章起：强制推进已有伏笔，禁止只挖坑不填坑。
pub const CONTINUITY_HOOKS_GUIDE: &str = r#"
### 连贯性要求：待回收伏笔（必读）

- 你必须将上方 [待回收伏笔（必须推进）] 与 [连续性档案] 中的 pending_hooks.md 视为**必读背景**。
- 每一章必须从中挑选 1-2 个现有钩子进行「推进」或「加深谜团」，严禁只挖坑不填坑、严禁抛弃已有悬念开启无关新线。
- 请根据前章末尾的物理状态与 pending_hooks 中的线索进行衔接，禁止空间/时间跳跃。
"#;

/// 悬念审计 JSON 单条（通用题材：玄幻戒指老头 / 都市神秘短信 / 言情消失初恋等）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditHookEntry {
    pub source: String,
    #[serde(rename = "type")]
    pub hook_type: String,
    pub status: String,
    pub urgency: i32,
}

/// 悬念审计 LLM 输出根结构。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditHooksReport {
    #[serde(default)]
    pub hooks: Vec<AuditHookEntry>,
}

/// 后置摘要任务 LLM 输出：`summary` + `hooks` + `state_updates`。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChapterContextReport {
    pub summary: String,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub state_updates: ChapterStateUpdates,
}

/// 人物/世界状态增量（写入 current_state.md）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChapterStateUpdates {
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub inventory: String,
}

/// 第 2 章起：续写硬约束（时空衔接 + 伏笔推进）。
pub const CONTINUITY_SEAM_HARD_RULE: &str = "         1.2 禁止跳跃时空：必须从前章末尾衔接点（500 字快照）的物理状态、地点与动作直接延续叙述；本章须从 [待回收伏笔] 中挑选 1-2 条悬念进行推进或加深，不得抛弃已有钩子开启无关新线。\n";

/// 构造「章节正文 → 结构化摘要/伏笔/状态」的 (system, user) prompt（写作完成后的后置子任务）。
pub fn build_extract_chapter_context_prompts(
    chapter_no: i32,
    chapter_title: &str,
    chapter_body: &str,
    genre: &str,
) -> (String, String) {
    let body = trim_text(chapter_body.trim(), 12_000);
    let genre_label = if genre.trim().is_empty() {
        "通用长篇小说"
    } else {
        genre.trim()
    };
    let title = if chapter_title.trim().is_empty() {
        format!("第{chapter_no}章")
    } else {
        chapter_title.trim().to_string()
    };
    let system_prompt = "你是一名长篇小说 continuity 编辑。请阅读章节正文，提取结构化剧情摘要、人物状态与未解悬念。\
不要改写正文，不要输出 Markdown 说明，只输出一个 JSON 对象（可用 ```json 围栏包裹）。".to_string();
    let user_prompt = format!(
        "请阅读以下「{genre_label}」题材第 {chapter_no} 章《{title}》正文，输出结构化上下文。\n\n\
         ## 正文\n{body}\n\n\
         ## 输出要求\n\
         严格输出如下 JSON Schema（字段名不可改）：\n\
         {{\n\
           \"summary\": \"结构化剧情总结：本章发生了什么（剧情进度，200字以内）；须含主角位置、心理状态、受伤/损耗情况\",\n\
           \"hooks\": [\"悬念1\", \"悬念2\"],\n\
           \"state_updates\": {{ \"location\": \"当前地点\", \"inventory\": \"获得或持有的关键道具/资源\" }}\n\
         }}\n\n\
         - summary：面向后续章节的真实摘要，禁止只写首句或空话。\n\
         - hooks：文中留下、尚未解答的谜团/伏笔，每条一句，建议 2-5 条；无则 []。\n\
         - state_updates：主角（或叙事焦点角色）章末所在地点与重要持物；未知则空字符串。"
    );
    (system_prompt, user_prompt)
}

/// 解析后置摘要 LLM 回包。
pub fn parse_chapter_context_output(raw: &str) -> anyhow::Result<ChapterContextReport> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("后置摘要返回为空");
    }
    if let Ok(r) = serde_json::from_str::<ChapterContextReport>(trimmed) {
        return Ok(r);
    }
    if let Some(body) = crate::state_sync::trailing_json_fence_body(trimmed) {
        if let Ok(r) = serde_json::from_str::<ChapterContextReport>(body) {
            return Ok(r);
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if end >= start {
            let slice = &trimmed[start..=end];
            if let Ok(r) = serde_json::from_str::<ChapterContextReport>(slice) {
                return Ok(r);
            }
        }
    }
    anyhow::bail!("无法解析为 ChapterContextReport JSON")
}

/// 将 `hooks` 字符串列表格式化为 `pending_hooks.md` 增量条目。
pub fn hooks_strings_to_pending_hooks_markdown(hooks: &[String]) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for (i, h) in hooks.iter().enumerate() {
        let source = h.trim();
        if source.is_empty() {
            continue;
        }
        let name = format!("悬念{}", i + 1);
        blocks.push(format!(
            "### [{name}]\n- [线索片段]：{source}\n- [潜在指向]：状态 active；由后置摘要任务提取，供后续章节推进\n",
        ));
    }
    blocks.join("\n")
}

/// 从 `chapter_summaries.md` 正文提取最近 `limit` 个 `## 第N章` 块。
pub fn extract_recent_chapter_summary_blocks(md: &str, limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let trimmed = md.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut blocks: Vec<(i32, String)> = Vec::new();
    let mut current_num: Option<i32> = None;
    let mut current_lines: Vec<String> = Vec::new();

    fn flush_block(blocks: &mut Vec<(i32, String)>, num: Option<i32>, lines: &mut Vec<String>) {
        if let Some(n) = num {
            let body = lines.join("\n").trim().to_string();
            if !body.is_empty() {
                blocks.push((n, body));
            }
        }
        lines.clear();
    }

    for line in trimmed.lines() {
        let t = line.trim();
        if t.starts_with("## 第") {
            let rest = t.strip_prefix("## 第").unwrap_or(t);
            let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num_str.parse::<i32>() {
                flush_block(&mut blocks, current_num, &mut current_lines);
                current_num = Some(n);
                current_lines.push(line.to_string());
                continue;
            }
        }
        if current_num.is_some() {
            current_lines.push(line.to_string());
        }
    }
    flush_block(&mut blocks, current_num, &mut current_lines);

    blocks.sort_by_key(|(n, _)| *n);
    blocks
        .into_iter()
        .rev()
        .take(limit)
        .map(|(n, body)| format!("## 第{n}章\n{body}"))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// 从摘要档或前文材料构造 `[最近两章摘要档]` 段（第 2 章起使用）。
pub fn recent_chapter_summaries_section(
    chapter_num: i32,
    state_documents: &HashMap<String, String>,
    previous_materials: &[(ChapterRecord, String)],
) -> String {
    if chapter_num < 2 {
        return String::new();
    }
    let from_doc = state_documents
        .get("chapter_summaries.md")
        .map(|s| extract_recent_chapter_summary_blocks(s, 2))
        .unwrap_or_default();

    let excerpt = if !from_doc.is_empty() {
        from_doc.join("\n\n")
    } else {
        let tail: Vec<&(ChapterRecord, String)> = previous_materials
            .iter()
            .rev()
            .take(2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let mut lines: Vec<String> = Vec::new();
        for (chapter, content) in tail {
            let summary = {
                let s = chapter.summary.trim();
                if s.is_empty() {
                    make_summary(content, DEFAULT_CHAPTER_SUMMARY_LIMIT)
                } else {
                    s.to_string()
                }
            };
            let title = if chapter.title.trim().is_empty() {
                format!("第{}章", chapter.number)
            } else {
                chapter.title.trim().to_string()
            };
            lines.push(format!("## 第{}章 {}\n- 摘要：{summary}", chapter.number, title));
        }
        if lines.is_empty() {
            "（暂无前两章摘要，请严格依据上一章末尾衔接点续写）".to_string()
        } else {
            lines.join("\n\n")
        }
    };

    format!("[最近两章摘要档]（必读，来自 chapter_summaries.md 或章节元数据）\n{excerpt}\n\n")
}

/// 构造「第一章正文 → 悬念钩子提取」的 (system, user) prompt（独立子任务，非阻塞主写作流）。
pub fn build_audit_hooks_prompts(
    chapter_title: &str,
    chapter_body: &str,
    genre: &str,
) -> (String, String) {
    let body = trim_text(chapter_body.trim(), 12_000);
    let genre_label = if genre.trim().is_empty() {
        "通用长篇小说"
    } else {
        genre.trim()
    };
    let system_prompt = "你是一名长篇小说 continuity 编辑，专责从章节正文中提取尚未解答的谜团、伏笔与潜在冲突。\
不要改写正文，不要输出 Markdown 说明，只输出一个 JSON 对象（可用 ```json 围栏包裹）。".to_string();
    let user_prompt = format!(
        "请阅读以下「{genre_label}」题材、第 1 章正文，提取 3-5 条【悬念钩子】。\n\n\
         章节标题：{chapter_title}\n\n\
         ## 正文\n{body}\n\n\
         ## 输出要求\n\
         严格输出如下 JSON Schema（字段名不可改）：\n\
         {{\n\
           \"hooks\": [\n\
             {{\n\
               \"source\": \"原文中的具体细节/句子摘要\",\n\
               \"type\": \"人物身份 | 世界秘密 | 关键道具 | 情感矛盾（择一或组合）\",\n\
               \"status\": \"active\",\n\
               \"urgency\": 1-5 的整数\n\
             }}\n\
           ]\n\
         }}\n\n\
         - 无论玄幻、都市、言情或悬疑，凡未解释的现象、道具、人物身份矛盾都必须收录。\n\
         - urgency：5 为最紧迫、1 为可延后。\n\
         - hooks 数组至少 1 条，建议 3-5 条。"
    );
    (system_prompt, user_prompt)
}

/// 解析悬念审计 LLM 回包。
pub fn parse_audit_hooks_output(raw: &str) -> anyhow::Result<AuditHooksReport> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("悬念审计返回为空");
    }
    if let Ok(r) = serde_json::from_str::<AuditHooksReport>(trimmed) {
        return Ok(r);
    }
    if let Some(body) = crate::state_sync::trailing_json_fence_body(trimmed) {
        if let Ok(r) = serde_json::from_str::<AuditHooksReport>(body) {
            return Ok(r);
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if end >= start {
            let slice = &trimmed[start..=end];
            if let Ok(r) = serde_json::from_str::<AuditHooksReport>(slice) {
                return Ok(r);
            }
        }
    }
    anyhow::bail!("无法解析为 AuditHooksReport JSON")
}

/// 将审计结果格式化为 `pending_hooks.md` 增量条目（供 patch 追加）。
pub fn hooks_report_to_pending_hooks_markdown(report: &AuditHooksReport) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for (i, h) in report.hooks.iter().enumerate() {
        if h.source.trim().is_empty() {
            continue;
        }
        let name = {
            let t = h.hook_type.trim();
            if t.is_empty() {
                format!("悬念{}", i + 1)
            } else {
                t.to_string()
            }
        };
        let urgency = h.urgency.clamp(1, 5);
        let status = if h.status.trim().is_empty() {
            "active".to_string()
        } else {
            h.status.trim().to_string()
        };
        let pointing = format!(
            "类型：{}；状态：{}；紧迫度 {urgency}/5（供作者/后续章节参考，勿在正文中提前揭晓答案）",
            h.hook_type.trim(),
            status
        );
        blocks.push(format!(
            "### [{name}]\n- [线索片段]：{}\n- [潜在指向]：{pointing}\n",
            h.source.trim()
        ));
    }
    if blocks.is_empty() {
        return String::new();
    }
    blocks.join("\n")
}

const PENDING_HOOKS_FOCUS_LIMIT: usize = 2000;

/// 按章节号动态组合通用指导 + 开篇/连贯性层。
pub fn get_chapter_dynamic_prompt(chapter_num: i32) -> String {
    let common = NARRATIVE_ENGINE_CORE.trim();
    if chapter_num <= 1 {
        format!(
            "{common}\n\n{}\n\n{}",
            CHAPTER_ONE_STRATEGY.trim(),
            PROLOGUE_SENSE_GUIDE.trim()
        )
    } else {
        format!("{common}\n\n{}", CONTINUITY_HOOKS_GUIDE.trim())
    }
}

/// inkoswin 章节生成 user prompt 段（方括号标题）。
pub fn chapter_dynamic_bracket_section(chapter_num: i32) -> String {
    format!(
        "[写作指导]\n{}\n\n",
        get_chapter_dynamic_prompt(chapter_num)
    )
}

/// Markdown 段（定时写作 / 写作管线 Draft 等同构注入）。
pub fn chapter_dynamic_markdown_section(chapter_num: i32) -> String {
    format!(
        "## 写作指导\n{}\n\n",
        get_chapter_dynamic_prompt(chapter_num)
    )
}

/// 兼容旧调用：默认按第 2 章逻辑（含连贯性伏笔要求）。
pub fn atmosphere_styling_bracket_section() -> String {
    chapter_dynamic_bracket_section(2)
}

/// 兼容旧调用：默认按第 2 章逻辑。
pub fn atmosphere_styling_markdown_section() -> String {
    chapter_dynamic_markdown_section(2)
}

/// 第 2 章起：单独放大 pending_hooks.md，避免在连续性档案中被截断。
pub fn pending_hooks_focus_section(
    chapter_num: i32,
    state_documents: &HashMap<String, String>,
) -> String {
    if chapter_num < 2 {
        return String::new();
    }
    let excerpt = state_documents
        .get("pending_hooks.md")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| trim_text(s, PENDING_HOOKS_FOCUS_LIMIT))
        .unwrap_or_else(|| "（暂无，但仍须承接前文已埋伏笔）".to_string());
    format!("[待回收伏笔（必须推进）]\n{excerpt}\n\n")
}

/// 第一章专属：正文后的 pending_hooks JSON 输出说明。
pub fn chapter1_hooks_json_output_section() -> &'static str {
    r#"         5. 第一章额外要求：正文结束后必须追加闭合的 ```json 代码围栏（仅此一处机器块），用于更新 pending_hooks.md。Schema 必须为单个 JSON 对象：
         {
           "summary": "本章悬念提取一句话摘要",
           "updates": [
             {
               "file": "pending_hooks.md",
               "action": "patch",
               "content": "（Markdown：每条悬念含 [悬念描述]、[关联线索]、[预期威胁]；可用 ===REPLACE_BLOCK=== 将新条目追加到「核心伏笔」区）"
             }
           ]
         }
         - updates 中仅允许 file 为 pending_hooks.md；action 优先 patch。
         - JSON 围栏之前必须是完整的 标题/摘要/正文，不要在正文中夹杂 JSON。
"#
}

/// Prompt 模板变量名：`{{LAST_CHAPTER_END}}`。
pub const PROMPT_VAR_LAST_CHAPTER_END: &str = "LAST_CHAPTER_END";

/// 将 `{{LAST_CHAPTER_END}}` 替换为上一章末尾衔接文本。
pub fn substitute_prompt_vars(text: &str, last_chapter_end: &str) -> String {
    text.replace(
        &format!("{{{{{PROMPT_VAR_LAST_CHAPTER_END}}}}}"),
        last_chapter_end,
    )
}

/// 上一章末尾衔接点正文块与硬约束行（`last_chapter_end` 为空时两者皆空）。
pub fn chapter_seam_sections(last_chapter_end: &str) -> (String, String) {
    let tail = last_chapter_end.trim();
    if tail.is_empty() {
        return (String::new(), String::new());
    }
    let block = format!("[上一章末尾衔接点]\n{tail}\n\n");
    let rule_template = "         1.1 本章开头必须在地理位置、人物动作上与以下片段实现零秒缝合：{{LAST_CHAPTER_END}}\n";
    let rule = substitute_prompt_vars(rule_template, tail);
    (block, rule)
}

/// 写作管线 / 定时写作用的 Markdown 衔接块与硬约束（与 [`chapter_seam_sections`] 文案一致）。
pub fn chapter_seam_markdown_sections(last_chapter_end: &str) -> (String, String) {
    let tail = last_chapter_end.trim();
    if tail.is_empty() {
        return (String::new(), String::new());
    }
    let block = format!("## 上一章末尾衔接点\n{tail}\n\n");
    let rule = substitute_prompt_vars(
        "本章开头必须在地理位置、人物动作上与以下片段实现零秒缝合：{{LAST_CHAPTER_END}}\n\n",
        tail,
    );
    (block, rule)
}

/// 构造生成章节的 (system, user) prompt。严格对齐 inkoswin.
///
/// `beats`：本章节拍（Beats First Workflow）。若非空，会作为最高优先级的 `[本章节拍]` 段
/// 注入到 user_prompt 中，并在「请满足以下要求」处追加"必须逐条命中节拍"的硬约束。
pub fn build_generation_prompts(
    project: &NovelProject,
    target_chapter: &ChapterRecord,
    previous_materials: &[(ChapterRecord, String)],
    beats: &[String],
    current_draft: &str,
    state_documents: &HashMap<String, String>,
    last_chapter_end: &str,
) -> (String, String) {
    let chapter_num = target_chapter.number;
    let narrative_constraints = core_narrative_constraints_section(project);
    let system_prompt = if chapter_num <= 1 {
        format!(
            "你是一名资深中文长篇小说作者。请根据已有设定创作开篇第一章：以悬念与氛围为主，不要解释创作过程。\n\
             {narrative_constraints}\
             输出须含标题/摘要/正文三段；正文结束后可追加一个闭合的 ```json 代码围栏用于 pending_hooks.md，除此之外不要输出解释。"
        )
    } else {
        format!(
            "你是一名资深中文长篇小说作者。请根据已有设定续写章节，保持人物性格、世界观、伏笔和叙事节奏一致。\n\
             {narrative_constraints}\
             不要解释创作过程，不要输出额外说明，只输出小说章节内容。"
        )
    };

    // 最近 12 章摘要
    let summary_tail: Vec<&(ChapterRecord, String)> = previous_materials
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut summary_lines: Vec<String> = Vec::new();
    for (chapter, content) in &summary_tail {
        let summary = {
            let s = chapter.summary.trim();
            if s.is_empty() {
                make_summary(content, 100)
            } else {
                s.to_string()
            }
        };
        let title = if chapter.title.trim().is_empty() {
            format!("第{}章", chapter.number)
        } else {
            chapter.title.trim().to_string()
        };
        summary_lines.push(format!(
            "第{}章《{}》：{}",
            chapter.number, title, summary
        ));
    }
    let history_summary = if summary_lines.is_empty() {
        "暂无已完成章节。".to_string()
    } else {
        summary_lines.join("\n")
    };

    // 最近 3 章节选
    let excerpt_tail: Vec<&(ChapterRecord, String)> = previous_materials
        .iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut excerpt_lines: Vec<String> = Vec::new();
    for (chapter, content) in &excerpt_tail {
        let title = if chapter.title.trim().is_empty() {
            format!("第{}章", chapter.number)
        } else {
            chapter.title.trim().to_string()
        };
        let excerpt = trim_text(content.trim(), 1200);
        let excerpt = if excerpt.is_empty() {
            "暂无正文".to_string()
        } else {
            excerpt
        };
        excerpt_lines.push(format!(
            "[第{}章《{}》节选]\n{excerpt}",
            chapter.number, title
        ));
    }
    let recent_excerpt = if excerpt_lines.is_empty() {
        "暂无。".to_string()
    } else {
        excerpt_lines.join("\n\n")
    };

    // 当前章节草稿
    let draft_text = {
        let s = current_draft.trim();
        if s.is_empty() {
            "当前章节暂无草稿。".to_string()
        } else {
            trim_text(s, 1600)
        }
    };

    let continuity_text = build_state_context_text(state_documents);
    let chapter_title_hint = if target_chapter.title.trim().is_empty() {
        format!("第{}章", target_chapter.number)
    } else {
        target_chapter.title.trim().to_string()
    };

    let title_label = if project.title.trim().is_empty() {
        "未命名小说".to_string()
    } else {
        project.title.trim().to_string()
    };

    // 本章节拍（Beats First Workflow）：若非空，作为最高优先级注入；若为空则跳过该段，
    // 同时也不在「请满足以下要求」里追加"必须逐条命中节拍"的硬约束（兼容旧流程）。
    let cleaned_beats: Vec<String> = beats
        .iter()
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .collect();
    let beats_section = if cleaned_beats.is_empty() {
        String::new()
    } else {
        let mut s = String::from(
            "[本章节拍]（最高优先级，必须逐条命中，不得跳过也不得新增大方向）\n",
        );
        for (i, b) in cleaned_beats.iter().enumerate() {
            s.push_str(&format!("{}. {}\n", i + 1, b));
        }
        s.push('\n');
        s
    };
    let beats_rule = if cleaned_beats.is_empty() {
        String::new()
    } else {
        "         2.1 必须依次推进上方[本章节拍]的每一条节拍，禁止跳过或新增大方向。\n".to_string()
    };

    let (seam_block, seam_rule) = chapter_seam_sections(last_chapter_end);
    let pending_hooks_section = pending_hooks_focus_section(chapter_num, state_documents);
    let recent_summaries_section =
        recent_chapter_summaries_section(chapter_num, state_documents, previous_materials);
    let continuity_hard_rule = if chapter_num >= 2 {
        CONTINUITY_SEAM_HARD_RULE
    } else {
        ""
    };
    let chapter1_json_output = if chapter_num <= 1 {
        chapter1_hooks_json_output_section()
    } else {
        ""
    };
    let extra_raw = project.extra_guidance.trim();
    let extra = if extra_raw.is_empty() {
        "无".to_string()
    } else {
        substitute_prompt_vars(extra_raw, last_chapter_end.trim())
    };

    let user_prompt = format!(
        "请为小说《{title_label}》创作第{n}章。\n\n\
         {narrative_constraints}\
         [创作目标]\n\
         - 目标章节：第{n}章\n\
         - 建议标题：{chapter_title_hint}\n\
         - 目标字数：{word_goal} 字（必须控制在目标上下浮动，不得明显偏离）\n\
         - 体裁：{genre}\n\n\
         {beats_section}\
         [小说设定]\n\
         - 故事核心：{premise}\n\
         - 主角与关键角色：{protagonists}\n\
         - 世界观与背景：{world_setting}\n\
         - 文风与节奏：{writing_style}\n\n\
         [总纲与后续方向]\n{outline}\n\n\
         [额外要求]\n{extra}\n\n\
         {atmosphere_section}\
         [前文摘要]\n{history_summary}\n\n\
         [最近章节正文节选]\n{recent_excerpt}\n\n\
         {recent_summaries_section}\
         {seam_block}\
         {pending_hooks_section}\
         [连续性档案]\n{continuity_text}\n\n\
         [当前章节现有草稿]\n{draft_text}\n\n\
         请满足以下要求：\n\
         1. 情节必须承接前文，不能与既有设定冲突。\n\
         {seam_rule}\
         {continuity_hard_rule}\
         2. 章节要有明显推进，不能只写重复铺垫。\n\
         {beats_rule}\
         3. 如果建议标题不合适，可以优化，但仍要符合当前情节。\n\
         3.1 正文字数必须尽量贴近目标字数，偏差控制在约 ±15% 内，禁止空洞重复凑字数。\n\
         4. 输出格式必须严格如下：\n\n\
         标题：章节标题\n\
         摘要：80字以内摘要\n\
         正文：\n\
         章节正文\n\
         {chapter1_json_output}",
        n = chapter_num,
        word_goal = project.chapter_word_goal,
        genre = blank_or(&project.genre, "未指定"),
        premise = blank_or(&project.premise, "未填写"),
        protagonists = blank_or(&project.protagonists, "未填写"),
        world_setting = blank_or(&project.world_setting, "未填写"),
        writing_style = blank_or(&project.writing_style, "未填写"),
        outline = blank_or(&project.outline, "未填写"),
        atmosphere_section = chapter_dynamic_bracket_section(chapter_num),
    );

    (system_prompt, user_prompt)
}

/// 构造「生成本章节拍」的 (system, user) prompt。
///
/// - 仅依赖最近若干章摘要 + 小说核心设定 + 大纲，不读全量状态档案；
/// - 要求输出 3-5 行短句，每行一条节拍，便于后续 `parse_beats_output` 解析。
pub fn build_beats_prompts(
    project: &NovelProject,
    target_chapter: &ChapterRecord,
    previous_materials: &[(ChapterRecord, String)],
) -> (String, String) {
    let system_prompt = "你是一名长篇小说章节节拍规划助手。\
你需要在正式写正文之前，先给出本章的 3-5 条节拍（Beats），每条节拍是一句话短句，描述本章的关键推进点。\
不要标号，不要解释，不要输出任何额外内容，只输出 3-5 行短句，每行一条节拍。"
        .to_string();

    // 复用与 build_generation_prompts 一致的「最近 12 章摘要 / 最近 3 章节选」节流，
    // 但节拍生成不需要节选，只取摘要即可。
    let summary_tail: Vec<&(ChapterRecord, String)> = previous_materials
        .iter()
        .rev()
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let mut summary_lines: Vec<String> = Vec::new();
    for (chapter, content) in &summary_tail {
        let summary = {
            let s = chapter.summary.trim();
            if s.is_empty() {
                make_summary(content, 100)
            } else {
                s.to_string()
            }
        };
        let title = if chapter.title.trim().is_empty() {
            format!("第{}章", chapter.number)
        } else {
            chapter.title.trim().to_string()
        };
        summary_lines.push(format!(
            "第{}章《{}》：{}",
            chapter.number, title, summary
        ));
    }
    let history_summary = if summary_lines.is_empty() {
        "暂无已完成章节。".to_string()
    } else {
        summary_lines.join("\n")
    };

    let title_label = if project.title.trim().is_empty() {
        "未命名小说".to_string()
    } else {
        project.title.trim().to_string()
    };
    let chapter_title_hint = if target_chapter.title.trim().is_empty() {
        format!("第{}章", target_chapter.number)
    } else {
        target_chapter.title.trim().to_string()
    };

    let user_prompt = format!(
        "请为小说《{title_label}》规划第{n}章的本章节拍。\n\n\
         [本章定位]\n\
         - 目标章节：第{n}章\n\
         - 建议标题：{chapter_title_hint}\n\
         - 目标字数：{word_goal} 字\n\
         - 体裁：{genre}\n\n\
         [小说核心]\n\
         - 故事核心：{premise}\n\
         - 主角与关键角色：{protagonists}\n\
         - 世界观与背景：{world_setting}\n\
         - 文风与节奏：{writing_style}\n\n\
         [总纲与后续方向]\n{outline}\n\n\
         [额外要求]\n{extra}\n\n\
         [前文摘要]\n{history_summary}\n\n\
         请输出 3-5 条本章节拍，每条短句 16~40 字，每行一条，遵守：\n\
         1. 节拍必须可执行：聚焦地点/人物/动作/冲突/揭示，不要写主旨口号。\n\
         2. 必须承接前文，不能与既有设定冲突。\n\
         3. 全部节拍合起来要让本章有明显推进。\n\
         4. 不要标号（不要 1.、- 等前缀），不要解释，只输出节拍正文，每行一条。\n",
        n = target_chapter.number,
        word_goal = project.chapter_word_goal,
        genre = blank_or(&project.genre, "未指定"),
        premise = blank_or(&project.premise, "未填写"),
        protagonists = blank_or(&project.protagonists, "未填写"),
        world_setting = blank_or(&project.world_setting, "未填写"),
        writing_style = blank_or(&project.writing_style, "未填写"),
        outline = blank_or(&project.outline, "未填写"),
        extra = blank_or(&project.extra_guidance, "无"),
    );

    (system_prompt, user_prompt)
}

/// 解析 LLM 输出的节拍：按行切分、去空、去 markdown 编号前缀、最多保留 5 条。
pub fn parse_beats_output(raw: &str) -> Vec<String> {
    let cleaned = raw.trim().trim_matches('`').trim();
    let mut beats: Vec<String> = Vec::new();
    for raw_line in cleaned.lines() {
        let mut line = raw_line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        // 去掉常见前缀：`- `、`* `、`• `、`1. `、`1) `、`1、`、`第1拍：` 等
        line = strip_beat_prefix(&line);
        let line = line.trim().trim_start_matches('"').trim_end_matches('"').trim();
        if line.is_empty() {
            continue;
        }
        beats.push(line.to_string());
        if beats.len() >= 5 {
            break;
        }
    }
    beats
}

fn strip_beat_prefix(line: &str) -> String {
    let mut s = line.to_string();
    // markdown 项目符号 / bullet
    for prefix in ["- ", "* ", "• ", "·", "—", "– "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start().to_string();
        }
    }
    // 数字编号：1. 2. 1) 2) 1、2、第1拍：
    let bytes = s.as_bytes();
    let mut idx = 0;
    // 可选「第」字
    if s.starts_with('第') {
        idx += '第'.len_utf8();
    }
    let start_digit = idx;
    while idx < bytes.len() && bytes[idx].is_ascii_digit() {
        idx += 1;
    }
    if idx > start_digit && idx < bytes.len() {
        // 跳过可选「拍/条/章」中文字符
        let rest = &s[idx..];
        let mut consumed = 0;
        for ch in rest.chars() {
            if matches!(ch, '拍' | '条' | '章') {
                consumed += ch.len_utf8();
            } else {
                break;
            }
        }
        let rest = &rest[consumed..];
        if let Some(stripped) = rest
            .strip_prefix("：")
            .or_else(|| rest.strip_prefix(": "))
            .or_else(|| rest.strip_prefix(":"))
            .or_else(|| rest.strip_prefix("、"))
            .or_else(|| rest.strip_prefix(". "))
            .or_else(|| rest.strip_prefix("."))
            .or_else(|| rest.strip_prefix(") "))
            .or_else(|| rest.strip_prefix(")"))
        {
            return stripped.trim_start().to_string();
        }
    }
    s
}

fn blank_or(s: &str, fallback: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t.to_string()
    }
}

/// 新建小说向导「AI 灵感生成」解析结果，对应 `NovelProject` 四字段。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitSettingsParsed {
    pub premise: String,
    pub protagonists: String,
    pub world_setting: String,
    pub writing_style: String,
}

impl InitSettingsParsed {
    /// 仅将非空字段写入 `project`，避免解析不完整时覆盖用户已有输入。
    pub fn merge_into(&self, project: &mut NovelProject) {
        if !self.premise.trim().is_empty() {
            project.premise = self.premise.clone();
        }
        if !self.protagonists.trim().is_empty() {
            project.protagonists = self.protagonists.clone();
        }
        if !self.world_setting.trim().is_empty() {
            project.world_setting = self.world_setting.clone();
        }
        if !self.writing_style.trim().is_empty() {
            project.writing_style = self.writing_style.clone();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InitSection {
    Premise,
    Protagonists,
    World,
    Style,
}

fn strip_heading_noise(mut s: &str) -> &str {
    s = s.trim();
    while s.starts_with('#') {
        s = s.trim_start_matches('#').trim_start();
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < bytes.len() {
        let after = &s[i..];
        if let Some(rest) = after
            .strip_prefix('.')
            .or_else(|| after.strip_prefix('、'))
            .or_else(|| after.strip_prefix(')'))
            .or_else(|| after.strip_prefix('）'))
        {
            s = rest.trim_start();
        }
    }
    s.trim()
}

fn is_premise_heading(h: &str, compact: &str) -> bool {
    if compact.contains("故事核心") && compact.contains("前提") {
        return true;
    }
    if compact.contains("故事核心") && (compact.contains("与前提") || compact.contains("和前提")) {
        return true;
    }
    if h == "故事核心" || compact == "故事核心" {
        return true;
    }
    // 短行且以「故事核心」开头，视为小节标题（避免吞掉长段正文）
    if h.starts_with("故事核心") {
        let after = h["故事核心".len()..].trim();
        if after.is_empty()
            || after.starts_with('/')
            || after.starts_with('／')
            || after.starts_with('与')
            || after.starts_with('和')
            || after.starts_with('：')
            || after.starts_with(':')
        {
            return true;
        }
        // 避免把正文「故事核心在于……」误判为标题
        let body_like = ["在于", "是", "围绕", "讲述", "描述", "展现", "通过", "从", "在"];
        if body_like.iter().any(|p| after.starts_with(*p)) {
            return false;
        }
    }
    false
}

fn classify_init_section_line(line: &str) -> Option<InitSection> {
    let h = strip_heading_noise(line);
    let compact: String = h.chars().filter(|c| !c.is_whitespace()).collect();
    if is_premise_heading(h, &compact) {
        return Some(InitSection::Premise);
    }
    if h.starts_with("主角与关键角色") {
        return Some(InitSection::Protagonists);
    }
    if h.starts_with("世界观与背景") {
        return Some(InitSection::World);
    }
    if h.starts_with("文风与节奏要求") {
        return Some(InitSection::Style);
    }
    None
}

/// 构造「新建小说 · 初始设定」的 (system, user) prompt。
/// `genre` 表示小说类型（如玄幻 / 悬疑），写入 user 上下文。
pub fn generate_init_settings_prompt(name: &str, genre: &str) -> (String, String) {
    let title = name.trim();
    let g = genre.trim();
    let system = "你是一名资深中文长篇小说架构师。仅用中文输出可直接粘贴进写作工作台的设定正文。\
禁止输出 Markdown 围栏（```）、JSON、YAML、表格；禁止自我解释或「以下为……」以外的多余前言——若必须过渡，至多一行后立即进入第一段。\
输出必须恰好四段：每段首行必须为下列方括号标签行之一，且标签行与原文字符完全一致（半角括号 [ ]、半角冒号 : 或中文冒号 ：均可）；标签行后即换行写正文，正文可多段落。\
四段都要有实质信息量，缺一不可。";

    let user = format!(
        "请基于下列信息构思小说骨架。\n\
         - 书名 / 暂定名：{title}\n\
         - 小说类型：{g}\n\n\
         请严格按下列四段输出（每段首行必须为标签行，下一行起为正文）：\n\n\
         [故事核心 / 前提]：\n\
         （须覆盖：核心冲突、主角目标、故事背景。）\n\n\
         [主角与关键角色]：\n\
         （须有主角姓名、性格、能力要点；另外写清 2～3 名重要配角（姓名或身份 + 简要作用）。）\n\n\
         [世界观与背景]：\n\
         （须交代：时代背景、主要地理区域、魔法体系或科技体系（二选一或与题材相符的等价设定亦可）。）\n\n\
         [文风与节奏要求]：\n\
         （须明示：叙事视角（如第一人称 / 第三人称）、文笔取向（简练 / 华丽等）、情境节奏偏好（快节奏 / 慢生活等）。）\n\n\
         重要：不要使用 ``` 围栏；不要使用除上述四类 [] 标签外的Markdown标题体系（避免 #）；四段正文均不得为空。\n\
         四段可按上述顺序自上而下排列。"
    );
    (system.to_string(), user)
}

/// 新建小说向导：估算目标章节数与单章字数。
pub fn generate_project_budget_prompt(project: &NovelProject) -> (String, String) {
    let title = blank_or(&project.title, "未命名小说");
    let genre = blank_or(&project.genre, "未指定");
    let premise = blank_or(&project.premise, "未填写");
    let protagonists = blank_or(&project.protagonists, "未填写");
    let world = blank_or(&project.world_setting, "未填写");
    let style = blank_or(&project.writing_style, "未填写");
    let system = "你是一名中文长篇小说商业策划编辑。请根据题材、故事复杂度与连载节奏，估算合理的全书章节数和单章目标字数。只输出 JSON，不要解释。";
    let user = format!(
        "请为以下小说估算篇幅，并严格输出 JSON：\n\n\
         - 书名：{title}\n\
         - 类型：{genre}\n\
         - 故事核心：{premise}\n\
         - 主角与关键角色：{protagonists}\n\
         - 世界观与背景：{world}\n\
         - 文风与节奏：{style}\n\n\
         输出格式：\n\
         {{\"target_chapters\": 120, \"chapter_word_goal\": 3000}}\n\n\
         约束：target_chapters 为 20-2000 的整数；chapter_word_goal 为 1000-8000 的整数。"
    );
    (system.to_string(), user)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectBudgetSuggestion {
    pub target_chapters: i32,
    pub chapter_word_goal: i32,
}

pub fn parse_project_budget_output(raw: &str) -> Option<ProjectBudgetSuggestion> {
    let cleaned = strip_outer_md_fence(raw.trim());
    let slice = if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
        &cleaned[start..=end]
    } else {
        cleaned.trim()
    };
    let v: serde_json::Value = serde_json::from_str(slice).ok()?;
    let chapters = v.get("target_chapters")?.as_i64()? as i32;
    let words = v.get("chapter_word_goal")?.as_i64()? as i32;
    Some(ProjectBudgetSuggestion {
        target_chapters: chapters.clamp(20, 2000),
        chapter_word_goal: words.clamp(1000, 8000),
    })
}

/// 新建小说向导：按目标章节数生成可写入 `story_state/outline.md` 的完整大纲。
pub fn generate_outline_prompt(project: &NovelProject) -> (String, String) {
    let title = blank_or(&project.title, "未命名小说");
    let genre = blank_or(&project.genre, "未指定");
    let premise = blank_or(&project.premise, "未填写");
    let protagonists = blank_or(&project.protagonists, "未填写");
    let world = blank_or(&project.world_setting, "未填写");
    let style = blank_or(&project.writing_style, "未填写");
    let chapters = project.target_chapters.max(1);
    let words = project.chapter_word_goal.max(500);
    let system = "你是一名中文长篇小说总编剧，擅长把全书目标拆成宏观大纲、分卷/阶段大纲与章节级细纲。只输出 Markdown 正文，不要代码围栏，不要解释。";
    let user = format!(
        "请为小说《{title}》生成 `outline.md`，写作时会作为长期状态档案注入 prompt。\n\n\
         [项目信息]\n\
         - 类型：{genre}\n\
         - 目标章节数：{chapters} 章\n\
         - 目标字/章：{words} 字\n\
         - 故事核心：{premise}\n\
         - 主角与关键角色：{protagonists}\n\
         - 世界观与背景：{world}\n\
         - 文风与节奏：{style}\n\n\
         [输出结构]\n\
         # 书籍大纲\n\n\
         ## 故事大纲（宏观）\n\
         写清起因、发展、高潮、结局、主线冲突与主题承诺。\n\n\
         ## 分卷 / 阶段大纲\n\
         按目标章节数切成 3-8 个阶段或分卷。每个阶段必须写明：章节范围、核心目标、主要矛盾、情感弧线、阶段钩子。\n\n\
         ## 细纲（章节级规划）\n\
         必须覆盖第1章到第{chapters}章。每章用 1 行或短条目，格式尽量统一：\n\
         ### 第N章 标题\n\
         - 场景：...\n\
         - 出场人物：...\n\
         - 核心事件：...\n\
         - 情绪/节奏：...\n\
         - 钩子/伏笔：...\n\n\
         [硬性要求]\n\
         - 不要写“待补充”。\n\
         - 细纲必须与目标章节数一致，不能只写前几章。\n\
         - 允许章节级规划简洁，但每章必须有可执行的剧情推进点。\n\
         - 不要输出 Markdown 代码围栏。"
    );
    (system.to_string(), user)
}

pub fn parse_outline_output(raw: &str) -> String {
    let cleaned = strip_outer_md_fence(raw.trim());
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("# 书籍大纲") {
        trimmed.to_string()
    } else {
        format!("# 书籍大纲\n\n{trimmed}")
    }
}

fn init_bracket_header_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // 单行小节标签，如 `[故事核心 / 前提]`；避免跨行抓取正文
        Regex::new(r"(?msi)\[\s*[^\[\]]+?\]\s*[：:]?").expect("valid regex")
    })
}

fn normalize_wide_brackets(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '［' => '[',
            '］' => ']',
            c => c,
        })
        .collect()
}

fn strip_outer_md_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let body = rest
            .find('\n')
            .map(|i| rest[i + 1..].trim_start())
            .unwrap_or(rest.trim());
        return if let Some(end) = body.rfind("```") {
            body[..end].trim().to_string()
        } else {
            body.trim().to_string()
        };
    }
    t.to_string()
}

fn classify_bracket_label(matched: &str) -> Option<InitSection> {
    let folded: String = matched
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '[' && *c != ']')
        .collect();
    if folded.contains("故事核心") {
        if folded.contains("前提")
            || folded.contains('/')
            || folded.contains('／')
            || folded == "故事核心"
            || folded.contains("与前提")
            || folded.contains("和前提")
        {
            return Some(InitSection::Premise);
        }
    }
    if folded.contains("主角与关键角色") {
        return Some(InitSection::Protagonists);
    }
    if folded.contains("世界观与背景") {
        return Some(InitSection::World);
    }
    if folded.contains("文风与节奏要求") {
        return Some(InitSection::Style);
    }
    None
}

/// 从 `generate_init_settings_prompt` 期望的 **[标签]：** 结构中解析四段；
/// 若未找齐四个锚点则返回 `None`。
pub fn parse_init_settings_bracketed(raw: &str) -> Option<InitSettingsParsed> {
    let text = normalize_wide_brackets(&strip_outer_md_fence(raw.trim()));
    if !text.contains('[') && !text.contains('［') {
        return None;
    }
    let re = init_bracket_header_regex();

    let mut best: [Option<(usize, usize)>; 4] = [None; 4];
    for m in re.find_iter(&text) {
        let s = m.as_str();
        let kind = classify_bracket_label(s)?;
        let idx = match kind {
            InitSection::Premise => 0usize,
            InitSection::Protagonists => 1,
            InitSection::World => 2,
            InitSection::Style => 3,
        };
        let hdr_start = m.start();
        let hdr_end = m.end();
        let take = match best[idx] {
            None => true,
            Some((prev_s, _)) => hdr_start < prev_s,
        };
        if take {
            best[idx] = Some((hdr_start, hdr_end));
        }
    }

    if best.iter().any(|slot| slot.is_none()) {
        return None;
    }

    fn section_order(i: usize) -> InitSection {
        match i {
            0 => InitSection::Premise,
            1 => InitSection::Protagonists,
            2 => InitSection::World,
            _ => InitSection::Style,
        }
    }

    let mut anchors: Vec<(usize, usize, InitSection)> = Vec::with_capacity(4);
    for i in 0..4 {
        let (hs, he) = best[i].unwrap();
        anchors.push((hs, he, section_order(i)));
    }
    anchors.sort_by_key(|a| a.0);

    let mut out = InitSettingsParsed::default();
    for i in 0..anchors.len() {
        let (_hs, hdr_end, sec) = anchors[i];
        let next_hdr = anchors.get(i + 1).map(|x| x.0).unwrap_or_else(|| text.len());
        let chunk = text[hdr_end..next_hdr].trim().to_string();
        match sec {
            InitSection::Premise => out.premise = chunk,
            InitSection::Protagonists => out.protagonists = chunk,
            InitSection::World => out.world_setting = chunk,
            InitSection::Style => out.writing_style = chunk,
        }
    }

    Some(out)
}

fn merge_init_pick(pref: String, fb: String) -> String {
    if !pref.trim().is_empty() {
        pref
    } else {
        fb
    }
}

fn bracket_has_any(hit: Option<&InitSettingsParsed>) -> bool {
    hit.map(|p| {
        !(p.premise.trim().is_empty()
            && p.protagonists.trim().is_empty()
            && p.world_setting.trim().is_empty()
            && p.writing_style.trim().is_empty())
    })
    .unwrap_or(false)
}

/// 聚合解析：优先方括号 Markdown 段落；与各段 legacy 启发式互补（避免单侧失败丢字段）。
pub fn parse_init_settings_output(raw: &str) -> InitSettingsParsed {
    let bracket_opt = parse_init_settings_bracketed(raw);
    let legacy = parse_legacy_init_settings(raw);
    let Some(ref br) = bracket_opt else {
        return legacy;
    };
    if !bracket_has_any(Some(br)) {
        return legacy;
    }
    InitSettingsParsed {
        premise: merge_init_pick(br.premise.clone(), legacy.premise.clone()),
        protagonists: merge_init_pick(br.protagonists.clone(), legacy.protagonists.clone()),
        world_setting: merge_init_pick(br.world_setting.clone(), legacy.world_setting.clone()),
        writing_style: merge_init_pick(br.writing_style.clone(), legacy.writing_style.clone()),
    }
}

/// 兼容旧模型的无括号四段标题 / orphan 容错解析。
pub fn parse_legacy_init_settings(raw: &str) -> InitSettingsParsed {
    let clean = raw.trim().trim_matches('`').trim();
    let lines: Vec<&str> = clean.lines().collect();
    let mut out = InitSettingsParsed::default();
    let mut cur: Option<InitSection> = None;
    let mut buf: Vec<String> = Vec::new();
    // 尚未进入任一段时累积的行（模型常把核心梗概写在第一个小标题前）
    let mut orphan: Vec<String> = Vec::new();

    fn flush(section: Option<InitSection>, buf: &mut Vec<String>, out: &mut InitSettingsParsed) {
        let joined = buf.join("\n").trim().to_string();
        buf.clear();
        let Some(s) = section else {
            return;
        };
        match s {
            InitSection::Premise => out.premise = joined,
            InitSection::Protagonists => out.protagonists = joined,
            InitSection::World => out.world_setting = joined,
            InitSection::Style => out.writing_style = joined,
        }
    }

    for raw_line in &lines {
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if cur.is_some() {
                buf.push(String::new());
            } else if !orphan.is_empty() {
                orphan.push(String::new());
            }
            continue;
        }
        if let Some(kind) = classify_init_section_line(trimmed) {
            // 模型跳过「故事核心」标题、直接把梗概写在「主角」之前：并入 premise
            if cur.is_none()
                && kind != InitSection::Premise
                && out.premise.is_empty()
                && !orphan.is_empty()
            {
                out.premise = orphan.join("\n").trim().to_string();
                orphan.clear();
            }
            flush(cur.take(), &mut buf, &mut out);
            // 显式出现「故事核心」标题时，丢弃标题前的短引言（多为套话）
            if kind == InitSection::Premise {
                orphan.clear();
            }
            // 允许「标题：正文」同在一行
            let rest = strip_init_header_prefix(trimmed, kind);
            cur = Some(kind);
            if !rest.is_empty() {
                buf.push(rest);
            }
            continue;
        }
        if let Some(ref _s) = cur {
            buf.push(line.to_string());
        } else {
            orphan.push(line.to_string());
        }
    }
    flush(cur, &mut buf, &mut out);
    if out.premise.trim().is_empty() && !orphan.is_empty() {
        out.premise = orphan.join("\n").trim().to_string();
    }
    out
}

fn strip_init_header_prefix(line: &str, kind: InitSection) -> String {
    let h = strip_heading_noise(line)
        .trim_end_matches(['：', ':'])
        .trim();
    let rest = match kind {
        InitSection::Premise => h
            .strip_prefix("故事核心 / 前提")
            .or_else(|| h.strip_prefix("故事核心/前提"))
            .or_else(|| h.strip_prefix("故事核心／前提"))
            .or_else(|| h.strip_prefix("故事核心与前提"))
            .or_else(|| h.strip_prefix("故事核心和前提"))
            .or_else(|| h.strip_prefix("故事核心")),
        InitSection::Protagonists => h.strip_prefix("主角与关键角色"),
        InitSection::World => h.strip_prefix("世界观与背景"),
        InitSection::Style => h.strip_prefix("文风与节奏要求"),
    };
    rest.unwrap_or("")
        .trim()
        .trim_start_matches(['：', ':'])
        .trim()
        .to_string()
}

/// 对齐 inkoswin `parse_generation_output`：解析「标题：/摘要：/正文：」三段。
/// 兼容 `#` 开头的 Markdown 标题 fallback；尾部 ```json 围栏（第一章悬念同步）会从正文中剥离。
pub fn parse_generation_output(raw_text: &str, fallback_title: &str) -> ChapterGenerationResult {
    let without_json = crate::state_sync::strip_audit_trailing_json_fence(raw_text);
    let clean = without_json.trim().trim_matches('`').trim();
    let lines: Vec<&str> = clean.lines().collect();

    let mut title = String::new();
    let mut summary = String::new();
    let mut body = String::new();
    let mut body_start: Option<usize> = None;

    for (index, raw_line) in lines.iter().enumerate() {
        let line = raw_line.trim();
        if title.is_empty() && line.starts_with('#') {
            title = line.trim_start_matches('#').trim().to_string();
            continue;
        }
        if title.is_empty()
            && (line.starts_with("标题：") || line.starts_with("标题:"))
        {
            title = line
                .splitn(2, |c| c == '：' || c == ':')
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string();
            continue;
        }
        if summary.is_empty()
            && (line.starts_with("摘要：") || line.starts_with("摘要:"))
        {
            summary = line
                .splitn(2, |c| c == '：' || c == ':')
                .nth(1)
                .unwrap_or("")
                .trim()
                .to_string();
            continue;
        }
        if line == "正文" || line == "正文：" || line == "正文:" {
            body_start = Some(index + 1);
            break;
        }
    }

    if let Some(start) = body_start {
        body = lines[start..].join("\n").trim().to_string();
    } else {
        let mut filtered: Vec<&str> = Vec::new();
        let mut seen_non_marker = false;
        for raw_line in &lines {
            let line = raw_line.trim();
            if line.starts_with("标题：") || line.starts_with("标题:") {
                continue;
            }
            if line.starts_with("摘要：") || line.starts_with("摘要:") {
                continue;
            }
            if line == "正文" || line == "正文：" || line == "正文:" {
                continue;
            }
            if !seen_non_marker && line.starts_with('#') {
                continue;
            }
            seen_non_marker = true;
            filtered.push(raw_line);
        }
        body = filtered.join("\n").trim().to_string();
    }

    if title.is_empty() {
        let f = fallback_title.trim();
        title = if f.is_empty() {
            "未命名章节".to_string()
        } else {
            f.to_string()
        };
    }
    if summary.is_empty() {
        summary = make_summary(&body, DEFAULT_CHAPTER_SUMMARY_LIMIT);
    }

    ChapterGenerationResult {
        title,
        content: body,
        summary,
        raw_text: raw_text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn substitute_prompt_vars_replaces_last_chapter_end() {
        let s = "衔接：{{LAST_CHAPTER_END}}，结束。";
        assert_eq!(
            substitute_prompt_vars(s, "她推开门"),
            "衔接：她推开门，结束。"
        );
    }

    #[test]
    fn core_narrative_constraints_includes_pov_and_style() {
        let mut project = NovelProject::default();
        project.pov = crate::project::NarrativePov::FirstPerson;
        project.style_preset = crate::project::StylePreset::HardboiledDetective;
        let s = core_narrative_constraints_section(&project);
        assert!(s.contains("[核心叙事强制约束]"));
        assert!(s.contains("第一人称"));
        assert!(s.contains("冷硬派"));
    }

    #[test]
    #[test]
    fn build_generation_prompts_includes_core_narrative_constraints() {
        let project = NovelProject {
            pov: crate::project::NarrativePov::ThirdPersonLimited,
            density: crate::project::NarrativeDensity::Medium,
            style_preset: crate::project::StylePreset::Standard,
            ..NovelProject::default()
        };
        let target = ChapterRecord {
            number: 2,
            title: "第二章".into(),
            summary: String::new(),
            status: "draft".into(),
            word_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            beats: Vec::new(),
            end_snapshot: String::new(),
        };
        let (sys, user) = build_generation_prompts(
            &project,
            &target,
            &[],
            &[],
            "",
            &HashMap::new(),
            "",
        );
        assert!(user.contains("[核心叙事强制约束]"));
        assert!(sys.contains("[核心叙事强制约束]"));
        assert!(!user.contains("林夜"));
        assert!(!user.contains("停尸房"));
    }

    #[test]
    fn get_chapter_dynamic_prompt_ch1_has_prologue() {
        let p = get_chapter_dynamic_prompt(1);
        assert!(p.contains("第一章专属创作指令"));
        assert!(p.contains("反大纲化与悬念构建"));
        assert!(p.contains("信息遮蔽原则"));
        assert!(p.contains("信息增量"));
    }

    #[test]
    fn parse_audit_hooks_output_parses_fence() {
        let raw = "```json\n{\"hooks\":[{\"source\":\"神秘短信\",\"type\":\"世界秘密\",\"status\":\"active\",\"urgency\":4}]}\n```";
        let r = parse_audit_hooks_output(raw).unwrap();
        assert_eq!(r.hooks.len(), 1);
        assert!(r.hooks[0].source.contains("神秘短信"));
    }

    #[test]
    fn hooks_report_to_markdown_formats_three_fields() {
        let report = AuditHooksReport {
            hooks: vec![AuditHookEntry {
                source: "雕像裂开".into(),
                hook_type: "世界秘密".into(),
                status: "active".into(),
                urgency: 5,
            }],
        };
        let md = hooks_report_to_pending_hooks_markdown(&report);
        assert!(md.contains("[线索片段]"));
        assert!(md.contains("[潜在指向]"));
        assert!(md.contains("雕像裂开"));
    }

    #[test]
    fn get_chapter_dynamic_prompt_ch2_has_continuity_hooks() {
        let p = get_chapter_dynamic_prompt(2);
        assert!(p.contains("连贯性要求"));
        assert!(p.contains("pending_hooks"));
    }

    #[test]
    fn parse_generation_output_strips_trailing_json_fence() {
        let raw = "标题：夜\n摘要：短\n正文：\n她推开门。\n\n```json\n{\"summary\":\"hooks\",\"updates\":[]}\n```";
        let r = parse_generation_output(raw, "第1章");
        assert!(r.content.contains("她推开门"));
        assert!(!r.content.contains("```json"));
        assert!(!r.content.contains("\"updates\""));
    }

    #[test]
    fn build_generation_prompts_ch1_includes_json_output_rule() {
        let project = NovelProject::default();
        let target = ChapterRecord {
            number: 1,
            title: "第一章".into(),
            summary: String::new(),
            status: "draft".into(),
            word_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            beats: Vec::new(),
            end_snapshot: String::new(),
        };
        let (_sys, user) = build_generation_prompts(
            &project,
            &target,
            &[],
            &[],
            "",
            &HashMap::new(),
            "",
        );
        assert!(user.contains("pending_hooks.md"));
        assert!(user.contains("第一章专属创作指令"));
    }

    #[test]
    fn parse_chapter_context_output_parses_fence() {
        let raw = "```json\n{\"summary\":\"主角抵达城北\",\"hooks\":[\"左眼异变\"],\"state_updates\":{\"location\":\"城北废站\",\"inventory\":\"旧罗盘\"}}\n```";
        let r = parse_chapter_context_output(raw).unwrap();
        assert!(r.summary.contains("城北"));
        assert_eq!(r.hooks.len(), 1);
        assert!(r.state_updates.location.contains("废站"));
    }

    #[test]
    fn extract_recent_chapter_summary_blocks_takes_last_two() {
        let md = "# 各章摘要\n\n## 第1章 开\n- 摘要：a\n\n## 第2章 中\n- 摘要：b\n\n## 第3章 末\n- 摘要：c\n";
        let blocks = extract_recent_chapter_summary_blocks(md, 2);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("第2章"));
        assert!(blocks[1].contains("第3章"));
    }

    #[test]
    fn hooks_strings_to_markdown_formats_entries() {
        let md = hooks_strings_to_pending_hooks_markdown(&["神秘短信".into()]);
        assert!(md.contains("[线索片段]"));
        assert!(md.contains("神秘短信"));
    }

    #[test]
    fn build_generation_prompts_ch2_includes_recent_summaries_and_hard_rule() {
        let project = NovelProject::default();
        let target = ChapterRecord {
            number: 2,
            title: "第二章".into(),
            summary: String::new(),
            status: "draft".into(),
            word_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            beats: Vec::new(),
            end_snapshot: String::new(),
        };
        let mut docs = HashMap::new();
        docs.insert(
            "chapter_summaries.md".to_string(),
            "## 第1章 一\n- 摘要：开篇\n\n## 第2章 二\n- 摘要：承接\n".to_string(),
        );
        let (_sys, user) = build_generation_prompts(
            &project,
            &target,
            &[],
            &[],
            "",
            &docs,
            "她推开门。",
        );
        assert!(user.contains("[最近两章摘要档]"));
        assert!(user.contains("禁止跳跃时空"));
        assert!(user.contains("[待回收伏笔（必须推进）]"));
    }

    #[test]
    fn build_generation_prompts_ch2_includes_pending_hooks_focus() {
        let project = NovelProject::default();
        let target = ChapterRecord {
            number: 2,
            title: "第二章".into(),
            summary: String::new(),
            status: "draft".into(),
            word_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            beats: Vec::new(),
            end_snapshot: String::new(),
        };
        let mut docs = HashMap::new();
        docs.insert(
            "pending_hooks.md".to_string(),
            "## 核心伏笔\n- 左眼金光".to_string(),
        );
        let (_sys, user) = build_generation_prompts(
            &project,
            &target,
            &[],
            &[],
            "",
            &docs,
            "",
        );
        assert!(user.contains("[待回收伏笔（必须推进）]"));
        assert!(user.contains("左眼金光"));
    }

    #[test]
    fn chapter_seam_sections_empty_when_no_tail() {
        let (block, rule) = chapter_seam_sections("");
        assert!(block.is_empty());
        assert!(rule.is_empty());
    }

    #[test]
    fn build_generation_prompts_injects_seam_when_last_chapter_end_set() {
        let project = NovelProject {
            title: "测试书".into(),
            genre: "玄幻".into(),
            premise: "核心".into(),
            extra_guidance: "自定义 {{LAST_CHAPTER_END}}".into(),
            chapter_word_goal: 3000,
            ..NovelProject::default()
        };
        let target = ChapterRecord {
            number: 2,
            title: "第二章".into(),
            summary: String::new(),
            status: "draft".into(),
            word_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            beats: Vec::new(),
            end_snapshot: String::new(),
        };
        let prev = ChapterRecord {
            number: 1,
            title: "第一章".into(),
            summary: "摘要".into(),
            status: "generated".into(),
            word_count: 100,
            created_at: String::new(),
            updated_at: String::new(),
            beats: Vec::new(),
            end_snapshot: String::new(),
        };
        let prev_body = "前文很长。".repeat(200);
        let materials = vec![(prev, prev_body)];
        let tail = "她停在城门口，雨还在下。";
        let (_sys, user) = build_generation_prompts(
            &project,
            &target,
            &materials,
            &[],
            "",
            &HashMap::new(),
            tail,
        );
        assert!(user.contains("[上一章末尾衔接点]"));
        assert!(user.contains(tail));
        assert!(user.contains("零秒缝合"));
        assert!(user.contains("自定义 她停在城门口，雨还在下。"));
    }

    #[test]
    fn build_generation_prompts_no_seam_for_first_chapter() {
        let project = NovelProject::default();
        let target = ChapterRecord {
            number: 1,
            title: "第一章".into(),
            summary: String::new(),
            status: "draft".into(),
            word_count: 0,
            created_at: String::new(),
            updated_at: String::new(),
            beats: Vec::new(),
            end_snapshot: String::new(),
        };
        let (_sys, user) = build_generation_prompts(
            &project,
            &target,
            &[],
            &[],
            "",
            &HashMap::new(),
            "",
        );
        assert!(!user.contains("[上一章末尾衔接点]"));
        assert!(!user.contains("零秒缝合"));
    }

    #[test]
    fn parse_standard_form() {
        let raw = "标题：夜雨\n摘要：主角归家遇故人\n正文：\n她推开门……";
        let r = parse_generation_output(raw, "第1章");
        assert_eq!(r.title, "夜雨");
        assert_eq!(r.summary, "主角归家遇故人");
        assert!(r.content.starts_with("她推开门"));
    }

    #[test]
    fn parse_markdown_fallback() {
        let raw = "# 夜雨\n\n她推开门……";
        let r = parse_generation_output(raw, "第1章");
        assert_eq!(r.title, "夜雨");
        assert!(r.summary.len() > 0);
        assert!(r.content.starts_with("她推开门"));
    }

    #[test]
    fn beats_parse_strips_common_prefixes() {
        let raw = "1. 主角到达雪山脚下\n- 与师妹相遇并争执\n* 突遇黑衣杀手围攻\n第4拍：师妹被劫走\n5) 主角立誓追凶";
        let beats = parse_beats_output(raw);
        assert_eq!(beats.len(), 5);
        assert_eq!(beats[0], "主角到达雪山脚下");
        assert_eq!(beats[1], "与师妹相遇并争执");
        assert_eq!(beats[2], "突遇黑衣杀手围攻");
        assert_eq!(beats[3], "师妹被劫走");
        assert_eq!(beats[4], "主角立誓追凶");
    }

    #[test]
    fn beats_parse_caps_at_five() {
        let raw = (1..=10)
            .map(|i| format!("{i}. beat-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let beats = parse_beats_output(&raw);
        assert_eq!(beats.len(), 5);
        assert_eq!(beats[0], "beat-1");
        assert_eq!(beats[4], "beat-5");
    }

    #[test]
    fn init_settings_parse_four_sections() {
        let raw = "以下为设定。\n\n\
             故事核心 / 前提\n\
             核心A\n\
             核心B\n\n\
             主角与关键角色\n\
             角色线\n\n\
             世界观与背景\n\
             世界\n\n\
             文风与节奏要求\n\
             文风\n";
        let p = parse_init_settings_output(raw);
        assert_eq!(p.premise, "核心A\n核心B");
        assert_eq!(p.protagonists, "角色线");
        assert_eq!(p.world_setting, "世界");
        assert_eq!(p.writing_style, "文风");
    }

    #[test]
    fn init_settings_markdown_story_core_only_heading() {
        let raw = "## 故事核心\n\
             这是核心段落。\n\n\
             主角与关键角色\n\
             张三\n";
        let p = parse_init_settings_output(raw);
        assert_eq!(p.premise, "这是核心段落。");
        assert_eq!(p.protagonists, "张三");
    }

    #[test]
    fn init_settings_preamble_before_protagonists_becomes_premise() {
        let raw = "在一个赛博朋克都市里，主角寻找失踪的妹妹。\n\n\
             主角与关键角色\n\
             主角：林夜\n";
        let p = parse_init_settings_output(raw);
        assert!(p.premise.contains("赛博朋克"));
        assert!(p.protagonists.contains("林夜"));
    }

    #[test]
    fn init_settings_numbered_premise_heading() {
        let raw = "1. 故事核心 / 前提\n\
             核心内容\n\n\
             主角与关键角色\n\
             配角\n";
        let p = parse_init_settings_output(raw);
        assert_eq!(p.premise, "核心内容");
        assert_eq!(p.protagonists, "配角");
    }

    #[test]
    fn init_settings_bracketed_four_blocks() {
        let raw = "好的，请查收。\n\n\
             [故事核心 / 前提]:\n\
             - 冲突：A\n\
             - 目标：B\n\
             - 背景：C\n\n\
             [主角与关键角色]:\n\
             主角林某；配角甲乙。\n\n\
             [世界观与背景]:\n\
             架空古代，东海郡，灵脉体系。\n\n\
             [文风与节奏要求]:\n\
             第三人称；简练；快节奏。\n";
        assert!(parse_init_settings_bracketed(raw).is_some());
        let p = parse_init_settings_output(raw);
        assert!(p.premise.contains("冲突"));
        assert!(p.protagonists.contains("林某"));
        assert!(p.world_setting.contains("灵脉"));
        assert!(p.writing_style.contains("第三人称"));
    }

    #[test]
    fn init_settings_bracketed_wide_brackets() {
        let raw = "\
             ［故事核心 / 前提］：\n梗概一行\n\
             \n\
             [主角与关键角色]:\n张三\n\
             \n\
             [世界观与背景]\n某地\n\
             \n\
             [文风与节奏要求]:\n第一人称。\n";
        let p = parse_init_settings_output(raw);
        assert_eq!(p.premise.lines().next().unwrap().trim(), "梗概一行");
        assert!(p.protagonists.contains("张三"));
        assert!(p.world_setting.contains("某地"));
        assert!(p.writing_style.contains("第一人称"));
    }

    #[test]
    fn init_settings_fallback_when_brackets_incomplete() {
        let raw = "[故事核心 / 前提]:\n仅此一段\n";
        assert!(parse_init_settings_bracketed(raw).is_none());
        assert!(parse_init_settings_output(raw).premise.contains("仅此"));
    }

    #[test]
    fn init_settings_same_line_after_colon() {
        let raw = "故事核心 / 前提：一行核心\n\n\
             主角与关键角色：张三\n\n\
             世界观与背景\n\
             架空\n\n\
             文风与节奏要求\n\
             快";
        let p = parse_init_settings_output(raw);
        assert_eq!(p.premise, "一行核心");
        assert_eq!(p.protagonists, "张三");
        assert_eq!(p.world_setting, "架空");
        assert_eq!(p.writing_style, "快");
    }

    #[test]
    fn parse_project_budget_output_reads_json_fence() {
        let raw = "```json\n{\"target_chapters\": 180, \"chapter_word_goal\": 3200}\n```";
        let p = parse_project_budget_output(raw).unwrap();
        assert_eq!(p.target_chapters, 180);
        assert_eq!(p.chapter_word_goal, 3200);
    }

    #[test]
    fn parse_outline_output_adds_title_when_missing() {
        let raw = "## 故事大纲（宏观）\n主角踏入旧城。";
        let out = parse_outline_output(raw);
        assert!(out.starts_with("# 书籍大纲"));
        assert!(out.contains("旧城"));
    }
}
