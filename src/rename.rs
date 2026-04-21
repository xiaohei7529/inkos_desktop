//! 全书实体改名：扫描 `chapters/` 与 `story_state/`，将 `from` 文本字面量替换为 `to`。
//!
//! 灵感来源：Narcooo/inkos `inkos rename` 命令。
//!
//! 安全策略：
//! - 替换前自动把所有命中文件 snapshot 到 `状态档案备份/<ts>/<rel>`；
//! - 只做大小写敏感的字面量替换，不正则；
//! - 章节命中后会同时更新章节标题字段（如果标题里出现旧名）。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Local;

use crate::history;
use crate::project::{NovelProject, ProjectStore};
use crate::state_sync::backup_root;

#[derive(Debug, Clone, Default)]
pub struct RenameReport {
    pub from: String,
    pub to: String,
    pub backup_dir: Option<PathBuf>,
    pub items: Vec<RenameItem>,
}

#[derive(Debug, Clone)]
pub struct RenameItem {
    pub kind: RenameKind,
    pub label: String,
    pub occurrences: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameKind {
    Chapter,
    StateFile,
    StoryFile,
    ProjectMeta,
}

impl RenameKind {
    pub fn label(&self) -> &'static str {
        match self {
            RenameKind::Chapter => "章节",
            RenameKind::StateFile => "状态档案",
            RenameKind::StoryFile => "扩展层",
            RenameKind::ProjectMeta => "项目资料",
        }
    }
}

pub fn rename_entity(
    store: &ProjectStore,
    project: &mut NovelProject,
    from: &str,
    to: &str,
) -> Result<RenameReport> {
    let from = from.trim();
    let to = to.trim();
    if from.is_empty() {
        return Err(anyhow!("旧名称不能为空"));
    }
    if from == to {
        return Err(anyhow!("新旧名称相同"));
    }

    let ts = Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let bk_dir = backup_root(store.root()).join(format!("rename-{ts}"));

    let mut report = RenameReport {
        from: from.to_string(),
        to: to.to_string(),
        backup_dir: Some(bk_dir.clone()),
        items: Vec::new(),
    };

    // 1) 章节文件
    let mut renamed_any = false;
    let chapters_dir = store.chapters_dir();
    if chapters_dir.exists() {
        for entry in fs::read_dir(&chapters_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let raw = fs::read_to_string(&path).unwrap_or_default();
            let count = raw.matches(from).count();
            if count == 0 {
                continue;
            }
            let new_text = raw.replace(from, to);
            backup_file(&bk_dir, store.root(), &path, &raw)?;
            // 章节级历史快照
            if let Some(num) = number_from_path(&path) {
                let _ = history::snapshot_chapter(
                    store.root(),
                    num,
                    &project
                        .chapters
                        .iter()
                        .find(|c| c.number == num)
                        .map(|c| c.title.clone())
                        .unwrap_or_default(),
                    &raw,
                    &new_text,
                    "rename",
                    &format!("rename `{from}` → `{to}`"),
                );
            }
            fs::write(&path, &new_text).with_context(|| format!("write {:?}", path))?;
            report.items.push(RenameItem {
                kind: RenameKind::Chapter,
                label: path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string(),
                occurrences: count,
            });
            renamed_any = true;
        }
    }

    // 2) 状态档案
    let state_dir = store.state_dir();
    if state_dir.exists() {
        for entry in fs::read_dir(&state_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let raw = fs::read_to_string(&path).unwrap_or_default();
            let count = raw.matches(from).count();
            if count == 0 {
                continue;
            }
            let new_text = raw.replace(from, to);
            backup_file(&bk_dir, store.root(), &path, &raw)?;
            fs::write(&path, &new_text)?;
            report.items.push(RenameItem {
                kind: RenameKind::StateFile,
                label: path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string(),
                occurrences: count,
            });
            renamed_any = true;
        }
    }

    // 3) story 扩展目录（intent / focus / fingerprint 等）
    let story_dir = store.root().join("story");
    if story_dir.exists() {
        for entry in walkdir::WalkDir::new(&story_dir).max_depth(3) {
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
                Some("md") | Some("json") | Some("yaml") | Some("yml")
            );
            if !ext_ok {
                continue;
            }
            let raw = fs::read_to_string(path).unwrap_or_default();
            let count = raw.matches(from).count();
            if count == 0 {
                continue;
            }
            let new_text = raw.replace(from, to);
            backup_file(&bk_dir, store.root(), path, &raw)?;
            fs::write(path, &new_text)?;
            let rel = path
                .strip_prefix(store.root())
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            report.items.push(RenameItem {
                kind: RenameKind::StoryFile,
                label: rel,
                occurrences: count,
            });
            renamed_any = true;
        }
    }

    // 4) project.json 中的字段
    let mut meta_count = 0usize;
    let strs: Vec<&mut String> = vec![
        &mut project.title,
        &mut project.premise,
        &mut project.protagonists,
        &mut project.world_setting,
        &mut project.writing_style,
        &mut project.outline,
        &mut project.extra_guidance,
    ];
    for s in strs {
        meta_count += s.matches(from).count();
        *s = s.replace(from, to);
    }
    for ch in &mut project.chapters {
        meta_count += ch.title.matches(from).count();
        ch.title = ch.title.replace(from, to);
        meta_count += ch.summary.matches(from).count();
        ch.summary = ch.summary.replace(from, to);
    }
    if meta_count > 0 {
        let _ = store.save_project(project);
        report.items.push(RenameItem {
            kind: RenameKind::ProjectMeta,
            label: "project.json".to_string(),
            occurrences: meta_count,
        });
        renamed_any = true;
    }

    if !renamed_any {
        report.backup_dir = None;
    }
    Ok(report)
}

fn backup_file(bk_dir: &Path, novel_root: &Path, path: &Path, raw: &str) -> Result<()> {
    let rel = path.strip_prefix(novel_root).unwrap_or(path);
    let target = bk_dir.join(rel);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&target, raw).with_context(|| format!("backup {:?}", target))?;
    Ok(())
}

fn number_from_path(path: &Path) -> Option<i32> {
    let stem = path.file_stem()?.to_str()?;
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}
