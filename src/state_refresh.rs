//! AI 刷新 `story_state/` 档案（对齐 inkoswin `build_state_document_prompts` + `_state_documents_worker`）。
//!
//! - `chapter_summaries.md`：仅本地 `build_chapter_summaries_document` 重建，不调 LLM。
//! - `book_rules.md`：默认不参与批量刷新（首章收尾可按业务侧策略单独触发）。

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
    "outline.md",
    "subplot_board.md",
    "emotional_arcs.md",
    "character_matrix.md",
];

/// `story/` 下纳入治理层刷新的文件。
pub const STORY_CONTROL_FILENAMES: &[&str] = &[
    "story/author_intent.md",
    "story/current_focus.md",
];

/// 构造「其他状态文件摘要」时会参考的文件（含 `story_state/` + `story/`）。
pub const RELATED_CONTEXT_FILENAMES: &[&str] = &[
    "book_rules.md",
    "current_state.md",
    "particle_ledger.md",
    "pending_hooks.md",
    "chapter_summaries.md",
    "novel_brief.md",
    "outline.md",
    "subplot_board.md",
    "emotional_arcs.md",
    "character_matrix.md",
    "story/author_intent.md",
    "story/current_focus.md",
];

/// 与 inkoswin `STATE_FILE_SPECS` 顺序一致；不含 `book_rules.md`。
pub const REFRESH_ALL_ORDER: &[&str] = &[
    "current_state.md",
    "particle_ledger.md",
    "pending_hooks.md",
    "chapter_summaries.md",
    "novel_brief.md",
    "outline.md",
    "subplot_board.md",
    "emotional_arcs.md",
    "character_matrix.md",
    "story/author_intent.md",
    "story/current_focus.md",
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
        filename: "outline.md",
        title: "书籍大纲",
        description: "故事大纲（宏观走向）、分卷大纲、细纲（章节级规划）",
        ai_guidance: "在细纲部分补充最新章节的规划（场景/人物/事件/钩子），宏观大纲除非剧情发生重大转折否则保持不变；细纲要具体可执行，防止续写时跑偏。",
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
    StateFileSpec {
        filename: "story/author_intent.md",
        title: "作者长期意图",
        description: "长期叙事方向、主题承诺、创作边界与不变核心",
        ai_guidance: "聚焦整本书的长期目标与边界，保持抽象稳定，不写具体剧情流水账。",
    },
    StateFileSpec {
        filename: "story/current_focus.md",
        title: "当前焦点（近 1-3 章）",
        description: "近期章节重点推进目标、关键冲突、情绪与节奏控制",
        ai_guidance: "只保留近 1-3 章可执行焦点，突出短期推进任务与风险，避免发散。",
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

/// Beats First Workflow 的上下文过滤：按本章节拍中的关键词，从 `story_state/` 档案集合里
/// 挑出真正相关的文件，避免给 LLM 灌入整盘上下文造成"注意力分散"。
///
/// 策略：
/// - **核心档案常驻**：`outline.md` / `novel_brief.md` / `chapter_summaries.md` / `book_rules.md` /
///   `current_state.md` 始终保留——它们是写作连续性的基础。
/// - **条件档案**：`character_matrix.md` / `emotional_arcs.md` / `particle_ledger.md` /
///   `pending_hooks.md` / `subplot_board.md` 只在 beats 中出现对应关键词时注入。
/// - **beats 为空时**回退为返回原 `docs` 克隆（保证旧流程不退化）。
///
/// 关键词在 beats.join("\n") 上做小写 substring 匹配。
pub fn filter_state_docs_by_beats(
    docs: &HashMap<String, String>,
    beats: &[String],
) -> HashMap<String, String> {
    if beats.iter().all(|b| b.trim().is_empty()) {
        return docs.clone();
    }

    // 核心档案：始终保留
    const ALWAYS_KEEP: &[&str] = &[
        "outline.md",
        "novel_brief.md",
        "chapter_summaries.md",
        "book_rules.md",
        "current_state.md",
    ];

    // 条件档案 + 触发它们的中文/英文关键词
    const CONDITIONAL: &[(&str, &[&str])] = &[
        (
            "character_matrix.md",
            &["角色", "关系", "矩阵", "相遇", "信息差", "结识", "敌对"],
        ),
        (
            "emotional_arcs.md",
            &["情感", "情绪", "心理", "成长", "心结", "动摇", "纠葛"],
        ),
        (
            "particle_ledger.md",
            &["物品", "道具", "资源", "钱", "货币", "物资", "灵石", "药材", "装备"],
        ),
        (
            "pending_hooks.md",
            &["伏笔", "承诺", "冲突", "钩子", "悬念", "回收", "兑现"],
        ),
        (
            "subplot_board.md",
            &["支线", "副线", "进度", "并行", "次要剧情"],
        ),
    ];

    let haystack = beats.join("\n").to_lowercase();
    let mut out: HashMap<String, String> = HashMap::new();
    for &key in ALWAYS_KEEP {
        if let Some(v) = docs.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    for &(filename, keywords) in CONDITIONAL {
        let hit = keywords
            .iter()
            .any(|kw| haystack.contains(&kw.to_lowercase()));
        if hit {
            if let Some(v) = docs.get(filename) {
                out.insert(filename.to_string(), v.clone());
            }
        }
    }
    out
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
    for filename in RELATED_CONTEXT_FILENAMES {
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

/// 批量 JSON Delta 模式的状态档案刷新 prompt（Phase 2）。
///
/// 与 `app.rs::start_state_sync` 同构：单次 LLM 调用返回 `StateSyncReport` JSON，
/// 由 `state_sync::apply_updates` 应用到 `story_state/*.md`。相对原逐文件 Markdown
/// 重写，能把 N 次往返折成 1 次，并避免覆盖整个文件。
///
/// - `target_files`：本次允许 LLM 修改的文件白名单（已剔除 chapter_summaries.md / book_rules.md）。
/// - `state_documents`：当前各档案的真实内容。已被 `filter_state_docs_by_beats` 过滤，
///   只注入与节拍相关的档案，避免上下文爆炸。
/// - `beats`：最新章节的本章节拍，作为本轮聚焦提示注入到 user prompt（非空时）。
pub fn build_batch_delta_prompts(
    project: &NovelProject,
    target_files: &[&str],
    state_documents: &HashMap<String, String>,
    chapter_digest: &str,
    beats: &[String],
) -> (String, String) {
    let allowed_files = target_files
        .iter()
        .map(|f| format!("- {f}"))
        .collect::<Vec<_>>()
        .join("\n");
    let allowed_or_none = if allowed_files.is_empty() {
        "（本轮无可刷新文件）".to_string()
    } else {
        allowed_files
    };

    let system_prompt = format!(
        "你是一名严谨的长篇小说 continuity bible 维护编辑。\n\
         基于「最近章节摘要」与「当前状态档案」的对比，仅输出**有实际变化**的文件更新；没有变化的文件**不要**列出。\n\n\
         允许修改的文件白名单：\n{allowed_or_none}\n\n\
         严格输出**单个 JSON 对象**，禁止任何额外说明或 Markdown 围栏。Schema：\n\
         {{\n  \"summary\": \"本轮刷新一句话摘要\",\n  \"updates\": [\n    {{ \"file\": \"current_state.md\", \"action\": \"replace\" | \"patch\", \"content\": \"...\" }}\n  ]\n}}\n\n\
         规则：\n\
         - action=replace：content 必须是该文件的完整新版本（含原有 frontmatter / 标题等结构）。\n\
         - action=patch：content 由若干块组成，每块格式必须**精确**：\n\
           ===REPLACE_BLOCK===\\n旧片段（需精确匹配现文件中的连续子串）\\n===WITH===\\n新片段\\n===END===\n\
         - 优先使用 patch（小改），仅在结构大改时使用 replace。\n\
         - 没有需要变更的文件就不要列出，updates 可以为空数组。\n\
         - 严禁更新 chapter_summaries.md 与 book_rules.md。\n\
         - 所有改动必须基于「最近章节摘要」中的事件，不得引入未发生的设定。\n\
         - JSON 中字符串内的换行请使用 \\n 转义。"
    );

    let mut related_blocks: Vec<String> = Vec::new();
    // 注入顺序与 ORDERED_STATE_FILES 一致，便于 LLM 形成稳定的引用顺序。
    let injected_order: &[&str] = &[
        "outline.md",
        "novel_brief.md",
        "current_state.md",
        "pending_hooks.md",
        "subplot_board.md",
        "emotional_arcs.md",
        "character_matrix.md",
        "particle_ledger.md",
        "chapter_summaries.md",
        "book_rules.md",
    ];
    for filename in injected_order {
        let Some(raw) = state_documents.get(*filename) else {
            continue;
        };
        let trimmed_raw = raw.trim();
        if trimmed_raw.is_empty() {
            continue;
        }
        let limit = match *filename {
            "outline.md" => 3000,
            "chapter_summaries.md" => 1200,
            "novel_brief.md" => 2200,
            _ => 800,
        };
        let body = trim_text(trimmed_raw, limit);
        let title = spec_for(filename).map(|s| s.title).unwrap_or(filename);
        related_blocks.push(format!("[{filename} | {title}]\n{body}"));
    }
    let related_block = if related_blocks.is_empty() {
        "（暂无状态档案，请基于章节摘要从空开始）".to_string()
    } else {
        related_blocks.join("\n\n")
    };

    let cleaned_beats: Vec<String> = beats
        .iter()
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .collect();
    let beats_section = if cleaned_beats.is_empty() {
        String::new()
    } else {
        let mut s = String::from("[本章节拍]（本轮 LLM 应聚焦的变更点）\n");
        for (i, b) in cleaned_beats.iter().enumerate() {
            s.push_str(&format!("{}. {}\n", i + 1, b));
        }
        s.push('\n');
        s
    };

    let user_prompt = format!(
        "请基于以下信息，输出小说《{title}》最新一轮状态档案变更的 JSON Delta。\n\n\
         [小说设定]\n\
         - 题材：{genre}\n\
         - 核心故事：{premise}\n\
         - 主角与关键角色：{protagonists}\n\
         - 世界观与背景：{world_setting}\n\
         - 文风与节奏：{writing_style}\n\
         - 大纲：{outline}\n\
         - 额外要求：{extra}\n\n\
         {beats_section}\
         [最近章节摘要]\n{chapter_digest}\n\n\
         [当前状态档案]\n{related_block}\n\n\
         请只输出 JSON 对象本身，不要加 Markdown 围栏，不要加任何额外说明。\n",
        title = if project.title.trim().is_empty() {
            "未命名小说"
        } else {
            project.title.trim()
        },
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
        beats_section = beats_section,
        related_block = related_block,
    );

    (system_prompt, user_prompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_docs() -> HashMap<String, String> {
        let mut m = HashMap::new();
        for f in STATE_DIR_FILENAMES {
            m.insert((*f).to_string(), format!("# {f}\n占位内容"));
        }
        m
    }

    #[test]
    fn filter_returns_full_docs_when_beats_empty() {
        let docs = build_docs();
        let out = filter_state_docs_by_beats(&docs, &[]);
        assert_eq!(out.len(), docs.len());
    }

    #[test]
    fn filter_returns_full_docs_when_beats_all_blank() {
        let docs = build_docs();
        let beats = vec!["".to_string(), "   ".to_string()];
        let out = filter_state_docs_by_beats(&docs, &beats);
        assert_eq!(out.len(), docs.len());
    }

    #[test]
    fn filter_keeps_core_and_drops_unrelated_conditional() {
        let docs = build_docs();
        // 与"角色/情感/物品/伏笔/支线"都不相关的纯地点/动作 beats
        let beats = vec!["主角抵达雪山脚下，环顾四周风雪皑皑".to_string()];
        let out = filter_state_docs_by_beats(&docs, &beats);
        // 核心档案保留 5 个
        for must in [
            "outline.md",
            "novel_brief.md",
            "chapter_summaries.md",
            "book_rules.md",
            "current_state.md",
        ] {
            assert!(out.contains_key(must), "missing core file {must}");
        }
        // 条件档案应全部缺席
        for never in [
            "character_matrix.md",
            "emotional_arcs.md",
            "particle_ledger.md",
            "pending_hooks.md",
            "subplot_board.md",
        ] {
            assert!(!out.contains_key(never), "should not include {never}");
        }
    }

    #[test]
    fn filter_includes_character_and_emotion_on_relevant_keywords() {
        let docs = build_docs();
        let beats = vec![
            "主角与师妹的关系出现裂痕，引发心理动摇".to_string(),
            "回收师傅遗物的伏笔".to_string(),
        ];
        let out = filter_state_docs_by_beats(&docs, &beats);
        assert!(out.contains_key("character_matrix.md"));
        assert!(out.contains_key("emotional_arcs.md"));
        assert!(out.contains_key("pending_hooks.md"));
        assert!(!out.contains_key("particle_ledger.md"));
        assert!(!out.contains_key("subplot_board.md"));
    }

    fn sample_project() -> NovelProject {
        let mut p = NovelProject::default();
        p.title = "示例小说".into();
        p.genre = "玄幻".into();
        p.premise = "主角凭借神秘玉佩入仙门".into();
        p.protagonists = "姜临".into();
        p.world_setting = "东大陆 / 北元域".into();
        p.writing_style = "冷峻紧凑".into();
        p.outline = "前 10 章奠定主角根基".into();
        p
    }

    #[test]
    fn batch_delta_prompt_lists_target_files_and_forbids_book_rules() {
        let p = sample_project();
        let docs = build_docs();
        let targets = vec!["current_state.md", "pending_hooks.md", "subplot_board.md"];
        let (system, user) =
            build_batch_delta_prompts(&p, &targets, &docs, "近 3 章摘要 ...", &[]);
        // 白名单列出所有 targets
        for t in &targets {
            assert!(system.contains(t), "system should list target {t}");
        }
        // 严禁列表覆盖 chapter_summaries.md / book_rules.md
        assert!(system.contains("chapter_summaries.md"));
        assert!(system.contains("book_rules.md"));
        // JSON schema 关键字
        assert!(system.contains("\"summary\""));
        assert!(system.contains("\"updates\""));
        assert!(system.contains("===REPLACE_BLOCK==="));
        // user prompt 含小说设定 + 最近章节摘要
        assert!(user.contains("示例小说"));
        assert!(user.contains("近 3 章摘要 ..."));
        // beats 为空时不应出现 [本章节拍] 段
        assert!(!user.contains("[本章节拍]"));
    }

    #[test]
    fn batch_delta_prompt_injects_beats_section() {
        let p = sample_project();
        let docs = build_docs();
        let targets = vec!["current_state.md"];
        let beats = vec![
            "姜临入山门，遇守门长老".to_string(),
            "玉佩首次显露异象".to_string(),
        ];
        let (_system, user) =
            build_batch_delta_prompts(&p, &targets, &docs, "摘要", &beats);
        assert!(user.contains("[本章节拍]"));
        assert!(user.contains("姜临入山门"));
        assert!(user.contains("玉佩首次显露异象"));
    }
}
