//! 文风指纹（style fingerprint）：
//!
//! - 启发式分析任意中文文本，输出 `StyleFingerprint`（句长、节奏、词频、对白比例等）；
//! - 落到 `<novel_root>/story/style_fingerprint.md`（人类可读的总结）；
//! - 提供 `imitation_prompt(...)` 把指纹注入到 draft / revise prompt。
//!
//! 灵感来源：Narcooo/inkos `inkos style import` / `inkos style apply`。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StyleFingerprint {
    pub source_label: String,
    pub total_chars: usize,
    pub total_sentences: usize,
    pub avg_sentence_chars: f32,
    pub max_sentence_chars: usize,
    pub min_sentence_chars: usize,
    pub dialogue_ratio: f32,
    pub long_sentence_ratio: f32,  // 句长 > 30 字的比例
    pub short_sentence_ratio: f32, // 句长 < 8 字的比例
    pub punctuation: PunctuationStats,
    pub top_terms: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PunctuationStats {
    pub commas: usize,
    pub periods: usize,
    pub questions: usize,
    pub exclamations: usize,
    pub dashes: usize,
    pub ellipses: usize,
}

pub fn fingerprint_path(novel_root: &Path) -> PathBuf {
    novel_root.join("story").join("style_fingerprint.md")
}

pub fn fingerprint_json_path(novel_root: &Path) -> PathBuf {
    novel_root.join("story").join("style_fingerprint.json")
}

pub fn analyze(text: &str, source_label: &str) -> StyleFingerprint {
    let cleaned: String = text.chars().filter(|c| !c.is_control()).collect();
    let total_chars = cleaned.chars().count();

    let mut sentences = Vec::new();
    let mut buf = String::new();
    for c in cleaned.chars() {
        buf.push(c);
        if matches!(c, '。' | '！' | '？' | '!' | '?' | '.' | '…') {
            let s = buf.trim().to_string();
            if !s.is_empty() {
                sentences.push(s);
            }
            buf.clear();
        }
    }
    if !buf.trim().is_empty() {
        sentences.push(buf.trim().to_string());
    }

    let total_sentences = sentences.len().max(1);
    let lens: Vec<usize> = sentences.iter().map(|s| s.chars().count()).collect();
    let avg = if !lens.is_empty() {
        lens.iter().sum::<usize>() as f32 / lens.len() as f32
    } else {
        0.0
    };
    let max = lens.iter().copied().max().unwrap_or(0);
    let min = lens.iter().copied().min().unwrap_or(0);
    let long = lens.iter().filter(|n| **n > 30).count() as f32 / total_sentences as f32;
    let short = lens.iter().filter(|n| **n < 8).count() as f32 / total_sentences as f32;

    let dialogue_lines = cleaned
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with('"')
                || t.starts_with('"')
                || t.starts_with('「')
                || t.starts_with('"')
        })
        .count();
    let total_lines = cleaned.lines().count().max(1);
    let dialogue_ratio = dialogue_lines as f32 / total_lines as f32;

    let punctuation = PunctuationStats {
        commas: cleaned.matches('，').count() + cleaned.matches(',').count(),
        periods: cleaned.matches('。').count() + cleaned.matches('.').count(),
        questions: cleaned.matches('？').count() + cleaned.matches('?').count(),
        exclamations: cleaned.matches('！').count() + cleaned.matches('!').count(),
        dashes: cleaned.matches("——").count(),
        ellipses: cleaned.matches("……").count() + cleaned.matches("…").count(),
    };

    let top_terms = top_bigrams(&cleaned, 20);

    StyleFingerprint {
        source_label: source_label.to_string(),
        total_chars,
        total_sentences,
        avg_sentence_chars: avg,
        max_sentence_chars: max,
        min_sentence_chars: min,
        dialogue_ratio,
        long_sentence_ratio: long,
        short_sentence_ratio: short,
        punctuation,
        top_terms,
    }
}

fn top_bigrams(text: &str, k: usize) -> Vec<(String, usize)> {
    let chars: Vec<char> = text
        .chars()
        .filter(|c| {
            !c.is_whitespace()
                && !c.is_ascii_punctuation()
                && !matches!(
                    *c,
                    '，' | '。'
                        | '！'
                        | '？'
                        | '；'
                        | '：'
                        | '"'
                        | '"'
                        | '「'
                        | '」'
                        | '（'
                        | '）'
                        | '《'
                        | '》'
                        | '、'
                )
        })
        .collect();
    let mut count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for w in chars.windows(2) {
        if w[0].is_ascii_digit() || w[1].is_ascii_digit() {
            continue;
        }
        let s: String = w.iter().collect();
        *count.entry(s).or_insert(0) += 1;
    }
    let mut v: Vec<_> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.into_iter().take(k).collect()
}

