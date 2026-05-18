//! 状态档案同步：解析 LLM 返回的 JSON 差异，安全地把变更写回 `<novel_root>/story_state/*.md`。
//!
//! 流程：
//! 1. [`parse_state_updates`] 把模型回包剥离 ```json``` 围栏后反序列化为 [`StateSyncReport`]。
//! 2. [`apply_updates`] 对每个出现在白名单中的文件：先把现有内容 snapshot 到
//!    `<novel_root>/状态档案备份/<ts>/<file>`，再按 `action` 写盘。
//! 3. 任何不在白名单内的文件、空内容、或 patch 找不到旧片段时都会被记录在 [`StateFileChange::note`]，
//!    主程序据此向用户与操作日志反馈。

use std::borrow::Cow;
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

/// 审计 AI 返回的「可执行动作」列表对应的 JSON 根结构（参见 `ReviewAuditReport::actions`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewAction {
    pub title: String,
    /// `"text_rewrite"`（改写正文）或 `"state_sync"`（同步档案）等扩展类型。
    pub action_type: String,
    pub description: String,
    /// 具体修改数据或与后续流程相关的 JSON 负载。
    pub payload: serde_json::Value,
}

/// 审计 AI 返回的 JSON 根对象：一组待处理动作。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewAuditReport {
    #[serde(default)]
    pub actions: Vec<ReviewAction>,
}

const AUDIT_JSON_FENCE_MARKER: &str = "```json";

/// 末尾 ```json … ```（审计约定的机器可读区块）已从 Markdown 审稿正文中剪掉；若无闭合围栏则原样返回。
pub fn strip_audit_trailing_json_fence(raw: &str) -> Cow<'_, str> {
    let marker = AUDIT_JSON_FENCE_MARKER;
    let Some(open) = raw.rfind(marker) else {
        return Cow::Borrowed(raw);
    };
    let after_open = &raw[open + marker.len()..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);
    match after_open.find("```") {
        Some(_) => {
            let md = raw[..open].trim_end_matches([' ', '\t']);
            Cow::Owned(md.trim_end_matches(['\r', '\n']).to_string())
        }
        None => Cow::Borrowed(raw),
    }
}

/// 从全文尾部截取闭合的 ` ```json ` 围栏内 JSON 文本（供悬念审计等复用）。
pub fn trailing_json_fence_body(trimmed_full: &str) -> Option<&str> {
    audit_review_json_fence_body(trimmed_full)
}

fn audit_review_json_fence_body(trimmed_full: &str) -> Option<&str> {
    let marker = AUDIT_JSON_FENCE_MARKER;
    let open = trimmed_full.rfind(marker)?;
    let after_open = &trimmed_full[open + marker.len()..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);
    let close = after_open.find("```")?;
    Some(after_open[..close].trim())
}

/// 从审计 LLM 全文中截取尾部 ` ```json ` 围栏内的 JSON，解析为 [`ReviewAuditReport`]。
pub fn parse_review_audit_report(raw: &str) -> Result<ReviewAuditReport> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("LLM 返回为空"));
    }
    if let Some(body) = audit_review_json_fence_body(trimmed) {
        if let Ok(r) = serde_json::from_str::<ReviewAuditReport>(body) {
            return Ok(r);
        }
    }
    if let Ok(r) = serde_json::from_str::<ReviewAuditReport>(trimmed) {
        return Ok(r);
    }
    // 容错：首尾大括号截取
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if end >= start {
            let slice = &trimmed[start..=end];
            if let Ok(r) = serde_json::from_str::<ReviewAuditReport>(slice) {
                return Ok(r);
            }
        }
    }
    Err(anyhow!(
        "无法解析为 ReviewAuditReport（需在正文末尾输出闭合的 ```json 围栏）"
    ))
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

/// 第一章写作完成后，从 LLM 全文尾部 JSON 同步悬念至 `pending_hooks.md` 的结果。
#[derive(Debug, Clone)]
pub struct PrologueHooksApplyResult {
    pub summary: String,
    pub updates_applied: usize,
}

