//! 审计/改写历史记录：按小说写入 `<novel_root>/审计记录/log.jsonl`。
//!
//! 每行一条 [`AuditRecord`]，记录时间、章节、模式（审计/改写）、服务商、模型、字数、用时与完整内容。
//! 用于在审核页随时回看以前生成的审计意见或 AI 改写版本，并支持一键载入到当前结果区。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Local;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub timestamp: String,
    pub time: String,
    pub date: String,
    /// "audit" | "rewrite"
    pub mode: String,
    pub chapter_no: i32,
    pub vendor_id: String,
    pub vendor_label: String,
    pub model: String,
    pub chars: usize,
    pub elapsed_secs: f64,
    pub content: String,
}

impl AuditRecord {
    pub fn mode_label(&self) -> &'static str {
        match self.mode.as_str() {
            "audit" => "审计",
            "rewrite" => "改写",
            _ => "其他",
        }
    }
}

pub fn audit_root(novel_root: &Path) -> PathBuf {
    novel_root.join("审计记录")
}

pub fn log_path(novel_root: &Path) -> PathBuf {
    audit_root(novel_root).join("log.jsonl")
}

/// 追加一条记录到 jsonl 日志，并返回完整记录（含时间戳字段）。
#[allow(clippy::too_many_arguments)]
pub fn append(
    novel_root: &Path,
    mode: &str,
    chapter_no: i32,
    vendor_id: &str,
    vendor_label: &str,
    model: &str,
    elapsed_secs: f64,
    content: &str,
) -> Result<AuditRecord> {
    let now = Local::now();
    fs::create_dir_all(audit_root(novel_root))?;
    let rec = AuditRecord {
        timestamp: now.format("%Y-%m-%dT%H:%M:%S").to_string(),
        time: now.format("%H:%M:%S").to_string(),
        date: now.format("%Y-%m-%d").to_string(),
        mode: mode.to_string(),
        chapter_no,
        vendor_id: vendor_id.to_string(),
        vendor_label: vendor_label.to_string(),
        model: model.to_string(),
        chars: content.chars().count(),
        elapsed_secs,
        content: content.to_string(),
    };
    let line = serde_json::to_string(&rec)? + "\n";
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(novel_root))?;
    f.write_all(line.as_bytes())?;
    Ok(rec)
}

/// 读取所有记录，按时间倒序（最新在前）。
pub fn list_all(novel_root: &Path) -> Vec<AuditRecord> {
    let path = log_path(novel_root);
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut out: Vec<AuditRecord> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<AuditRecord>(l).ok())
        .collect();
    out.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    out
}