pub fn save(novel_root: &Path, fp: &StyleFingerprint) -> Result<()> {
    fs::create_dir_all(novel_root.join("story"))?;
    let json = serde_json::to_string_pretty(fp)?;
    fs::write(fingerprint_json_path(novel_root), json)?;
    fs::write(fingerprint_path(novel_root), to_markdown(fp))
        .with_context(|| format!("write {:?}", fingerprint_path(novel_root)))?;
    Ok(())
}

pub fn load(novel_root: &Path) -> Option<StyleFingerprint> {
    let raw = fs::read_to_string(fingerprint_json_path(novel_root)).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn read_markdown(novel_root: &Path) -> String {
    fs::read_to_string(fingerprint_path(novel_root)).unwrap_or_default()
}

fn to_markdown(fp: &StyleFingerprint) -> String {
    let mut out = String::new();
    out.push_str("# 文风指纹（Style Fingerprint）\n\n");
    out.push_str("> 由 InkOS Desktop 自动分析。可手动编辑下方备注，但请保留 JSON 兼容字段。\n\n");
    out.push_str(&format!("- 样本：{}\n", fp.source_label));
    out.push_str(&format!("- 总字数：{}\n", fp.total_chars));
    out.push_str(&format!("- 句子数：{}\n", fp.total_sentences));
    out.push_str(&format!("- 平均句长：{:.1}\n", fp.avg_sentence_chars));
    out.push_str(&format!("- 句长范围：[{}, {}]\n", fp.min_sentence_chars, fp.max_sentence_chars));
    out.push_str(&format!("- 长句比例（>30 字）：{:.1}%\n", fp.long_sentence_ratio * 100.0));
    out.push_str(&format!("- 短句比例（<8 字）：{:.1}%\n", fp.short_sentence_ratio * 100.0));
    out.push_str(&format!("- 对白行占比：{:.1}%\n", fp.dialogue_ratio * 100.0));
    out.push_str("\n## 标点节奏\n");
    out.push_str(&format!(
        "- 逗号 {}，句号 {}，问号 {}，感叹号 {}，破折号 {}，省略号 {}\n",
        fp.punctuation.commas,
        fp.punctuation.periods,
        fp.punctuation.questions,
        fp.punctuation.exclamations,
        fp.punctuation.dashes,
        fp.punctuation.ellipses
    ));
    out.push_str("\n## 高频二元短语\n");
    for (t, n) in &fp.top_terms {
        out.push_str(&format!("- `{t}` × {n}\n"));
    }
    out.push_str("\n## 模仿要点（手动补充）\n- \n");
    out
}

/// 把指纹拼成 prompt 注入。可在 draft / revise 前注入。
pub fn imitation_prompt(novel_root: &Path) -> String {
    let Some(fp) = load(novel_root) else {
        return String::new();
    };
    let mut out = String::new();
    out.push_str("### 文风模仿要点（来自 style_fingerprint）\n");
    out.push_str(&format!(
        "- 平均句长 {:.1} 字；长句占比 {:.0}%；短句占比 {:.0}%；力求保持类似节奏。\n",
        fp.avg_sentence_chars,
        fp.long_sentence_ratio * 100.0,
        fp.short_sentence_ratio * 100.0
    ));
    out.push_str(&format!(
        "- 对白行占比约 {:.0}%；维持相近对白密度。\n",
        fp.dialogue_ratio * 100.0
    ));
    out.push_str(&format!(
        "- 标点偏好：逗号 {}、句号 {}、破折号 {}、省略号 {}。\n",
        fp.punctuation.commas,
        fp.punctuation.periods,
        fp.punctuation.dashes,
        fp.punctuation.ellipses
    ));
    if !fp.top_terms.is_empty() {
        let words: Vec<String> = fp.top_terms.iter().take(8).map(|(t, _)| t.clone()).collect();
        out.push_str(&format!("- 可适度使用的高频词：{}\n", words.join("、")));
    }
    out
}
