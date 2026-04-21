//! 章节版本历史：每次「替换原文 / 手动覆盖」前先把旧版本快照写入磁盘，并记录修订日志。
//!
//! 目录结构（位于 `<novel_root>/章节历史/第NNN章/`）：
//! - `<timestamp>.md`            备份文件（含 frontmatter）
//! - `audit-<timestamp>.md`      自动审计产物（链式工作流写入）
//! - `log.jsonl`                 修订日志（每行一条 `ChapterRevision`）

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterRevision {
    pub timestamp: String,
    pub source: String,
    pub note: String,
    pub backup_file: String,
    pub old_chars: usize,
    pub new_chars: usize,
}

pub fn history_root(novel_root: &Path) -> PathBuf {
    novel_root.join("章节历史")
}

pub fn chapter_history_dir(novel_root: &Path, n: i32) -> PathBuf {
    history_root(novel_root).join(format!("第{n:03}章"))
}

pub fn snapshot_chapter(
    novel_root: &Path,
    n: i32,
    title: &str,
    old_body: &str,
    new_body: &str,
    source: &str,
    note: &str,
) -> Result<ChapterRevision> {
    let dir = chapter_history_dir(novel_root, n);
    fs::create_dir_all(&dir).with_context(|| format!("create_dir_all {:?}", dir))?;

    let now = chrono::Local::now();
    let ts = now.format("%Y%m%d-%H%M%S").to_string();
    let backup_file = format!("{ts}.md");
    let backup_path = dir.join(&backup_file);

    let header = format!(
        "---\n章节: 第{n}章 {title}\n备份时间: {}\n来源: {source}\n备注: {note}\n---\n\n# {title}\n\n",
        now.format("%Y-%m-%d %H:%M:%S")
    );
    fs::write(&backup_path, format!("{header}{old_body}\n"))
        .with_context(|| format!("write {:?}", backup_path))?;

    let rev = ChapterRevision {
        timestamp: now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        source: source.to_string(),
        note: note.to_string(),
        backup_file,
        old_chars: old_body.chars().filter(|c| !c.is_whitespace()).count(),
        new_chars: new_body.chars().filter(|c| !c.is_whitespace()).count(),
    };

    let log_path = dir.join("log.jsonl");
    let line = serde_json::to_string(&rev)? + "\n";
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open log {:?}", log_path))?;
    f.write_all(line.as_bytes())?;
    Ok(rev)
}

pub fn list_chapter_revisions(novel_root: &Path, n: i32) -> Vec<ChapterRevision> {
    let log = chapter_history_dir(novel_root, n).join("log.jsonl");
    let Ok(text) = fs::read_to_string(&log) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(r) = serde_json::from_str::<ChapterRevision>(line) {
            out.push(r);
        }
    }
    out
}

pub fn read_revision(novel_root: &Path, n: i32, file: &str) -> Result<String> {
    let path = chapter_history_dir(novel_root, n).join(file);
    Ok(fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?)
}

/// 去掉 frontmatter，只保留正文（标题之后的内容）。
pub fn revision_body_only(raw: &str) -> String {
    let text = raw.trim_start_matches('\u{feff}');
    let body = if let Some(rest) = text.strip_prefix("---") {
        if let Some(idx) = rest.find("\n---") {
            let after = &rest[idx + 4..];
            after.trim_start_matches('\n')
        } else {
            text
        }
    } else {
        text
    };
    let lines: Vec<&str> = body.lines().collect();
    let mut start = 0;
    if let Some(first) = lines.first() {
        if first.starts_with('#') {
            start = 1;
            while start < lines.len() && lines[start].trim().is_empty() {
                start += 1;
            }
        }
    }
    lines[start..].join("\n").trim().to_string()
}

/// 读取第 `n` 章最近一次审计 markdown 正文（去掉 header）。
pub fn latest_audit_text(novel_root: &Path, n: i32) -> Option<String> {
    let dir = chapter_history_dir(novel_root, n);
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else { return None };
    for e in entries.flatten() {
        let p = e.path();
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else { continue };
        if !(name.starts_with("audit-") && name.ends_with(".md")) {
            continue;
        }
        let mtime = e.metadata().and_then(|m| m.modified()).ok()?;
        candidates.push((mtime, p));
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let (_, path) = candidates.into_iter().next()?;
    let raw = fs::read_to_string(&path).ok()?;
    Some(strip_audit_header(&raw))
}

fn strip_audit_header(raw: &str) -> String {
    if let Some(idx) = raw.find("\n---\n") {
        raw[idx + 5..].trim().to_string()
    } else {
        raw.trim().to_string()
    }
}

pub fn save_audit_artifact(novel_root: &Path, n: i32, content: &str) -> Result<PathBuf> {
    let dir = chapter_history_dir(novel_root, n);
    fs::create_dir_all(&dir)?;
    let now = chrono::Local::now();
    let ts = now.format("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(format!("audit-{ts}.md"));
    let header = format!(
        "# 第{n}章 自动审计\n\n时间：{}\n\n---\n\n",
        now.format("%Y-%m-%d %H:%M:%S")
    );
    fs::write(&path, format!("{header}{content}"))?;
    Ok(path)
}