const PROLOGUE_HOOKS_WHITELIST: &[&str] = &["pending_hooks.md"];

/// 解析正文末尾 ` ```json ` 围栏中的 [`StateSyncReport`]，仅应用 `pending_hooks.md` 的 patch/replace。
pub fn apply_prologue_pending_hooks_from_raw(
    raw: &str,
    novel_root: &Path,
    state_dir: &Path,
) -> Result<Option<PrologueHooksApplyResult>> {
    let trimmed = raw.trim();
    let Some(json_body) = audit_review_json_fence_body(trimmed) else {
        return Ok(None);
    };
    let mut report: StateSyncReport = parse_state_updates(json_body)?;
    report.updates.retain(|u| {
        let file = u.file.trim();
        let action = u.action.trim().to_lowercase();
        file == "pending_hooks.md" && (action == "patch" || action == "replace" || action.is_empty())
    });
    if report.updates.is_empty() {
        return Ok(None);
    }
    let n = report.updates.len();
    let summary = report.summary.clone();
    let _changes = apply_updates(novel_root, state_dir, &report, PROLOGUE_HOOKS_WHITELIST);
    Ok(Some(PrologueHooksApplyResult {
        summary,
        updates_applied: n,
    }))
}

/// 将 [`AuditHooksReport`] 增量 patch 至 `pending_hooks.md`（第一章悬念提取子任务落盘）。
pub fn apply_audit_hooks_report(
    report: &crate::inkoswin_prompt::AuditHooksReport,
    novel_root: &Path,
    state_dir: &Path,
) -> Result<Option<PrologueHooksApplyResult>> {
    let markdown = crate::inkoswin_prompt::hooks_report_to_pending_hooks_markdown(report);
    if markdown.trim().is_empty() {
        return Ok(None);
    }
    let patch_content = format!(
        "===REPLACE_BLOCK===\n## 核心伏笔\n\n===WITH===\n## 核心伏笔\n\n{markdown}\n===END==="
    );
    let sync_report = StateSyncReport {
        summary: format!("悬念审计提取 {} 条钩子", report.hooks.len()),
        updates: vec![StateUpdate {
            file: "pending_hooks.md".to_string(),
            action: "patch".to_string(),
            content: patch_content,
        }],
    };
    let n = sync_report.updates.len();
    let summary = sync_report.summary.clone();
    let _ = apply_updates(novel_root, state_dir, &sync_report, PROLOGUE_HOOKS_WHITELIST);
    Ok(Some(PrologueHooksApplyResult {
        summary,
        updates_applied: n,
    }))
}

const CHAPTER_CONTEXT_WHITELIST: &[&str] = &["pending_hooks.md", "current_state.md"];

/// 后置摘要任务落盘结果。
#[derive(Debug, Clone)]
pub struct ChapterContextApplyResult {
    pub summary: String,
    pub hooks_applied: usize,
    pub state_files_touched: usize,
}

