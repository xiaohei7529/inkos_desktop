//! 操作日志：按日期写入 `<novel_root>/操作日志/YYYY-MM-DD.jsonl`。
//!
//! 用于后期回看「打开项目 / 保存章节 / AI 审计 / AI 改写 / 替换原文 / 定时写作 …」等关键事件。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpLogEntry {
    pub time: String,
    pub timestamp: String,
    pub kind: String,
    pub detail: String,
}

pub fn oplog_root(novel_root: &Path) -> PathBuf {
    novel_root.join("操作日志")
}

pub fn day_path(novel_root: &Path, date: NaiveDate) -> PathBuf {
    oplog_root(novel_root).join(format!("{}.jsonl", date.format("%Y-%m-%d")))
}

pub fn append(novel_root: &Path, kind: &str, detail: &str) -> Result<()> {
    let now = Local::now();
    fs::create_dir_all(oplog_root(novel_root))?;
    let path = day_path(novel_root, now.date_naive());
    let entry = OpLogEntry {
        time: now.format("%H:%M:%S").to_string(),
        timestamp: now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        kind: kind.to_string(),
        detail: detail.to_string(),
    };
    let line = serde_json::to_string(&entry)? + "\n";
    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// 不抛错版本，便于日志记录失败时不影响主流程。
pub fn try_append(novel_root: Option<&Path>, kind: &str, detail: &str) {
    if let Some(root) = novel_root {
        let _ = append(root, kind, detail);
    }
}

pub fn read_day(novel_root: &Path, date: NaiveDate) -> Vec<OpLogEntry> {
    let p = day_path(novel_root, date);
    let Ok(text) = fs::read_to_string(&p) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(e) = serde_json::from_str::<OpLogEntry>(line) {
            out.push(e);
        }
    }
    out
}

pub fn list_dates(novel_root: &Path) -> Vec<NaiveDate> {
    let dir = oplog_root(novel_root);
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Ok(d) = NaiveDate::parse_from_str(stem, "%Y-%m-%d") {
            out.push(d);
        }
    }
    out.sort();
    out.reverse();
    out
}
