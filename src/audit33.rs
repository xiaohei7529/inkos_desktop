//! 33 维审计 + 去 AI 味维度。
//!
//! 灵感来源：Narcooo/inkos `inkos audit` 的 33-dim continuity & anti-AI checklist。
//! 这里把维度做成 Rust 常量，可以独立用于审计 prompt 拼装、UI 显示与本地启发式打分。

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditCategory {
    Continuity,
    Character,
    Plot,
    Style,
    AntiAi,
    World,
}

impl AuditCategory {
    pub fn label(&self) -> &'static str {
        match self {
            AuditCategory::Continuity => "连续性",
            AuditCategory::Character => "人物",
            AuditCategory::Plot => "情节",
            AuditCategory::Style => "文风",
            AuditCategory::AntiAi => "去 AI 味",
            AuditCategory::World => "世界观",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AuditDim {
    pub id: &'static str,
    pub category: AuditCategory,
    pub label: &'static str,
    pub question: &'static str,
}

pub const AUDIT_DIMS: &[AuditDim] = &[
    // 连续性
    AuditDim { id: "c01", category: AuditCategory::Continuity, label: "时间线一致", question: "本章时间推进是否与上一章末尾连贯？" },
    AuditDim { id: "c02", category: AuditCategory::Continuity, label: "地点过渡", question: "出场地点是否给出合理过渡，没有空降？" },
    AuditDim { id: "c03", category: AuditCategory::Continuity, label: "道具连贯", question: "关键道具的位置/状态是否与 particle_ledger 一致？" },
    AuditDim { id: "c04", category: AuditCategory::Continuity, label: "知情度边界", question: "角色是否知道本章节出现的信息（非 god view 漏给）？" },
    AuditDim { id: "c05", category: AuditCategory::Continuity, label: "伏笔回收", question: "本章是否回收/推进了 pending_hooks 中的伏笔？" },
    AuditDim { id: "c06", category: AuditCategory::Continuity, label: "因果链", question: "事件发生是否有合理的内/外因，避免「突然就」？" },
    // 人物
    AuditDim { id: "p01", category: AuditCategory::Character, label: "人设一致", question: "主要角色言行是否与 character_matrix 设定一致？" },
    AuditDim { id: "p02", category: AuditCategory::Character, label: "动机充分", question: "角色行为动机是否在文本中可被读者感知？" },
    AuditDim { id: "p03", category: AuditCategory::Character, label: "对白个性", question: "对白是否带有该角色独有的口头禅/语气？" },
    AuditDim { id: "p04", category: AuditCategory::Character, label: "情绪曲线", question: "本章情绪曲线是否与 emotional_arcs 中的目标走向一致？" },
    AuditDim { id: "p05", category: AuditCategory::Character, label: "成长可见", question: "主角是否在本章产生哪怕极小的成长/变化？" },
    // 情节
    AuditDim { id: "n01", category: AuditCategory::Plot, label: "本章目标", question: "章节意图卡中的核心目标是否达成？" },
    AuditDim { id: "n02", category: AuditCategory::Plot, label: "推进感", question: "整体故事是否往前推进了一步，而非原地踏步？" },
    AuditDim { id: "n03", category: AuditCategory::Plot, label: "节奏", question: "动作 / 对白 / 心理 / 描写比例是否平衡？" },
    AuditDim { id: "n04", category: AuditCategory::Plot, label: "悬念结尾", question: "结尾是否留有合理悬念或情感余韵？" },
    AuditDim { id: "n05", category: AuditCategory::Plot, label: "支线推进", question: "subplot_board 中至少一条支线是否被推进或冷处理标注？" },
    AuditDim { id: "n06", category: AuditCategory::Plot, label: "无注水", question: "是否存在与目标无关的注水段落？" },
    // 文风
    AuditDim { id: "s01", category: AuditCategory::Style, label: "句式多样", question: "句式长短是否多样，无单一节奏？" },
    AuditDim { id: "s02", category: AuditCategory::Style, label: "词汇密度", question: "动词/具象名词占比是否高于形容词堆叠？" },
    AuditDim { id: "s03", category: AuditCategory::Style, label: "语感统一", question: "整体语感是否与 writing_style 中的设定一致？" },
    AuditDim { id: "s04", category: AuditCategory::Style, label: "感官层次", question: "是否调用了至少两种感官（视/听/触/味/嗅）？" },
    AuditDim { id: "s05", category: AuditCategory::Style, label: "意象", question: "是否有 1-2 个独特意象作为本章 visual hook？" },
    // 去 AI 味
    AuditDim { id: "a01", category: AuditCategory::AntiAi, label: "套话识别", question: "是否大量出现「不仅…而且」「事实上」「与此同时」等模式化连接？" },
    AuditDim { id: "a02", category: AuditCategory::AntiAi, label: "排比堆叠", question: "是否出现 GPT 式三段排比（A，B，C 三件事）？" },
    AuditDim { id: "a03", category: AuditCategory::AntiAi, label: "总结欲", question: "段落末尾是否存在 AI 喜爱的「概括 + 升华」收尾？" },
    AuditDim { id: "a04", category: AuditCategory::AntiAi, label: "形容词堆叠", question: "是否在描写中堆叠 3+ 个同义形容词？" },
    AuditDim { id: "a05", category: AuditCategory::AntiAi, label: "人称漂移", question: "第三人称视角是否被 AI 漂移成「我们」「你」？" },
    AuditDim { id: "a06", category: AuditCategory::AntiAi, label: "破折号滥用", question: "是否过度使用破折号 / 圆括号补充？" },
    AuditDim { id: "a07", category: AuditCategory::AntiAi, label: "中性温吞", question: "是否表达过分中性，缺乏立场/态度？" },
    // 世界观
    AuditDim { id: "w01", category: AuditCategory::World, label: "设定不破", question: "是否违反 world_setting / book_rules 中的世界观？" },
    AuditDim { id: "w02", category: AuditCategory::World, label: "技/法体系", question: "技/法/能力使用是否遵循设定的代价与限制？" },
    AuditDim { id: "w03", category: AuditCategory::World, label: "组织/称谓", question: "出现的组织、称谓、官职是否与既定设定一致？" },
    AuditDim { id: "w04", category: AuditCategory::World, label: "文化合理", question: "对话与风俗描写是否符合世界文化语境？" },
];

pub fn checklist_markdown() -> String {
    let mut groups: BTreeMap<&'static str, Vec<&AuditDim>> = BTreeMap::new();
    for d in AUDIT_DIMS {
        groups.entry(d.category.label()).or_default().push(d);
    }
    let mut out = String::new();
    out.push_str("### 33 维审计清单\n");
    for (cat, dims) in groups {
        out.push_str(&format!("\n#### {cat}\n"));
        for d in dims {
            out.push_str(&format!("- [{}] {}：{}\n", d.id, d.label, d.question));
        }
    }
    out.push_str("\n请对每条给出：通过 / 风险 / 不通过 三态，给出证据原文片段（≤30 字），并在末尾给出整体改稿建议（不超过 10 条）。\n");
    out
}

/// 启发式快速扫描（不调用 LLM），返回 (维度 id, 命中次数)。
pub fn quick_scan(body: &str) -> Vec<(&'static str, usize)> {
    let patterns: &[(&str, &[&str])] = &[
        ("a01", &["不仅", "而且", "事实上", "与此同时", "总而言之"]),
        ("a02", &["第一", "第二", "第三"]),
        ("a06", &["——", "（", "(", "()"]),
        ("a04", &["闪闪发光", "波澜壮阔", "无与伦比"]),
        ("a07", &["或许", "也许", "可能", "似乎"]),
    ];
    let mut out = Vec::new();
    for (id, words) in patterns {
        let n: usize = words.iter().map(|w| body.matches(w).count()).sum();
        if n > 0 {
            out.push((*id, n));
        }
    }
    out
}
