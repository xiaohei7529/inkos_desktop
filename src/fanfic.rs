//! 同人书向导 + 连续批量写 N 章规划。
//!
//! 灵感来源：Narcooo/inkos `inkos fanfic` 命令族。
//!
//! 提供：
//! - 由原作（参考文本）抽取角色/世界观/口头禅，作为同人书 brief 的填空草稿；
//! - 把同人 brief 落到 `story/fanfic_brief.md`；
//! - 计算「连续批写 N 章」时的批次拆分策略。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FanficBrief {
    pub origin_title: String,
    pub origin_author: String,
    pub origin_summary: String,
    pub characters: Vec<String>,
    pub world_rules: Vec<String>,
    pub catchphrases: Vec<String>,
    pub fanfic_premise: String,
    pub forbidden: Vec<String>,
}

pub fn brief_path(novel_root: &Path) -> PathBuf {
    novel_root.join("story").join("fanfic_brief.md")
}

pub fn save(novel_root: &Path, brief: &FanficBrief) -> Result<()> {
    fs::create_dir_all(novel_root.join("story"))?;
    fs::write(brief_path(novel_root), to_markdown(brief))
        .with_context(|| format!("write {:?}", brief_path(novel_root)))?;
    Ok(())
}

pub fn read(novel_root: &Path) -> String {
    fs::read_to_string(brief_path(novel_root)).unwrap_or_default()
}

fn to_markdown(b: &FanficBrief) -> String {
    let join = |v: &[String]| -> String {
        if v.is_empty() {
            "- 待补充".into()
        } else {
            v.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n")
        }
    };
    format!(
        "# 同人书草案（Fanfic Brief）\n\n> 由 InkOS Desktop 同人向导生成；可在 Studio 内继续编辑。\n\n## 原作\n- 名称：{title}\n- 作者：{author}\n\n## 原作概要\n{summary}\n\n## 沿用角色\n{chars}\n\n## 沿用世界规则\n{world}\n\n## 角色口头禅 / 标志性台词\n{catch}\n\n## 同人主线设定\n{prem}\n\n## 禁忌\n{forb}\n",
        title = b.origin_title.trim(),
        author = b.origin_author.trim(),
        summary = if b.origin_summary.trim().is_empty() { "待补充" } else { b.origin_summary.trim() },
        chars = join(&b.characters),
        world = join(&b.world_rules),
        catch = join(&b.catchphrases),
        prem = if b.fanfic_premise.trim().is_empty() { "待补充" } else { b.fanfic_premise.trim() },
        forb = join(&b.forbidden),
    )
}

/// 从一段原作样本中启发式抽取候选项（不调用 LLM）。
pub fn quick_extract(sample: &str) -> FanficBrief {
    let mut brief = FanficBrief::default();
    let dlg: Vec<&str> = sample
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            if t.starts_with('"') || t.starts_with('"') || t.starts_with('「') {
                Some(t)
            } else {
                None
            }
        })
        .collect();
    let mut catch = Vec::new();
    for d in dlg.iter().take(5) {
        catch.push(d.trim().to_string());
    }
    brief.catchphrases = catch;

    // 候选角色：所有「{X}道」「{X}说」前的 2-3 字短语
    let mut chars = std::collections::HashSet::new();
    for sep in ["道", "说", "笑道", "冷哼"] {
        for (i, _) in sample.match_indices(sep) {
            let start = i.saturating_sub(8);
            let around = &sample[start..i];
            if let Some(name) = around.rsplit(|c: char| {
                c == '\n' || c == '。' || c == '，' || c == '！' || c == '？' || c == '、'
            }).next()
            {
                let t = name.trim();
                if t.chars().count() >= 1 && t.chars().count() <= 6 {
                    chars.insert(t.to_string());
                }
            }
        }
    }
    let mut chars: Vec<_> = chars.into_iter().collect();
    chars.sort();
    chars.truncate(20);
    brief.characters = chars;

    brief.origin_summary = sample
        .lines()
        .take(6)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    brief
}

#[derive(Debug, Clone)]
pub struct BatchBatch {
    pub from: i32,
    pub to: i32,
    pub estimated_minutes: i32,
}

#[derive(Debug, Clone)]
pub struct BatchPlan {
    pub start_chapter: i32,
    pub total_chapters: i32,
    pub batch_size: i32,
    pub minutes_per_chapter: i32,
    pub batches: Vec<BatchBatch>,
}

pub fn plan_batch(
    start_chapter: i32,
    total_chapters: i32,
    batch_size: i32,
    minutes_per_chapter: i32,
) -> BatchPlan {
    let batch_size = batch_size.max(1);
    let total = total_chapters.max(0);
    let mpc = minutes_per_chapter.max(1);
    let mut batches = Vec::new();
    let mut from = start_chapter.max(1);
    let end_excl = from + total;
    while from < end_excl {
        let to = (from + batch_size - 1).min(end_excl - 1);
        let chs = to - from + 1;
        batches.push(BatchBatch {
            from,
            to,
            estimated_minutes: chs * mpc,
        });
        from = to + 1;
    }
    BatchPlan {
        start_chapter: start_chapter.max(1),
        total_chapters: total,
        batch_size,
        minutes_per_chapter: mpc,
        batches,
    }
}
