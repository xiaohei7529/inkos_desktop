//! AIGC 检测：本地启发式分数 + LLM 评分 prompt。
//!
//! 灵感来源：Narcooo/inkos `inkos detect` 命令。
//!
//! 启发式只是「快筛」，最终建议结合 LLM 评分。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AigcSignal {
    pub label: String,
    pub hits: usize,
    pub score_delta: f32,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AigcReport {
    pub overall_score: f32,
    pub level: String,
    pub signals: Vec<AigcSignal>,
    pub samples: Vec<String>,
}

/// 评分约定：0 = 100% 人类，1 = 100% AI；0.4-0.6 不确定。
pub fn quick_score(body: &str) -> AigcReport {
    let mut report = AigcReport::default();
    let total_chars = body.chars().count() as f32;
    if total_chars < 200.0 {
        report.level = "样本太短".into();
        return report;
    }

    let mut score = 0.20_f32; // baseline

    let push = |r: &mut AigcReport, s: AigcSignal, score: &mut f32| {
        if s.hits > 0 {
            *score += s.score_delta;
        }
        r.signals.push(s);
    };

    // 1) 套话短语
    let cliches = [
        "不仅",
        "而且",
        "事实上",
        "与此同时",
        "然而",
        "然而值得注意的是",
        "总而言之",
        "综上所述",
        "正所谓",
        "毋庸置疑",
    ];
    let mut cliche_hits = 0usize;
    let mut cliche_samples = Vec::new();
    for w in cliches {
        let n = body.matches(w).count();
        if n > 0 {
            cliche_hits += n;
            if cliche_samples.len() < 4 {
                cliche_samples.push(w.to_string());
            }
        }
    }
    push(
        &mut report,
        AigcSignal {
            label: "套话短语".into(),
            hits: cliche_hits,
            score_delta: (cliche_hits as f32 * 0.018).min(0.18),
            note: format!("命中：{}", cliche_samples.join("、")),
        },
        &mut score,
    );

    // 2) 三段排比 / 对仗模板
    let trio = body.matches("第一").count() + body.matches("首先").count();
    push(
        &mut report,
        AigcSignal {
            label: "排比模板".into(),
            hits: trio,
            score_delta: (trio as f32 * 0.04).min(0.12),
            note: "「第一/第二/第三」「首先/其次/最后」结构".into(),
        },
        &mut score,
    );

    // 3) 破折号 / 圆括号补充
    let dashes = body.matches("——").count() + body.matches("(").count() + body.matches("（").count();
    push(
        &mut report,
        AigcSignal {
            label: "破折号 / 圆括号补充".into(),
            hits: dashes,
            score_delta: (dashes as f32 * 0.01).min(0.10),
            note: "AI 模型偏爱用破折号补充说明".into(),
        },
        &mut score,
    );

    // 4) 形容词堆叠
    let stacked = stacked_adjective_hits(body);
    push(
        &mut report,
        AigcSignal {
            label: "形容词堆叠".into(),
            hits: stacked,
            score_delta: (stacked as f32 * 0.04).min(0.16),
            note: "三个以上同义形容词连用".into(),
        },
        &mut score,
    );

    // 5) 中性温吞词
    let modals = body.matches("或许").count()
        + body.matches("也许").count()
        + body.matches("似乎").count()
        + body.matches("大概").count();
    push(
        &mut report,
        AigcSignal {
            label: "中性温吞词".into(),
            hits: modals,
            score_delta: (modals as f32 * 0.012).min(0.10),
            note: "「或许/也许/似乎/大概」频度高".into(),
        },
        &mut score,
    );

    // 6) 句长方差（句长太均匀 → AI 嫌疑）
    let sentences: Vec<&str> = body
        .split(|c: char| matches!(c, '。' | '！' | '？' | '!' | '?' | '…'))
        .filter(|s| !s.trim().is_empty())
        .collect();
    let var = sentence_length_variance(&sentences);
    let monotone_hit = if var < 16.0 && sentences.len() >= 8 { 1 } else { 0 };
    push(
        &mut report,
        AigcSignal {
            label: "句长单调".into(),
            hits: monotone_hit,
            score_delta: if monotone_hit == 1 { 0.10 } else { 0.0 },
            note: format!("句长方差 {:.1}", var),
        },
        &mut score,
    );

    let score = score.clamp(0.0, 1.0);
    report.overall_score = score;
    report.level = if score < 0.30 {
        "偏人类".into()
    } else if score < 0.55 {
        "可疑".into()
    } else {
        "偏 AI".into()
    };
    if !cliche_samples.is_empty() {
        report.samples = cliche_samples;
    }
    report
}

fn stacked_adjective_hits(body: &str) -> usize {
    let chars: Vec<char> = body.chars().collect();
    let mut hits = 0usize;
    let mut run = 0usize;
    for c in chars {
        if c == '的' {
            run += 1;
            if run >= 3 {
                hits += 1;
                run = 0;
            }
        } else if c == '，' || c == '。' || c == '；' || c == '！' || c == '？' || c == '\n' {
            run = 0;
        }
    }
    hits
}

fn sentence_length_variance(sentences: &[&str]) -> f32 {
    if sentences.is_empty() {
        return 0.0;
    }
    let lens: Vec<f32> = sentences.iter().map(|s| s.chars().count() as f32).collect();
    let mean = lens.iter().sum::<f32>() / lens.len() as f32;
    let var = lens.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / lens.len() as f32;
    var
}

/// LLM 评分 prompt：让模型按 InkOS 标准输出 0-1 评分 + 证据。
pub fn llm_judge_prompt(body: &str) -> String {
    let mut out = String::new();
    out.push_str("## 任务\n");
    out.push_str(
        "请把下列中文段落判定其「AI 生成痕迹」程度，输出严格的 JSON：\n\n",
    );
    out.push_str("```\n{\n  \"score\": 0.0,        // 0-1，越高越像 AI\n  \"verdict\": \"偏人类|可疑|偏AI\",\n  \"evidence\": [\"原文片段1\", \"...\"],\n  \"suggestions\": [\"如何改写以降低 AI 痕迹\"]\n}\n```\n\n");
    out.push_str("评分参考：套话连接、三段排比、破折号补充、形容词堆叠、中性温吞、句长单调、缺少独特意象、过度概括收尾。\n\n## 文本\n");
    out.push_str(body);
    out
}
