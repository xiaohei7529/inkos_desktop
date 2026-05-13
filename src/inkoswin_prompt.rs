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

/// 构造生成章节的 (system, user) prompt。严格对齐 inkoswin.
pub fn build_generation_prompts(
    project: &NovelProject,
    target_chapter: &ChapterRecord,
    previous_materials: &[(ChapterRecord, String)],
    current_draft: &str,
    state_documents: &HashMap<String, String>,
) -> (String, String) {
    let system_prompt = "你是一名资深中文长篇小说作者。请根据已有设定续写章节，保持人物性格、世界观、伏笔和叙事节奏一致。不要解释创作过程，不要输出额外说明，只输出小说章节内容。".to_string();

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

    let user_prompt = format!(
        "请为小说《{title_label}》创作第{n}章。\n\n\
         [创作目标]\n\
         - 目标章节：第{n}章\n\
         - 建议标题：{chapter_title_hint}\n\
         - 目标字数：{word_goal} 字（必须控制在目标上下浮动，不得明显偏离）\n\
         - 体裁：{genre}\n\n\
         [小说设定]\n\
         - 故事核心：{premise}\n\
         - 主角与关键角色：{protagonists}\n\
         - 世界观与背景：{world_setting}\n\
         - 文风与节奏：{writing_style}\n\n\
         [总纲与后续方向]\n{outline}\n\n\
         [额外要求]\n{extra}\n\n\
         [前文摘要]\n{history_summary}\n\n\
         [最近章节正文节选]\n{recent_excerpt}\n\n\
         [连续性档案]\n{continuity_text}\n\n\
         [当前章节现有草稿]\n{draft_text}\n\n\
         请满足以下要求：\n\
         1. 情节必须承接前文，不能与既有设定冲突。\n\
         2. 章节要有明显推进，不能只写重复铺垫。\n\
         3. 如果建议标题不合适，可以优化，但仍要符合当前情节。\n\
         3.1 正文字数必须尽量贴近目标字数，偏差控制在约 ±15% 内，禁止空洞重复凑字数。\n\
         4. 输出格式必须严格如下：\n\n\
         标题：章节标题\n\
         摘要：80字以内摘要\n\
         正文：\n\
         章节正文\n",
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

fn blank_or(s: &str, fallback: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t.to_string()
    }
}

/// 对齐 inkoswin `parse_generation_output`：解析「标题：/摘要：/正文：」三段。
/// 兼容 `#` 开头的 Markdown 标题 fallback。
pub fn parse_generation_output(raw_text: &str, fallback_title: &str) -> ChapterGenerationResult {
    let clean = raw_text.trim().trim_matches('`').trim();
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
}
