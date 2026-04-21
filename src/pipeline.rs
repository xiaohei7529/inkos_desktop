//! 写作管线五段式（plan → compose → draft → audit → revise），与 Narcooo/inkos `agent` 命令对齐。
//!
//! 此模块只生成 prompt 文本与上下文，让 `app.rs` / `llm` 去执行实际 LLM 调用。
//! 中间产物会落到 `story/runtime/chapter-XXXX.*` 下，用 [`crate::runtime`] 维护。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::intent;
use crate::project::{NovelProject, ProjectStore};
use crate::runtime::{PipelineContext, RuleStack};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineStage {
    Plan,
    Compose,
    Draft,
    Audit,
    Revise,
}

impl PipelineStage {
    pub fn label(&self) -> &'static str {
        match self {
            PipelineStage::Plan => "Plan · 规划",
            PipelineStage::Compose => "Compose · 取材",
            PipelineStage::Draft => "Draft · 起草",
            PipelineStage::Audit => "Audit · 审计",
            PipelineStage::Revise => "Revise · 改稿",
        }
    }

    pub fn slug(&self) -> &'static str {
        match self {
            PipelineStage::Plan => "plan",
            PipelineStage::Compose => "compose",
            PipelineStage::Draft => "draft",
            PipelineStage::Audit => "audit",
            PipelineStage::Revise => "revise",
        }
    }

    pub fn all() -> &'static [PipelineStage] {
        &[
            PipelineStage::Plan,
            PipelineStage::Compose,
            PipelineStage::Draft,
            PipelineStage::Audit,
            PipelineStage::Revise,
        ]
    }
}

/// 公共 system prompt 头部。
pub fn base_system_prompt(project: &NovelProject) -> String {
    let title = project.title.trim();
    let style = if project.writing_style.trim().is_empty() {
        "保持中文长篇小说一线作者级别的语感与节奏".to_string()
    } else {
        project.writing_style.trim().to_string()
    };
    let genre = if project.genre.trim().is_empty() {
        "中文长篇小说".to_string()
    } else {
        project.genre.trim().to_string()
    };
    format!(
        "你是 InkOS 写作管线中的协作智能体。\n小说：《{title}》\n题材：{genre}\n文风约束：{style}\n你必须严格遵守作者长期意图与近期焦点（详见后文 governance 块），\n不允许人为延展剧情、不允许跳章、不允许偏离世界观。"
    )
}

/// 把 author_intent / current_focus 拼成 governance 块。
pub fn governance_block(novel_root: &Path) -> String {
    intent::governance_prompt(novel_root, 1500)
}

pub fn build_plan_prompt(
    store: &ProjectStore,
    project: &NovelProject,
    chapter_no: i32,
    chapter_title_hint: &str,
) -> String {
    let mut out = String::new();
    out.push_str("## 任务\n");
    out.push_str(&format!(
        "你正在为《{}》撰写第 {chapter_no} 章「{chapter_title_hint}」的 *章节意图卡（Intent Card）*。\n",
        project.title
    ));
    out.push_str("请用 6 个段落给出：\n");
    out.push_str("1. 本章核心目标（1 句）\n");
    out.push_str("2. 必须发生的关键事件（3-6 条）\n");
    out.push_str("3. 主要人物的情绪线变化\n");
    out.push_str("4. 与已有伏笔/钩子的呼应（请参考 pending_hooks.md / particle_ledger.md）\n");
    out.push_str("5. 本章新增伏笔（如无写「无」）\n");
    out.push_str("6. 写作禁忌（哪些套路、桥段、用语本章不允许出现）\n\n");

    out.push_str("## 当前长期治理\n");
    out.push_str(&governance_block(store.root()));

    out.push_str("\n## 上一章末态摘要\n");
    if let Some(prev) = project.chapters.iter().filter(|c| c.number == chapter_no - 1).next() {
        if !prev.summary.trim().is_empty() {
            out.push_str(&prev.summary);
            out.push('\n');
        } else {
            out.push_str("（无）\n");
        }
    } else {
        out.push_str("（这是第一章）\n");
    }
    out
}

pub fn build_compose_prompt(
    store: &ProjectStore,
    project: &NovelProject,
    chapter_no: i32,
    intent_md: &str,
) -> String {
    let mut out = String::new();
    out.push_str("## 任务\n");
    out.push_str(&format!(
        "请为《{}》第 {chapter_no} 章准备 *素材清单（Compose Sheet）*：从下列状态档案中抽取本章会动用到的人物、场景、道具、伏笔，并标注它们当前状态。\n\n",
        project.title
    ));
    out.push_str("## 上一步：章节意图卡\n");
    out.push_str(intent_md);
    out.push_str("\n\n## 状态档案目录\n");
    out.push_str("`story_state/` 下的 9 个 Markdown 文件请按需引用：\n");
    out.push_str("- book_rules.md / current_state.md / particle_ledger.md / pending_hooks.md\n");
    out.push_str("- chapter_summaries.md / novel_brief.md / subplot_board.md / emotional_arcs.md / character_matrix.md\n\n");
    out.push_str("## 输出格式\n请输出 Markdown 表格 + 简短点评，不需要写小说正文。\n\n");
    out.push_str("## 长期治理\n");
    out.push_str(&governance_block(store.root()));
    out
}