/// 应用 [`ChapterContextReport`]：更新章节元数据摘要、重建 `chapter_summaries.md`、同步伏笔与状态。
pub fn apply_chapter_context_report(
    report: &crate::inkoswin_prompt::ChapterContextReport,
    chapter_no: i32,
    project: &mut crate::project::NovelProject,
    store: &crate::project::ProjectStore,
    novel_root: &Path,
    state_dir: &Path,
) -> Result<ChapterContextApplyResult> {
    let summary = report.summary.trim().to_string();
    if !summary.is_empty() {
        if let Some(ch) = crate::project::ProjectStore::get_chapter_mut(project, chapter_no) {
            ch.summary = summary.clone();
        }
    }

    let summaries_body = store.build_chapter_summaries_document(project);
    store.write_story_state_file("chapter_summaries.md", &summaries_body)?;

    let mut updates: Vec<StateUpdate> = Vec::new();

    let hooks_md = crate::inkoswin_prompt::hooks_strings_to_pending_hooks_markdown(&report.hooks);
    if !hooks_md.trim().is_empty() {
        let patch_content = format!(
            "===REPLACE_BLOCK===\n## 核心伏笔\n\n===WITH===\n## 核心伏笔\n\n{hooks_md}\n===END==="
        );
        updates.push(StateUpdate {
            file: "pending_hooks.md".to_string(),
            action: "patch".to_string(),
            content: patch_content,
        });
    }

    let loc = report.state_updates.location.trim();
    let inv = report.state_updates.inventory.trim();
    if !loc.is_empty() || !inv.is_empty() {
        let mut block = format!("\n\n<!-- post-write ch {chapter_no} -->\n## 章末快照（第{chapter_no}章）\n");
        if !loc.is_empty() {
            block.push_str(&format!("- 地点：{loc}\n"));
        }
        if !inv.is_empty() {
            block.push_str(&format!("- 持物/资源：{inv}\n"));
        }
        updates.push(StateUpdate {
            file: "current_state.md".to_string(),
            action: "patch".to_string(),
            content: format!("===REPLACE_BLOCK===\n\n===WITH===\n{block}\n===END==="),
        });
    }

    let hooks_applied = if hooks_md.trim().is_empty() {
        0
    } else {
        report.hooks.len()
    };

    let mut state_files_touched = 1usize; // chapter_summaries always rebuilt
    if !updates.is_empty() {
        let sync_report = StateSyncReport {
            summary: if summary.is_empty() {
                format!("第 {chapter_no} 章后置摘要同步")
            } else {
                summary.chars().take(80).collect()
            },
            updates,
        };
        let changes = apply_updates(novel_root, state_dir, &sync_report, CHAPTER_CONTEXT_WHITELIST);
        state_files_touched = 1
            + changes
                .iter()
                .filter(|c| c.new_chars != c.old_chars)
                .count();
    }

    store.save_project(project)?;

    Ok(ChapterContextApplyResult {
        summary: if summary.is_empty() {
            report.summary.clone()
        } else {
            summary
        },
        hooks_applied,
        state_files_touched,
    })
}

