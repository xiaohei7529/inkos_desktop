//! 写作数据分析：章节字数趋势、字数达标率、状态分布等。
//!
//! 灵感来源：Narcooo/inkos `inkos analytics` 命令。

use serde::{Deserialize, Serialize};

use crate::project::{ChapterRecord, NovelProject};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AnalyticsReport {
    pub total_chapters: usize,
    pub total_words: i64,
    pub avg_words: f32,
    pub max_words: i32,
    pub min_words: i32,
    pub goal: i32,
    pub on_target: usize,
    pub under_target: usize,
    pub over_target: usize,
    pub status_distribution: Vec<(String, usize)>,
    pub recent_velocity: Vec<VelocityPoint>,
    pub word_count_series: Vec<WordCountPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WordCountPoint {
    pub chapter_no: i32,
    pub words: i32,
    pub status: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VelocityPoint {
    pub date: String,
    pub chapters: usize,
    pub words: i64,
}

pub fn analyze(project: &NovelProject, word_goal: i32, tolerance: i32) -> AnalyticsReport {
    let mut chs: Vec<&ChapterRecord> = project.chapters.iter().collect();
    chs.sort_by_key(|c| c.number);
    let mut report = AnalyticsReport {
        goal: word_goal,
        ..Default::default()
    };

    if chs.is_empty() {
        return report;
    }

    let mut min = i32::MAX;
    let mut max = 0i32;
    let mut sum: i64 = 0;
    let mut on = 0usize;
    let mut under = 0usize;
    let mut over = 0usize;

    let lo = (word_goal - tolerance).max(0);
    let hi = word_goal + tolerance;

    let mut status_count: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut velocity: std::collections::BTreeMap<String, VelocityPoint> =
        std::collections::BTreeMap::new();

    for c in &chs {
        let w = c.word_count.max(0);
        sum += w as i64;
        if w > 0 {
            min = min.min(w);
            max = max.max(w);
        }
        if w < lo {
            under += 1;
        } else if w > hi {
            over += 1;
        } else {
            on += 1;
        }

        report.word_count_series.push(WordCountPoint {
            chapter_no: c.number,
            words: w,
            status: c.status.clone(),
            updated_at: c.updated_at.clone(),
        });

        *status_count.entry(c.status.clone()).or_insert(0) += 1;

        let day = c.updated_at.get(..10).unwrap_or("").to_string();
        if !day.is_empty() {
            let entry = velocity.entry(day.clone()).or_insert(VelocityPoint {
                date: day,
                chapters: 0,
                words: 0,
            });
            entry.chapters += 1;
            entry.words += w as i64;
        }
    }

    report.total_chapters = chs.len();
    report.total_words = sum;
    report.avg_words = sum as f32 / chs.len() as f32;
    report.min_words = if min == i32::MAX { 0 } else { min };
    report.max_words = max;
    report.on_target = on;
    report.under_target = under;
    report.over_target = over;
    report.status_distribution = status_count.into_iter().collect();
    report.status_distribution.sort_by(|a, b| b.1.cmp(&a.1));
    report.recent_velocity = velocity.into_values().collect();
    let n = report.recent_velocity.len();
    if n > 30 {
        report.recent_velocity.drain(0..n - 30);
    }

    report
}
