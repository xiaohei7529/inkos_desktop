//! 从单个文本文件按「第X章」（或自定义分隔正则）拆分章节，增量导入到 `chapters/NNN.md`。
//!
//! 灵感来源：Narcooo/inkos `inkos import chapters` 命令。
//!
//! - 默认正则：`^\s*第\s*[一二三四五六七八九十百千零〇0-9]+\s*章\s*[^\n]*$`
//! - 已存在的章节文件默认跳过（断点续导）；可通过 `overwrite=true` 强制覆盖。
//! - 拆分顺序按文本中出现的先后；若无法匹配任意标题，则整个文件作为「第 1 章」。

use anyhow::{Context, Result};
use regex::Regex;
use std::fs;

use crate::chapter_md::compose_chapter_markdown;
use crate::project::{NovelProject, ProjectStore};

#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub source_label: String,
    pub split_regex: String,
    pub starting_number: i32,
    pub overwrite: bool,
}

impl Default for ImportPlan {
    fn default() -> Self {
        Self {
            source_label: String::new(),
            split_regex: default_split_regex().to_string(),
            starting_number: 1,
            overwrite: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportItem {
    pub number: i32,
    pub title: String,
    pub content: String,
    pub status: ImportStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportStatus {
    Created,
    Skipped,
    Overwritten,
    Empty,
}

impl ImportStatus {
    pub fn label(&self) -> &'static str {
        match self {
            ImportStatus::Created => "新建",
            ImportStatus::Overwritten => "覆盖",
            ImportStatus::Skipped => "已存在 · 跳过",
            ImportStatus::Empty => "正文为空 · 跳过",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub items: Vec<ImportItem>,
    pub created: usize,
    pub skipped: usize,
    pub overwritten: usize,
    pub empty: usize,
}

pub fn default_split_regex() -> &'static str {
    r"(?m)^\s*第\s*[一二三四五六七八九十百千零〇0-9]+\s*章(?:\s+|[：:、\-—]|$)[^\n]*$"
}

/// 仅做拆分，不写盘。返回 (标题, 正文) 列表。
pub fn split_text(raw: &str, regex: &str) -> Result<Vec<(String, String)>> {
    let re = Regex::new(regex).with_context(|| format!("非法正则：{regex}"))?;
    let text = raw.trim_start_matches('\u{feff}');
    let matches: Vec<_> = re.find_iter(text).collect();
    if matches.is_empty() {
        let body = text.trim();
        if body.is_empty() {
            return Ok(Vec::new());
        }
        return Ok(vec![("第1章".to_string(), body.to_string())]);
    }

    let mut out = Vec::new();
    for (i, m) in matches.iter().enumerate() {
        let title = m.as_str().trim().to_string();
        let body_start = m.end();
        let body_end = matches.get(i + 1).map(|n| n.start()).unwrap_or(text.len());
        let body = text[body_start..body_end].trim().to_string();
        out.push((title, body));
    }
    Ok(out)
}

/// 完整导入：按拆分结果增量写入 `chapters/NNN.md` 并把项目元数据保存。
pub fn import_text(
    store: &ProjectStore,
    project: &mut NovelProject,
    raw: &str,
    plan: &ImportPlan,
) -> Result<ImportReport> {
    let segments = split_text(raw, &plan.split_regex)?;
    let mut report = ImportReport::default();

    let mut number = plan.starting_number.max(1);
    fs::create_dir_all(store.chapters_dir())?;

    for (title, body) in segments {
        let body_trim = body.trim();
        if body_trim.is_empty() {
            report.empty += 1;
            report.items.push(ImportItem {
                number,
                title,
                content: String::new(),
                status: ImportStatus::Empty,
            });
            number += 1;
            continue;
        }

        let path = store.chapter_file(number);
        if path.exists() && !plan.overwrite {
            report.skipped += 1;
            report.items.push(ImportItem {
                number,
                title,
                content: String::new(),
                status: ImportStatus::Skipped,
            });
            number += 1;
            continue;
        }

        // 优先用文本里的章节行作为标题，fallback 到「第N章」
        let clean_title = if title.trim().is_empty() {
            format!("第{number}章")
        } else {
            // 把「第十二章 风暴前夜」截短为标题，保留全文
            title.trim().to_string()
        };

        let md = compose_chapter_markdown(&clean_title, body_trim);
        fs::write(&path, &md).with_context(|| format!("write {:?}", path))?;

        let status = if path.exists() && plan.overwrite {
            ImportStatus::Overwritten
        } else {
            ImportStatus::Created
        };
        match status {
            ImportStatus::Overwritten => report.overwritten += 1,
            _ => report.created += 1,
        }
        report.items.push(ImportItem {
            number,
            title: clean_title,
            content: body_trim.to_string(),
            status,
        });
        number += 1;
    }

    let _ = store.save_project(project);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_basic() {
        let raw = "第一章 序幕\n这是第一章正文\n\n第二章 风暴\n第二章正文";
        let segs = split_text(raw, default_split_regex()).unwrap();
        assert_eq!(segs.len(), 2);
        assert!(segs[0].1.contains("第一章正文"));
        assert!(segs[1].0.contains("第二章"));
        assert!(segs[1].1.contains("第二章正文"));
    }

    #[test]
    fn split_title_with_punctuation() {
        let raw = "第一章：序幕\n正文\n\n第二章-风暴\n正文";
        let segs = split_text(raw, default_split_regex()).unwrap();
        assert_eq!(segs.len(), 2);
        assert!(segs[0].0.contains("序幕"));
        assert!(segs[1].0.contains("风暴"));
    }

    #[test]
    fn split_no_match() {
        let raw = "纯散文，没有标题";
        let segs = split_text(raw, default_split_regex()).unwrap();
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].0, "第1章");
    }
}