#[cfg(test)]
mod prologue_hooks_tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_state_dir() -> (PathBuf, PathBuf) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let novel = std::env::temp_dir().join(format!(
            "inkos_prologue_hooks_{}_{}",
            std::process::id(),
            stamp
        ));
        let state = novel.join("story_state");
        fs::create_dir_all(&state).unwrap();
        fs::write(
            state.join("pending_hooks.md"),
            "# 未闭合伏笔\n\n## 核心伏笔\n",
        )
        .unwrap();
        (novel, state)
    }

    #[test]
    fn apply_prologue_pending_hooks_patches_file() {
        let (novel, state) = temp_state_dir();
        let raw = "标题：一\n摘要：s\n正文：\n正文。\n\n```json\n\
{\"summary\":\"发现新悬念\",\"updates\":[{\"file\":\"pending_hooks.md\",\"action\":\"patch\",\
\"content\":\"===REPLACE_BLOCK===\\n## 核心伏笔\\n\\n===WITH===\\n## 核心伏笔\\n\\n- [悬念1] 左眼金光\\n\\n===END===\"}]}\n```";
        let res = apply_prologue_pending_hooks_from_raw(raw, &novel, &state)
            .unwrap()
            .expect("should apply");
        assert_eq!(res.updates_applied, 1);
        let text = fs::read_to_string(state.join("pending_hooks.md")).unwrap();
        assert!(text.contains("左眼金光"));
        let _ = fs::remove_dir_all(&novel);
    }

    #[test]
    fn apply_audit_hooks_report_appends_entries() {
        let (novel, state) = temp_state_dir();
        let report = crate::inkoswin_prompt::AuditHooksReport {
            hooks: vec![crate::inkoswin_prompt::AuditHookEntry {
                source: "戒指微微发烫".into(),
                hook_type: "关键道具".into(),
                status: "active".into(),
                urgency: 4,
            }],
        };
        let res = apply_audit_hooks_report(&report, &novel, &state)
            .unwrap()
            .expect("applied");
        assert_eq!(res.updates_applied, 1);
        let text = fs::read_to_string(state.join("pending_hooks.md")).unwrap();
        assert!(text.contains("戒指微微发烫"));
        assert!(text.contains("[线索片段]"));
        let _ = fs::remove_dir_all(&novel);
    }

    #[test]
    fn apply_chapter_context_report_updates_summaries_and_hooks() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let novel = std::env::temp_dir().join(format!(
            "inkos_ch_ctx_{}_{}",
            std::process::id(),
            stamp
        ));
        let state = novel.join("story_state");
        fs::create_dir_all(&state).unwrap();
        fs::write(
            state.join("pending_hooks.md"),
            "# 未闭合伏笔\n\n## 核心伏笔\n",
        )
        .unwrap();
        fs::write(state.join("current_state.md"), "# 世界状态\n").unwrap();

        let store = crate::project::ProjectStore::new(novel.clone());
        let mut project = store.load_project().unwrap();
        store.ensure_chapter(&mut project, 1, "第一章").unwrap();
        store
            .save_chapter(
                &mut project,
                1,
                "第一章",
                "她推开门，雨还在下。",
                "generated",
                "旧摘要",
                &[],
            )
            .unwrap();

        let report = crate::inkoswin_prompt::ChapterContextReport {
            summary: "主角在雨中抵达旧站，左眼刺痛加剧。".into(),
            hooks: vec!["左眼金光未解".into()],
            state_updates: crate::inkoswin_prompt::ChapterStateUpdates {
                location: "城北旧站".into(),
                inventory: "破损罗盘".into(),
            },
        };
        let res = apply_chapter_context_report(
            &report,
            1,
            &mut project,
            &store,
            &novel,
            &state,
        )
        .unwrap();
        assert!(res.summary.contains("旧站"));
        assert_eq!(res.hooks_applied, 1);
        let ch = crate::project::ProjectStore::get_chapter(&project, 1).unwrap();
        assert!(ch.summary.contains("旧站"));
        let summaries = fs::read_to_string(state.join("chapter_summaries.md")).unwrap();
        assert!(summaries.contains("旧站"));
        let hooks = fs::read_to_string(state.join("pending_hooks.md")).unwrap();
        assert!(hooks.contains("左眼金光"));
        let current = fs::read_to_string(state.join("current_state.md")).unwrap();
        assert!(current.contains("城北旧站"));
        let _ = fs::remove_dir_all(&novel);
    }

    #[test]
    fn apply_prologue_skips_without_fence() {
        let (novel, state) = temp_state_dir();
        let raw = "标题：一\n正文：\n只有正文";
        assert!(
            apply_prologue_pending_hooks_from_raw(raw, &novel, &state)
                .unwrap()
                .is_none()
        );
        let _ = fs::remove_dir_all(&novel);
    }
}

#[cfg(test)]
mod review_audit_tests {
    use super::*;

    #[test]
    fn strip_audit_fence_removes_closed_block() {
        let raw = "### 审稿\n打分 8。\n```json\n{\"actions\":[]}\n```";
        match strip_audit_trailing_json_fence(raw) {
            Cow::Owned(s) => assert_eq!(s.trim(), "### 审稿\n打分 8。"),
            Cow::Borrowed(_) => panic!("expected owned strip"),
        }
    }

    #[test]
    fn parse_review_audit_report_from_fence() {
        let raw = "优点：好\n\n```json\n{\"actions\":[{\"title\":\"\",\"action_type\":\"state_sync\",\"description\":\"\",\"payload\":{\"file\":\"current_state.md\",\"action\":\"patch\",\"content\":\"\"}}]}\n```\n";
        let r = parse_review_audit_report(raw).unwrap();
        assert_eq!(r.actions.len(), 1);
        assert_eq!(r.actions[0].action_type, "state_sync");
    }
}
