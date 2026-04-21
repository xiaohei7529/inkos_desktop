//! 全书搜索：跨 `chapters/` + `story_state/` + `story/` 子目录。
//!
//! 灵感来源：Narcooo/inkos 的 `inkos search` 命令与 Studio 顶部的全书搜索。
//!
//! - 默认大小写敏感；可设置 `case_insensitive=true` 转小写后匹配。
//! - 每个命中返回上下文（命中行 + 前后 1 行）。

use std::fs;
use std::path::Path;

use crate::project::ProjectStore;

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub source: SearchSource,
    pub label: String,
    pub line_no: usize,
    pub line: String,
    pub context_before: Option<String>,
    pub context_after: Option<String>,
    /// 章节号（仅对章节命中有效）。
    pub chapter_no: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchSource {
    Chapter,
    StateFile,
    Story,
}

impl SearchSource {
    pub fn label(&self) -> &'static str {
        match self {
            SearchSource::Chapter => "章节",
            SearchSource::StateFile => "状态档案",
            SearchSource::Story => "扩展层",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub case_insensitive: bool,
    pub max_hits: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            query: String::new(),
            case_insensitive: false,
            max_hits: 500,
        }
    }
}

pub fn search(store: &ProjectStore, opts: &SearchOptions) -> Vec<SearchHit> {
    let q = opts.query.trim();
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }

    // 章节
    if store.chapters_dir().exists() {
        let mut entries: Vec<_> = fs::read_dir(store.chapters_dir())
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let raw = fs::read_to_string(&path).unwrap_or_default();
            let label = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let chapter_no = number_from_path(&path);
            scan_text(
                &raw,
                q,
                opts,
                SearchSource::Chapter,
                &label,
                chapter_no,
                &mut out,
            );
            if out.len() >= opts.max_hits {
                return out;
            }
        }
    }

    // 状态档案
    if store.state_dir().exists() {
        for entry in fs::read_dir(store.state_dir()).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let raw = fs::read_to_string(&path).unwrap_or_default();
            let label = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            scan_text(&raw, q, opts, SearchSource::StateFile, &label, None, &mut out);
            if out.len() >= opts.max_hits {
                return out;
            }
        }
    }

    // story 扩展层
    let story = store.root().join("story");
    if story.exists() {
        for entry in walkdir::WalkDir::new(&story).max_depth(3) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let ext_ok = matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("md") | Some("json") | Some("yaml") | Some("yml") | Some("txt")
            );
            if !ext_ok {
                continue;
            }
            let raw = fs::read_to_string(path).unwrap_or_default();
            let rel = path
                .strip_prefix(store.root())
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            scan_text(&raw, q, opts, SearchSource::Story, &rel, None, &mut out);
            if out.len() >= opts.max_hits {
                return out;
            }
        }
    }
    out
}

fn scan_text(
    text: &str,
    needle: &str,
    opts: &SearchOptions,
    source: SearchSource,
    label: &str,
    chapter_no: Option<i32>,
    out: &mut Vec<SearchHit>,
) {
    let lines: Vec<&str> = text.lines().collect();
    let needle_cmp = if opts.case_insensitive {
        needle.to_lowercase()
    } else {
        needle.to_string()
    };
    for (i, line) in lines.iter().enumerate() {
        let hay = if opts.case_insensitive {
            line.to_lowercase()
        } else {
            line.to_string()
        };
        if hay.contains(&needle_cmp) {
            out.push(SearchHit {
                source: source.clone(),
                label: label.to_string(),
                line_no: i + 1,
                line: line.to_string(),
                context_before: i.checked_sub(1).map(|j| lines[j].to_string()),
                context_after: lines.get(i + 1).map(|l| l.to_string()),
                chapter_no,
            });
            if out.len() >= opts.max_hits {
                return;
            }
        }
    }
}

fn number_from_path(path: &Path) -> Option<i32> {
    let stem = path.file_stem()?.to_str()?;
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}
