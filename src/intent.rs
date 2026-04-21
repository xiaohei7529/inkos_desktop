//! 作者意图（author_intent.md）与当前焦点（current_focus.md）模板与读写。
//!
//! 灵感来源：Narcooo/inkos 的「Input Governance」层——把作者长期目标与近期 1-3 章的关注点
//! 结构化进入写作管线，避免 LLM 漂移。
//!
//! 落到 `<novel_root>/story/` 目录。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const AUTHOR_INTENT_TEMPLATE: &str = include_str!("../assets/intent/author_intent.md");
pub const CURRENT_FOCUS_TEMPLATE: &str = include_str!("../assets/intent/current_focus.md");

pub fn story_dir(novel_root: &Path) -> PathBuf {
    novel_root.join("story")
}

pub fn author_intent_path(novel_root: &Path) -> PathBuf {
    story_dir(novel_root).join("author_intent.md")
}

pub fn current_focus_path(novel_root: &Path) -> PathBuf {
    story_dir(novel_root).join("current_focus.md")
}

/// 项目首次进入新版时，若 `story/` 不存在则创建并写入两个默认模板。
/// 永远不会覆盖已有文件。
pub fn ensure_default_templates(novel_root: &Path) -> Result<()> {
    fs::create_dir_all(story_dir(novel_root))
        .with_context(|| format!("create_dir_all {:?}", story_dir(novel_root)))?;

    let intent = author_intent_path(novel_root);
    if !intent.exists() {
        fs::write(&intent, AUTHOR_INTENT_TEMPLATE)
            .with_context(|| format!("write {:?}", intent))?;
    }
    let focus = current_focus_path(novel_root);
    if !focus.exists() {
        fs::write(&focus, CURRENT_FOCUS_TEMPLATE)
            .with_context(|| format!("write {:?}", focus))?;
    }
    Ok(())
}

pub fn read_author_intent(novel_root: &Path) -> String {
    fs::read_to_string(author_intent_path(novel_root)).unwrap_or_default()
}

pub fn read_current_focus(novel_root: &Path) -> String {
    fs::read_to_string(current_focus_path(novel_root)).unwrap_or_default()
}

pub fn write_author_intent(novel_root: &Path, body: &str) -> Result<()> {
    fs::create_dir_all(story_dir(novel_root))?;
    let body = body.trim_end().to_string() + "\n";
    fs::write(author_intent_path(novel_root), body)?;
    Ok(())
}

pub fn write_current_focus(novel_root: &Path, body: &str) -> Result<()> {
    fs::create_dir_all(story_dir(novel_root))?;
    let body = body.trim_end().to_string() + "\n";
    fs::write(current_focus_path(novel_root), body)?;
    Ok(())
}

/// 拼装一段可注入到管线 prompt 的「Input Governance」上下文：
/// - 作者长期意图（截断）
/// - 近期 1-3 章关注点（截断）
pub fn governance_prompt(novel_root: &Path, max_each: usize) -> String {
    let mut out = String::new();
    let intent = read_author_intent(novel_root);
    let focus = read_current_focus(novel_root);

    let trim = |s: &str| -> String {
        let t = s.trim();
        if t.chars().count() <= max_each {
            t.to_string()
        } else {
            let cut: String = t.chars().take(max_each).collect();
            format!("{cut}\n…（已截断）")
        }
    };

    if !intent.trim().is_empty() {
        out.push_str("### 作者长期意图（author_intent.md）\n");
        out.push_str(&trim(&intent));
        out.push_str("\n\n");
    }
    if !focus.trim().is_empty() {
        out.push_str("### 当前焦点（current_focus.md）\n");
        out.push_str(&trim(&focus));
        out.push_str("\n\n");
    }
    out
}
