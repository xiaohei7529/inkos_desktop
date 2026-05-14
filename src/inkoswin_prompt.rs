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
        ""
    } else {
        "         2.1 必须依次推进上方[本章节拍]的每一条节拍，禁止跳过或新增大方向。\n"
    };

    let user_prompt = format!(
        "请为小说《{title_label}》创作第{n}章。\n\n\
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
         [前文摘要]\n{history_summary}\n\n\
         [最近章节正文节选]\n{recent_excerpt}\n\n\
         [连续性档案]\n{continuity_text}\n\n\
         [当前章节现有草稿]\n{draft_text}\n\n\
         请满足以下要求：\n\
         1. 情节必须承接前文，不能与既有设定冲突。\n\
         2. 章节要有明显推进，不能只写重复铺垫。\n\
         {beats_rule}\
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
    fn beats_parse_ignores_blank_lines_and_fences() {
        let raw = "```\n\n  - a\n\n  b\n\n```";
        let beats = parse_beats_output(raw);
        assert_eq!(beats, vec!["a".to_string(), "b".to_string()]);
    }
}
