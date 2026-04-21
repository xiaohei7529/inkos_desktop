//! AI 刷新 `story_state/` 档案（对齐 inkoswin `build_state_document_prompts` + `_state_documents_worker`）。
//!
//! - `chapter_summaries.md`：仅本地 `build_chapter_summaries_document` 重建，不调 LLM。
//! - `book_rules.md`：不参与批量刷新（硬约束由用户或专用「AI 生成」维护）。

use std::collections::HashMap;

use crate::project::NovelProject;

/// `story_state/` 下会参与「相关档案摘要」拼接的全部文件名（含 book_rules）。
pub const STATE_DIR_FILENAMES: &[&str] = &[
    "book_rules.md",
    "current_state.md",
    "particle_ledger.md",
    "pending_hooks.md",
    "chapter_summaries.md",
    "novel_brief.md",
    "subplot_board.md",
    "emotional_arcs.md",
    "character_matrix.md",
];

/// 与 inkoswin `STATE_FILE_SPECS` 顺序一致；不含 `book_rules.md`。
pub const REFRESH_ALL_ORDER: &[&str] = &[
    "current_state.md",
    "particle_ledger.md",
    "pending_hooks.md",
    "chapter_summaries.md",
    "novel_brief.md",
    "subplot_board.md",
    "emotional_arcs.md",
    "character_matrix.md",
];

#[derive(Debug, Clone)]
pub struct StateFileSpec {
    pub filename: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub ai_guidance: &'static str,
}

const SPECS: &[StateFileSpec] = &[
    StateFileSpec {
        filename: "current_state.md",
        title: "世界状态",
        description: "角色位置、关系网络、已知信息、情感弧线",
        ai_guidance: "聚焦当前时间线、角色位置、关系变化、信息边界和情感温度。",
    },
    StateFileSpec {
        filename: "particle_ledger.md",
        title: "资源账本",
        description: "物品、金钱、物资数量及衰减追踪",
        ai_guidance: "记录重要资源、货币、道具、消耗品和关键物资的来源、变化与剩余量。",
    },
    StateFileSpec {
        filename: "pending_hooks.md",
        title: "未闭合伏笔",
        description: "铺垫、对读者的承诺、未解决冲突",
        ai_guidance: "只保留仍然有效的伏笔、承诺与冲突，并标注预期兑现方向。",
    },
    StateFileSpec {
        filename: "chapter_summaries.md",
        title: "各章摘要",
        description: "出场人物、关键事件、状态变化、伏笔动态",
        ai_guidance: "按章节总结人物出场、关键事件、状态变化与伏笔推进，保持简明清晰。",
    },
    StateFileSpec {
        filename: "novel_brief.md",
        title: "小说设定摘要",
        description: "故事核心、角色、世界观、文风、大纲等写作台设定在长期档案中的镜像",
        ai_guidance: "与项目设定保持一致，结构固定为六个二级标题，便于写作台一键载入；不足处写「待补充」，勿编造未出现的情节。",
    },
    StateFileSpec {
        filename: "subplot_board.md",
        title: "支线进度板",
        description: "A/B/C 线状态、停滞检测",
        ai_guidance: "把主要支线拆成 A/B/C 线，标注推进阶段、最近触发章节和停滞风险。",
    },
    StateFileSpec {
        filename: "emotional_arcs.md",
        title: "情感弧线",
        description: "按角色追踪情绪变化和成长",
        ai_guidance: "按角色跟踪情绪起伏、触发事件、成长节点和未完成的心理课题。",
    },
    StateFileSpec {
        filename: "character_matrix.md",
        title: "角色交互矩阵",
        description: "相遇记录、信息边界",
        ai_guidance: "记录角色相遇、交互张力、彼此掌握的信息差，以及关系演化方向。",
    },
];

pub fn spec_for(filename: &str) -> Option<&'static StateFileSpec> {
    SPECS.iter().find(|s| s.filename == filename)
}

pub fn is_ai_refreshable(filename: &str) -> bool {
    spec_for(filename).is_some()
}

pub fn allowed_state_filename(filename: &str) -> bool {
    STATE_DIR_FILENAMES.iter().any(|f| *f == filename)
}

