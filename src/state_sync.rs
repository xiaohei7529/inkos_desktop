//! 状态档案同步：解析 LLM 返回的 JSON 差异，安全地把变更写回 `<novel_root>/story_state/*.md`。
//!
//! 流程：
//! 1. [`parse_state_updates`] 把模型回包剥离 ```json``` 围栏后反序列化为 [`StateSyncReport`]。
//! 2. [`apply_updates`] 对每个出现在白名单中的文件：先把现有内容 snapshot 到
//!    `<novel_root>/状态档案备份/<ts>/<file>`，再按 `action` 写盘。
//! 3. 任何不在白名单内的文件、空内容、或 patch 找不到旧片段时都会被记录在 [`StateFileChange::note`]，
//!    主程序据此向用户与操作日志反馈。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateUpdate {
    pub file: String,
    /// "replace" | "patch"
    pub action: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StateSyncReport {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub updates: Vec<StateUpdate>,
}

#[derive(Debug, Clone)]
pub struct StateFileChange {
    pub file: String,
    pub old_chars: usize,
    pub new_chars: usize,
    pub action: String,
    pub backup_path: Option<PathBuf>,
    pub note: String,
}

impl StateFileChange {
    pub fn delta(&self) -> i64 {
        self.new_chars as i64 - self.old_chars as i64
    }
}

/// 状态档案备份目录：`<novel_root>/状态档案备份/<timestamp>/`
pub fn backup_root(novel_root: &Path) -> PathBuf {
    novel_root.join("状态档案备份")
}

/// 剥离常见的 ```json ... ``` 围栏 / 前后噪声后反序列化。
pub fn parse_state_updates(raw: &str) -> Result<StateSyncReport> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("LLM 返回为空"));
    }
    // 1) fenced ```json ... ``` 或 ``` ... ```
    if let Some(stripped) = strip_fence(trimmed) {
        if let Ok(r) = serde_json::from_str::<StateSyncReport>(stripped.trim()) {
            return Ok(r);
        }
    }
    // 2) 直接尝试整段 JSON
    if let Ok(r) = serde_json::from_str::<StateSyncReport>(trimmed) {
        return Ok(r);
    }
    // 3) 取首个 { 到最后一个 } 之间的子串
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if end > start {
            let slice = &trimmed[start..=end];
            if let Ok(r) = serde_json::from_str::<StateSyncReport>(slice) {
                return Ok(r);
            }
        }
    }
    Err(anyhow!("无法解析 LLM 返回为 StateSyncReport JSON"))
}

fn strip_fence(s: &str) -> Option<&str> {
    let s = s.trim();
    let body = s.strip_prefix("```json").or_else(|| s.strip_prefix("```"))?;
    body.strip_suffix("```").or(Some(body))
}

/// 应用一组 updates。`whitelist` 用于限制只允许写哪些文件名（如 STATE_FILES）。
pub fn apply_updates(
    novel_root: &Path,
    state_dir: &Path,
    report: &StateSyncReport,
    whitelist: &[&str],
) -> Vec<StateFileChange> {
    let mut out = Vec::new();
    if report.updates.is_empty() {
        return out;
    }
    let ts = Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let backup_dir = backup_root(novel_root).join(&ts);

    for upd in &report.updates {
        let file_name = upd.file.trim();
        let mut change = StateFileChange {
            file: file_name.to_string(),
            old_chars: 0,
            new_chars: 0,
            action: upd.action.trim().to_lowercase(),
            backup_path: None,
            note: String::new(),
        };
        if file_name.is_empty() {
            change.note = "跳过：空文件名".into();
            out.push(change);
            continue;
        }
        if !whitelist.iter().any(|w| *w == file_name) {
            change.note = "跳过：不在白名单".into();
            out.push(change);
            continue;
        }
        // 禁止路径穿越
        if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
            change.note = "跳过：非法文件名".into();
            out.push(change);
            continue;
        }

        let path = state_dir.join(file_name);
        let old = fs::read_to_string(&path).unwrap_or_default();
        change.old_chars = old.chars().count();

        // 备份旧版（仅当存在内容时）
        if !old.is_empty() {
            if let Err(e) = fs::create_dir_all(&backup_dir) {
                change.note = format!("备份失败：{e}");
                out.push(change);
                continue;
            }
            let bk = backup_dir.join(file_name);
            if let Err(e) = fs::write(&bk, &old) {
                change.note = format!("备份失败：{e}");
                out.push(change);
                continue;
            }
            change.backup_path = Some(bk);
        }

        let new_text = match change.action.as_str() {
            "replace" | "" => upd.content.clone(),
            "patch" => match apply_patch_blocks(&old, &upd.content) {
                Ok((s, n_applied, n_failed)) => {
                    if n_failed > 0 {
                        change.note = format!("patch 应用 {n_applied} 处，{n_failed} 处旧片段未找到（已追加到文件尾部）");
                    } else {
                        change.note = format!("patch 已应用 {n_applied} 处");
                    }
                    s
                }
                Err(e) => {
                    change.note = format!("patch 解析失败：{e}（已降级为 append）");
                    let mut merged = old.clone();
                    merged.push_str("\n\n<!-- LLM patch fallback -->\n");
                    merged.push_str(&upd.content);
                    merged
                }
            },
            other => {
                change.note = format!("未知 action {other}，已跳过");
                out.push(change);
                continue;
            }
        };

        if new_text == old {
            change.new_chars = change.old_chars;
            if change.note.is_empty() {
                change.note = "无实际变化".into();
            }
            out.push(change);
            continue;
        }

        if let Err(e) = fs::write(&path, &new_text) {
            change.note = format!("写盘失败：{e}");
            out.push(change);
            continue;
        }
        change.new_chars = new_text.chars().count();
        out.push(change);
    }
    out
}

/// patch 内容由若干块组成，每块格式：
/// `===REPLACE_BLOCK===\n旧片段\n===WITH===\n新片段\n===END===`
///
/// 返回 (新文本, 已应用块数, 未找到块数)。
fn apply_patch_blocks(old: &str, patch: &str) -> Result<(String, usize, usize)> {
    let mut text = old.to_string();
    let mut applied = 0usize;
    let mut failed = 0usize;
    let mut failed_blocks: Vec<String> = Vec::new();

    let mut rest = patch;
    while let Some(start) = rest.find("===REPLACE_BLOCK===") {
        rest = &rest[start + "===REPLACE_BLOCK===".len()..];
        let Some(mid) = rest.find("===WITH===") else {
            return Err(anyhow!("缺少 ===WITH=== 分隔"));
        };
        let old_part = rest[..mid].trim_matches('\n').trim_matches('\r');
        rest = &rest[mid + "===WITH===".len()..];
        let Some(end) = rest.find("===END===") else {
            return Err(anyhow!("缺少 ===END=== 分隔"));
        };
        let new_part = rest[..end].trim_matches('\n').trim_matches('\r');
        rest = &rest[end + "===END===".len()..];

        if old_part.is_empty() {
            failed += 1;
            failed_blocks.push(new_part.to_string());
            continue;
        }
        if let Some(pos) = text.find(old_part) {
            text.replace_range(pos..pos + old_part.len(), new_part);
            applied += 1;
        } else {
            failed += 1;
            failed_blocks.push(new_part.to_string());
        }
    }

    if !failed_blocks.is_empty() {
        text.push_str("\n\n<!-- LLM patch unmatched blocks (appended) -->\n");
        for b in &failed_blocks {
            text.push_str(b);
            text.push_str("\n\n");
        }
    }
    Ok((text, applied, failed))
}