pub fn build_draft_prompt(
    store: &ProjectStore,
    project: &NovelProject,
    chapter_no: i32,
    chapter_title: &str,
    intent_md: &str,
    compose_md: &str,
    word_goal: i32,
    word_tolerance: i32,
) -> String {
    let mut out = String::new();
    out.push_str("## 任务\n");
    out.push_str(&format!(
        "正式撰写《{}》第 {chapter_no} 章「{chapter_title}」。\n",
        project.title
    ));
    out.push_str(&format!(
        "目标字数：{word_goal} 字，允许波动 ±{word_tolerance} 字。\n请以第三人称限知/全知视角写作，遵守文风指纹与 governance 块。\n\n",
    ));

    out.push_str("## 章节意图卡\n");
    out.push_str(intent_md);
    out.push_str("\n\n## 素材清单\n");
    out.push_str(compose_md);
    out.push_str("\n\n## 输出格式\n");
    out.push_str("仅输出章节正文，不要写章节标题、不要写「（完）」之类。请充分使用动作 + 心理 + 对白的混合节奏。\n\n");
    out.push_str("## 长期治理\n");
    out.push_str(&governance_block(store.root()));
    out
}

pub fn build_audit_prompt(
    store: &ProjectStore,
    project: &NovelProject,
    chapter_no: i32,
    body: &str,
) -> String {
    let mut out = String::new();
    out.push_str("## 任务\n");
    out.push_str(&format!(
        "请按 InkOS 33 维审计标准审计《{}》第 {chapter_no} 章正文，输出 *审计报告*（不是改稿）。\n\n",
        project.title
    ));
    out.push_str(&crate::audit33::checklist_markdown());
    out.push_str("\n\n## 正文\n");
    out.push_str(body);
    out.push_str("\n\n## 长期治理\n");
    out.push_str(&governance_block(store.root()));
    out
}

pub fn build_revise_prompt(
    store: &ProjectStore,
    project: &NovelProject,
    chapter_no: i32,
    body: &str,
    audit_report: &str,
    word_goal: i32,
    word_tolerance: i32,
) -> String {
    let mut out = String::new();
    out.push_str("## 任务\n");
    out.push_str(&format!(
        "请基于下列审计报告，逐条修正《{}》第 {chapter_no} 章正文，并最终输出 *改稿后的完整章节正文*。\n",
        project.title
    ));
    out.push_str(&format!(
        "字数目标 {word_goal} ± {word_tolerance}。请避免无意义扩写。\n\n"
    ));
    out.push_str("## 审计报告\n");
    out.push_str(audit_report);
    out.push_str("\n\n## 原稿\n");
    out.push_str(body);
    out.push_str("\n\n## 长期治理\n");
    out.push_str(&governance_block(store.root()));
    out
}

/// 派生 per-chapter context（写到 runtime/chapter-XXXX.context.json）。
pub fn make_context(
    project: &NovelProject,
    vendor: &str,
    model: &str,
    temperature: &str,
    max_tokens: &str,
    word_goal: i32,
    word_tolerance: i32,
    state_files: Vec<String>,
    author_intent_chars: usize,
    current_focus_chars: usize,
    style_loaded: bool,
) -> PipelineContext {
    let _ = project;
    PipelineContext {
        vendor: vendor.to_string(),
        model: model.to_string(),
        temperature: temperature.to_string(),
        max_tokens: max_tokens.to_string(),
        word_goal,
        word_tolerance,
        state_files,
        author_intent_chars,
        current_focus_chars,
        style_fingerprint_loaded: style_loaded,
    }
}

/// 由 book_rules / intent / focus 中提取规则字符串列表。
pub fn collect_rule_stack(
    novel_root: &Path,
    project: &NovelProject,
    word_goal: i32,
    word_tolerance: i32,
) -> RuleStack {
    let _ = project;
    let book_rules = std::fs::read_to_string(novel_root.join("story_state/book_rules.md"))
        .unwrap_or_default();
    let intent = intent::read_author_intent(novel_root);
    let focus = intent::read_current_focus(novel_root);
    let style = std::fs::read_to_string(novel_root.join("story/style_fingerprint.md"))
        .unwrap_or_default();

    RuleStack {
        from_book_rules: extract_bullets(&book_rules, 12),
        from_author_intent: extract_bullets(&intent, 8),
        from_current_focus: extract_bullets(&focus, 8),
        from_style_fingerprint: extract_bullets(&style, 8),
        effective_max_words: word_goal + word_tolerance,
        effective_min_words: (word_goal - word_tolerance).max(0),
    }
}

fn extract_bullets(text: &str, limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        let stripped = l
            .trim_start_matches('-')
            .trim_start_matches('*')
            .trim_start_matches('•')
            .trim();
        if stripped.is_empty() || stripped.starts_with('#') || stripped.starts_with('>') {
            continue;
        }
        if l.starts_with('-') || l.starts_with('*') || l.starts_with('•') {
            out.push(stripped.to_string());
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}