fn trim_text(text: &str, limit: usize) -> String {
    let stripped = text.trim();
    let count = stripped.chars().count();
    if count <= limit {
        return stripped.to_string();
    }
    let cut: String = stripped.chars().take(limit).collect();
    format!("{}...", cut.trim_end())
}

/// 去掉模型偶尔包上的整段 ```markdown 围栏（对齐 inkoswin `_sanitize_model_markdown`）。
pub fn sanitize_model_markdown(text: &str) -> String {
    let mut stripped = text.trim().to_string();
    if stripped.starts_with("```") && stripped.ends_with("```") {
        let lines: Vec<&str> = stripped.lines().collect();
        if lines.len() >= 3 {
            stripped = lines[1..lines.len() - 1].join("\n").trim().to_string();
        }
    }
    stripped
}

/// 构造单文件刷新 prompt（对齐 inkoswin `story_generator.build_state_document_prompts`）。
pub fn build_state_document_prompts(
    project: &NovelProject,
    spec: &StateFileSpec,
    current_content: &str,
    chapter_digest: &str,
    state_documents: &HashMap<String, String>,
) -> (String, String) {
    let system_prompt = "你是一名长篇小说 continuity bible 管理员。\
你的职责是把当前小说进度整理成稳定、可追踪、可供后续续写直接参考的 Markdown 状态文件。\
不要写空话，不要重复模板说明，只输出该文件最终内容。";

    let mut related_context: Vec<String> = Vec::new();
    for filename in STATE_DIR_FILENAMES {
        if *filename == spec.filename {
            continue;
        }
        let Some(raw) = state_documents.get(*filename) else {
            continue;
        };
        let trimmed = trim_text(raw, 800);
        if trimmed.is_empty() {
            continue;
        }
        let title = spec_for(filename)
            .map(|s| s.title)
            .unwrap_or(filename);
        related_context.push(format!("[{filename} | {title}]\n{trimmed}"));
    }

    let related_block = if related_context.is_empty() {
        "暂无。".to_string()
    } else {
        related_context.join("\n\n")
    };

    let user_prompt = format!(
        r#"请刷新小说《{title}》的状态文件 `{filename}`。

[文件定位]
- 标题：{doc_title}
- 说明：{description}
- 写作要求：{ai_guidance}

[小说设定]
- 题材：{genre}
- 核心故事：{premise}
- 主角与关键角色：{protagonists}
- 世界观与背景：{world_setting}
- 文风与节奏：{writing_style}
- 大纲：{outline}
- 额外要求：{extra}

[章节摘要]
{chapter_digest}

[当前文件内容]
{current_body}

[其他状态文件摘要]
{related_block}

请输出一个完整的 Markdown 文件，并满足：
1. 只保留当前仍然有效的信息。
2. 信息不足处明确写“待补充”，不要编造细节。
3. 结构清晰，适合后续章节写作时直接查阅。
4. 输出时不要加解释，不要加代码块围栏。
"#,
        title = if project.title.trim().is_empty() {
            "未命名小说"
        } else {
            project.title.trim()
        },
        filename = spec.filename,
        doc_title = spec.title,
        description = spec.description,
        ai_guidance = spec.ai_guidance,
        genre = empty_as_placeholder(&project.genre),
        premise = empty_as_placeholder(&project.premise),
        protagonists = empty_as_placeholder(&project.protagonists),
        world_setting = empty_as_placeholder(&project.world_setting),
        writing_style = empty_as_placeholder(&project.writing_style),
        outline = empty_as_placeholder(&project.outline),
        extra = if project.extra_guidance.trim().is_empty() {
            "无".to_string()
        } else {
            project.extra_guidance.trim().to_string()
        },
        chapter_digest = chapter_digest.trim(),
        current_body = if current_content.trim().is_empty() {
            "当前文件为空，请从头生成。".to_string()
        } else {
            current_content.trim().to_string()
        },
        related_block = related_block,
    );

    (system_prompt.to_string(), user_prompt)
}

fn empty_as_placeholder(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        "未填写".to_string()
    } else {
        t.to_string()
    }
}
