//! 章节 Markdown 与字数工具（对齐 `inkoswin.project_store`）。

/// 与 Python `count_story_units` 一致：去掉所有空白后的字符数。
pub fn count_story_units(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

pub const DEFAULT_CHAPTER_SUMMARY_LIMIT: usize = 90;

pub fn make_summary(text: &str, limit: usize) -> String {
    let stripped = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let n = stripped.chars().count();
    if n <= limit {
        return stripped;
    }
    // 首句优先：若首句完整且不超长，优先使用，避免摘要总是机械截断。
    if let Some(first_sentence) = first_sentence(&stripped) {
        if first_sentence.chars().count() <= limit {
            return first_sentence.to_string();
        }
    }
    truncated_with_ellipsis(&stripped, limit)
}

fn first_sentence(text: &str) -> Option<&str> {
    let mut end_idx = None;
    for (idx, ch) in text.char_indices() {
        if matches!(ch, '。' | '！' | '？' | '!' | '?') {
            end_idx = Some(idx + ch.len_utf8());
            break;
        }
    }
    end_idx.map(|n| text[..n].trim()).filter(|s| !s.is_empty())
}

fn truncated_with_ellipsis(text: &str, limit: usize) -> String {
    let truncated: String = text.chars().take(limit).collect();
    format!("{}...", truncated.trim_end())
}

pub fn compose_chapter_markdown(title: &str, content: &str) -> String {
    let clean_title = if title.trim().is_empty() {
        "未命名章节"
    } else {
        title.trim()
    };
    let clean_content = content.trim_end();
    if clean_content.is_empty() {
        format!("# {clean_title}\n")
    } else {
        format!("# {clean_title}\n\n{clean_content}\n")
    }
}

pub fn parse_chapter_markdown(raw_text: &str) -> (String, String) {
    let text = raw_text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return (String::new(), String::new());
    }
    let mut lines = text.lines();
    let first = lines.next().unwrap_or("");
    if first.starts_with('#') {
        let title = first.trim_start_matches('#').trim().to_string();
        let content: String = lines.collect::<Vec<_>>().join("\n").trim().to_string();
        (title, content)
    } else {
        (String::new(), text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compose_roundtrip() {
        let md = compose_chapter_markdown("第一章", "正文\n第二段");
        let (t, c) = parse_chapter_markdown(&md);
        assert_eq!(t, "第一章");
        assert_eq!(c, "正文\n第二段");
    }
}
