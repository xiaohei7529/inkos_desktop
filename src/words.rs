//! 字数治理：目标字数 ± 容差 + 单次纠偏归一化 prompt。
//!
//! 灵感来源：Narcooo/inkos 的 word-count governor 与 normalize 阶段。

use crate::chapter_md::count_story_units;

#[derive(Debug, Clone, Copy)]
pub struct WordBudget {
    pub goal: i32,
    pub tolerance: i32,
}

impl Default for WordBudget {
    fn default() -> Self {
        Self {
            goal: 2500,
            tolerance: 350,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordStatus {
    Under,
    OnTarget,
    Over,
}

impl WordStatus {
    pub fn label(&self) -> &'static str {
        match self {
            WordStatus::Under => "偏短",
            WordStatus::OnTarget => "达标",
            WordStatus::Over => "偏长",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WordReport {
    pub actual: i32,
    pub goal: i32,
    pub tolerance: i32,
    pub min: i32,
    pub max: i32,
    pub delta: i32,
    pub status: WordStatus,
}

impl WordReport {
    pub fn ratio(&self) -> f32 {
        if self.goal <= 0 {
            return 1.0;
        }
        (self.actual as f32) / (self.goal as f32)
    }
}

pub fn evaluate(body: &str, budget: WordBudget) -> WordReport {
    let actual = count_story_units(body) as i32;
    let goal = budget.goal.max(0);
    let tol = budget.tolerance.max(0);
    let min = (goal - tol).max(0);
    let max = goal + tol;
    let status = if actual < min {
        WordStatus::Under
    } else if actual > max {
        WordStatus::Over
    } else {
        WordStatus::OnTarget
    };
    WordReport {
        actual,
        goal,
        tolerance: tol,
        min,
        max,
        delta: actual - goal,
        status,
    }
}

/// 单次纠偏归一化 prompt：交给 normalize agent 调用，仅一次性纠偏，不允许反复扩写。
pub fn normalize_prompt(body: &str, report: &WordReport) -> String {
    let mut out = String::new();
    out.push_str("## 任务：字数归一化（单次纠偏）\n");
    out.push_str(&format!(
        "目标：{} 字；当前：{} 字（{}）。允许波动 ±{}，目标区间 [{}, {}]。\n",
        report.goal,
        report.actual,
        report.status.label(),
        report.tolerance,
        report.min,
        report.max
    ));
    match report.status {
        WordStatus::Under => {
            out.push_str(&format!(
                "请扩写约 {} 字，方法限定为：①补充感官细节；②加入一段必要的人物心理；③在已有情节节点之间补一个动作过渡。\n禁止：新增剧情线、新增角色、改变结尾。\n",
                report.min - report.actual
            ));
        }
        WordStatus::Over => {
            out.push_str(&format!(
                "请精简约 {} 字，方法限定为：①去掉冗余描写；②合并重复信息；③缩短形容词堆叠。\n禁止：删除关键动作或对白、改变情节走向。\n",
                report.actual - report.max
            ));
        }
        WordStatus::OnTarget => {
            out.push_str("字数已在区间内，仅做最小润色：去除明显赘字、修正错别字、统一标点。\n");
        }
    }
    out.push_str("\n## 输出格式\n仅输出归一化后的章节正文，不要解释。\n\n## 原稿\n");
    out.push_str(body);
    out
}
