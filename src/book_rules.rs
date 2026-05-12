//! `book_rules.md` 数据模型与 AI 生成 prompt。
//!
//! 对齐参考项目 `Narcooo/inkos`（`packages/core/src/models/book-rules.ts`
//! + `packages/core/src/agents/architect.ts` 中 `bookRulesPrompt`），把"叙事硬约束"
//! 落地为 YAML frontmatter + Markdown 正文的形式：
//!
//! ```text
//! ---
//! version: "1.0"
//! protagonist:
//!   name: ...
//!   personalityLock: [...]
//!   behavioralConstraints: [...]
//! genreLock:
//!   primary: ...
//!   forbidden: [...]
//! prohibitions: [...]
//! chapterTypesOverride: [...]
//! fatigueWordsOverride: [...]
//! additionalAuditDimensions: [...]
//! enableFullCastTracking: false
//! fanficMode: ...                 # 可选
//! allowedDeviations: [...]        # 可选
//! ---
//!
//! ## 叙事视角
//! ## 核心冲突驱动
//! ```
//!
//! 这里**手写一个轻量级 YAML 解析**，足以覆盖 Schema 中的 scalar / inline list /
//! 嵌套对象（不引入 serde_yaml 依赖）。
//!
//! 该模块只负责 *parse / render / 生成 prompt*；UI 调用与落盘在 `app.rs`。

use crate::project::NovelProject;

#[derive(Debug, Clone, Default)]
pub struct Protagonist {
    pub name: String,
    pub personality_lock: Vec<String>,
    pub behavioral_constraints: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GenreLock {
    pub primary: String,
    pub forbidden: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NumericalSystemOverrides {
    /// `hardCap` 在 inkos 中允许 number 或 string，这里统一存为字符串展示原值。
    pub hard_cap: Option<String>,
    pub resource_types: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EraConstraints {
    pub enabled: bool,
    pub period: Option<String>,
    pub region: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BookRules {
    pub version: String,
    pub protagonist: Option<Protagonist>,
    pub genre_lock: Option<GenreLock>,
    pub numerical_system_overrides: Option<NumericalSystemOverrides>,
    pub era_constraints: Option<EraConstraints>,
    pub prohibitions: Vec<String>,
    pub chapter_types_override: Vec<String>,
    pub fatigue_words_override: Vec<String>,
    pub additional_audit_dimensions: Vec<String>,
    pub enable_full_cast_tracking: bool,
    pub fanfic_mode: Option<String>, // canon | au | ooc | cp
    pub allowed_deviations: Vec<String>,
}

impl Default for BookRules {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            protagonist: None,
            genre_lock: None,
            numerical_system_overrides: None,
            era_constraints: None,
            prohibitions: Vec::new(),
            chapter_types_override: Vec::new(),
            fatigue_words_override: Vec::new(),
            additional_audit_dimensions: Vec::new(),
            enable_full_cast_tracking: false,
            fanfic_mode: None,
            allowed_deviations: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ParsedBookRules {
    pub rules: BookRules,
    pub body: String,
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/// 从原始 markdown 文本中提取 YAML frontmatter 与正文，对齐
/// `parseBookRules`（`packages/core/src/models/book-rules.ts`）。
///
/// - 容忍 LLM 用 ```` ``` ```` / ```` ```md ```` / ```` ```yaml ```` 包裹整段输出；
/// - frontmatter 不一定在文件开头，能匹配到任意位置的 `---\n…\n---`；
/// - frontmatter 解析失败时回退为默认 BookRules + 整段文本作为 body。
pub fn parse_book_rules(raw: &str) -> ParsedBookRules {
    let stripped = strip_code_fence(raw);

    if let Some((fm, body)) = split_frontmatter(&stripped) {
        let rules = parse_yaml_frontmatter(&fm).unwrap_or_default();
        return ParsedBookRules {
            rules,
            body: body.trim().to_string(),
        };
    }

    ParsedBookRules {
        rules: BookRules::default(),
        body: stripped.trim().to_string(),
    }
}

fn strip_code_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let opens = ["```md", "```markdown", "```yaml", "```"];
    for prefix in opens {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            // 跳过紧跟着的换行
            let rest = rest.strip_prefix('\n').unwrap_or(rest);
            if let Some(inner) = rest.strip_suffix("```") {
                return inner.trim_end().to_string();
            }
            // 没有匹配的结尾 ```，原样返回
            return trimmed.to_string();
        }
    }
    trimmed.to_string()
}

fn split_frontmatter(text: &str) -> Option<(String, String)> {
    // 找到第一个 `---` 起始（行首），以及紧跟其后的下一个 `---`。
    let lines: Vec<&str> = text.lines().collect();
    let mut start_idx: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "---" {
            start_idx = Some(i);
            break;
        }
    }
    let start = start_idx?;
    for j in (start + 1)..lines.len() {
        if lines[j].trim() == "---" {
            let fm = lines[(start + 1)..j].join("\n");
            let body = if j + 1 < lines.len() {
                lines[(j + 1)..].join("\n")
            } else {
                String::new()
            };
            return Some((fm, body));
        }
    }
    None
}

fn parse_yaml_frontmatter(fm: &str) -> Option<BookRules> {
    let mut rules = BookRules::default();

    // 把整段拆成顶层键 -> 块；保持顺序无关紧要。
    let blocks = top_level_blocks(fm);
    if blocks.is_empty() {
        return None;
    }

    for (key, value_first_line, child_block) in blocks {
        match key.as_str() {
            "version" => {
                rules.version = unquote_scalar(&value_first_line).unwrap_or_else(|| "1.0".into());
            }
            "protagonist" => {
                let mut p = Protagonist::default();
                for (k, v, _) in top_level_blocks(&child_block) {
                    match k.as_str() {
                        "name" => {
                            p.name = unquote_scalar(&v).unwrap_or_default();
                        }
                        "personalityLock" => {
                            p.personality_lock = parse_list(&v, &child_block, "personalityLock");
                        }
                        "behavioralConstraints" => {
                            p.behavioral_constraints =
                                parse_list(&v, &child_block, "behavioralConstraints");
                        }
                        _ => {}
                    }
                }
                rules.protagonist = Some(p);
            }
            "genreLock" => {
                let mut g = GenreLock::default();
                for (k, v, _) in top_level_blocks(&child_block) {
                    match k.as_str() {
                        "primary" => g.primary = unquote_scalar(&v).unwrap_or_default(),
                        "forbidden" => g.forbidden = parse_list(&v, &child_block, "forbidden"),
                        _ => {}
                    }
                }
                rules.genre_lock = Some(g);
            }
            "numericalSystemOverrides" => {
                let mut n = NumericalSystemOverrides::default();
                for (k, v, _) in top_level_blocks(&child_block) {
                    match k.as_str() {
                        "hardCap" => n.hard_cap = unquote_scalar(&v),
                        "resourceTypes" => {
                            n.resource_types = parse_list(&v, &child_block, "resourceTypes");
                        }
                        _ => {}
                    }
                }
                rules.numerical_system_overrides = Some(n);
            }
            "eraConstraints" => {
                let mut e = EraConstraints::default();
                for (k, v, _) in top_level_blocks(&child_block) {
                    match k.as_str() {
                        "enabled" => e.enabled = parse_bool(&v),
                        "period" => e.period = unquote_scalar(&v),
                        "region" => e.region = unquote_scalar(&v),
                        _ => {}
                    }
                }
                rules.era_constraints = Some(e);
            }
            "prohibitions" => {
                rules.prohibitions = parse_list(&value_first_line, &child_block, "prohibitions");
            }
            "chapterTypesOverride" => {
                rules.chapter_types_override =
                    parse_list(&value_first_line, &child_block, "chapterTypesOverride");
            }
            "fatigueWordsOverride" => {
                rules.fatigue_words_override =
                    parse_list(&value_first_line, &child_block, "fatigueWordsOverride");
            }
            "additionalAuditDimensions" => {
                rules.additional_audit_dimensions =
                    parse_list(&value_first_line, &child_block, "additionalAuditDimensions");
            }
            "enableFullCastTracking" => {
                rules.enable_full_cast_tracking = parse_bool(&value_first_line);
            }
            "fanficMode" => {
                rules.fanfic_mode = unquote_scalar(&value_first_line);
            }
            "allowedDeviations" => {
                rules.allowed_deviations =
                    parse_list(&value_first_line, &child_block, "allowedDeviations");
            }
            _ => {}
        }
    }

    Some(rules)
}

/// 把 frontmatter 文本拆分成「顶层键 + 行内值 + 缩进子块」。
///
/// 子块仅包含原始缩进文本，留给下一层 `top_level_blocks` 递归解析。
fn top_level_blocks(text: &str) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_end();

        // 跳过空行、注释、行内只是文档分隔的行
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            i += 1;
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        if indent != 0 {
            // 该行属于上一个块的子内容（理论上不会出现在顶层调用，但稳妥跳过）
            i += 1;
            continue;
        }

        if let Some(colon) = trimmed.find(':') {
            let key = trimmed[..colon].trim().to_string();
            let rest = trimmed[colon + 1..].to_string();
            let value_first_line = rest.trim().to_string();

            // 收集后续缩进行作为子块
            let mut child_lines: Vec<String> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let l = lines[j];
                if l.trim().is_empty() {
                    child_lines.push(String::new());
                    j += 1;
                    continue;
                }
                let l_indent = l.len() - l.trim_start().len();
                if l_indent == 0 {
                    break;
                }
                // 去掉 2 个空格的最小缩进，便于递归解析
                let dedented = if l.starts_with("  ") { &l[2..] } else { l.trim_start() };
                child_lines.push(dedented.to_string());
                j += 1;
            }
            let child_block = child_lines.join("\n");
            out.push((key, value_first_line, child_block));
            i = j;
        } else {
            i += 1;
        }
    }

    out
}

fn unquote_scalar(v: &str) -> Option<String> {
    let t = v.trim();
    if t.is_empty() || t == "~" || t.eq_ignore_ascii_case("null") {
        return None;
    }
    let unq = if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
    {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    };
    if unq.is_empty() {
        None
    } else {
        Some(unq)
    }
}

fn parse_bool(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "on" | "1"
    )
}

/// 解析数组：支持 inline `[a, b, c]` 与 block style（- a / - b）。
fn parse_list(value_first_line: &str, child_block: &str, key: &str) -> Vec<String> {
    let v = value_first_line.trim();
    if v.starts_with('[') && v.ends_with(']') && v.len() >= 2 {
        return split_inline_list(&v[1..v.len() - 1]);
    }
    // block style：在 child_block 里找 `- xxx`
    let mut out = Vec::new();
    for line in child_block.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("- ") {
            if let Some(s) = unquote_scalar(rest) {
                out.push(s);
            }
        } else if t == "-" {
            // 空项跳过
        }
    }
    let _ = key; // key 仅供调试预留
    out
}

fn split_inline_list(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut buf = String::new();
    let mut in_quote: Option<char> = None;
    let mut depth: i32 = 0;
    for ch in s.chars() {
        match (ch, in_quote) {
            ('\'', None) => {
                in_quote = Some('\'');
                buf.push(ch);
            }
            ('"', None) => {
                in_quote = Some('"');
                buf.push(ch);
            }
            (c, Some(q)) if c == q => {
                in_quote = None;
                buf.push(ch);
            }
            ('[', None) | ('{', None) => {
                depth += 1;
                buf.push(ch);
            }
            (']', None) | ('}', None) => {
                depth -= 1;
                buf.push(ch);
            }
            (',', None) if depth == 0 => {
                if let Some(s) = unquote_scalar(&buf) {
                    items.push(s);
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    if !buf.trim().is_empty() {
        if let Some(s) = unquote_scalar(&buf) {
            items.push(s);
        }
    }
    items
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

pub fn render_book_rules(parsed: &ParsedBookRules) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("version: {}\n", quote_scalar(&parsed.rules.version)));

    if let Some(p) = &parsed.rules.protagonist {
        out.push_str("protagonist:\n");
        out.push_str(&format!("  name: {}\n", quote_scalar(&p.name)));
        out.push_str(&format!(
            "  personalityLock: {}\n",
            render_inline_list(&p.personality_lock)
        ));
        out.push_str(&format!(
            "  behavioralConstraints: {}\n",
            render_inline_list(&p.behavioral_constraints)
        ));
    }

    if let Some(g) = &parsed.rules.genre_lock {
        out.push_str("genreLock:\n");
        out.push_str(&format!("  primary: {}\n", quote_scalar(&g.primary)));
        out.push_str(&format!(
            "  forbidden: {}\n",
            render_inline_list(&g.forbidden)
        ));
    }

    if let Some(n) = &parsed.rules.numerical_system_overrides {
        out.push_str("numericalSystemOverrides:\n");
        if let Some(c) = &n.hard_cap {
            out.push_str(&format!("  hardCap: {}\n", quote_scalar(c)));
        }
        out.push_str(&format!(
            "  resourceTypes: {}\n",
            render_inline_list(&n.resource_types)
        ));
    }

    if let Some(e) = &parsed.rules.era_constraints {
        out.push_str("eraConstraints:\n");
        out.push_str(&format!("  enabled: {}\n", e.enabled));
        if let Some(p) = &e.period {
            out.push_str(&format!("  period: {}\n", quote_scalar(p)));
        }
        if let Some(r) = &e.region {
            out.push_str(&format!("  region: {}\n", quote_scalar(r)));
        }
    }

    out.push_str(&format!(
        "prohibitions: {}\n",
        render_inline_list(&parsed.rules.prohibitions)
    ));
    out.push_str(&format!(
        "chapterTypesOverride: {}\n",
        render_inline_list(&parsed.rules.chapter_types_override)
    ));
    out.push_str(&format!(
        "fatigueWordsOverride: {}\n",
        render_inline_list(&parsed.rules.fatigue_words_override)
    ));
    out.push_str(&format!(
        "additionalAuditDimensions: {}\n",
        render_inline_list(&parsed.rules.additional_audit_dimensions)
    ));
    out.push_str(&format!(
        "enableFullCastTracking: {}\n",
        parsed.rules.enable_full_cast_tracking
    ));
    if let Some(m) = &parsed.rules.fanfic_mode {
        out.push_str(&format!("fanficMode: {}\n", quote_scalar(m)));
    }
    if !parsed.rules.allowed_deviations.is_empty() {
        out.push_str(&format!(
            "allowedDeviations: {}\n",
            render_inline_list(&parsed.rules.allowed_deviations)
        ));
    }
    out.push_str("---\n\n");
    out.push_str(parsed.body.trim_end());
    out.push('\n');
    out
}

fn quote_scalar(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_string();
    }
    let need_quote = s.contains(':')
        || s.contains('#')
        || s.starts_with('-')
        || s.starts_with('"')
        || s.starts_with('\'')
        || s.contains('\n')
        || matches!(s, "true" | "false" | "null" | "yes" | "no");
    if need_quote {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn render_inline_list(items: &[String]) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let parts: Vec<String> = items.iter().map(|s| quote_scalar(s)).collect();
    format!("[{}]", parts.join(", "))
}

// ---------------------------------------------------------------------------
// Generation prompt（对齐 architect.bookRulesPrompt）
// ---------------------------------------------------------------------------

/// 生成 (system, user) 两条 prompt，输出**只**包含 `book_rules.md` 的内容
/// （YAML frontmatter + Markdown 正文，无 SECTION 标记、无代码围栏）。
///
/// 与 inkos `architect.ts` 中 `bookRulesPrompt` 对齐：
/// - 字段顺序：version → protagonist → genreLock → (numericalSystemOverrides) →
///   prohibitions → chapterTypesOverride → fatigueWordsOverride →
///   additionalAuditDimensions → enableFullCastTracking
/// - 数值/资源体系仅在题材/设定中显式涉及时输出 `numericalSystemOverrides`
/// - 同人模式下额外输出 `fanficMode` / `allowedDeviations`
/// - 末尾要求两段叙事指导：`## 叙事视角` / `## 核心冲突驱动`
pub fn build_generation_prompts(
    project: &NovelProject,
    fanfic_mode: Option<&str>,
    extra_user_brief: Option<&str>,
    state_context: Option<&str>,
) -> (String, String) {
    let title = pick(&project.title, "未命名");
    let genre = pick(&project.genre, "通用");
    let target_chapters = if project.target_chapters > 0 {
        project.target_chapters
    } else {
        100
    };
    let chapter_word_goal = if project.chapter_word_goal > 0 {
        project.chapter_word_goal
    } else {
        3000
    };

    // 是否输出 numericalSystemOverrides：依据 premise/world_setting/outline 中
    // 出现明显的数值/资源体系信号词来判定（修真、灵气、积分、积分卡、资源、储量、点数…）。
    let need_numerical = looks_like_numerical_system(project);
    let numerical_block = if need_numerical {
        "numericalSystemOverrides:\n  hardCap: (根据设定确定，可写数字或字符串)\n  resourceTypes: [(核心资源类型列表)]\n"
    } else {
        ""
    };

    // 同人模式
    let fanfic_block = if let Some(mode) = fanfic_mode {
        format!(
            "fanficMode: \"{}\"\nallowedDeviations: [(列出允许偏离的关键设定，3-5 条)]\n",
            mode
        )
    } else {
        String::new()
    };

    let book_rules_template = format!(
        r#"```
---
version: "1.0"
protagonist:
  name: (主角名)
  personalityLock: [(3-5个性格关键词)]
  behavioralConstraints: [(3-5条行为约束)]
genreLock:
  primary: {genre}
  forbidden: [(2-3种禁止混入的文风)]
{numerical_block}prohibitions:
  - (3-5条本书禁忌)
chapterTypesOverride: []
fatigueWordsOverride: []
additionalAuditDimensions: []
enableFullCastTracking: false
{fanfic_block}---

## 叙事视角
(描述本书叙事视角和风格)

## 核心冲突驱动
(描述本书的核心矛盾和驱动力)
```"#
    );

    let system_prompt = format!(
        r#"你是一个专业的网络小说架构师，正在为一本 {genre} 网文生成 `book_rules.md`。

book_rules.md 是本书的"硬约束档案"：约束主角人格 / 行为边界、题材锁定、禁忌、附加审计维度等，
后续所有写作 / 审计 / 改写都会以它为底线，因此**必须可被严格 YAML 解析**。

## 输出格式（严格遵守）
- 第一行必须是 `---`，YAML frontmatter 与 Markdown 正文之间必须有 `---` 分隔；
- 不要输出 `=== SECTION: ===` 之类的标记；
- 不要用 ```` ``` ```` 把整段输出包裹起来（你只输出文件内容本身）；
- 字段名与下方模板**完全一致**（驼峰拼写）；
- 数组优先使用 inline 风格 `[a, b, c]`，每个元素必须是完整短语；
- `enableFullCastTracking` 只能是 `true` / `false`；
- `genreLock.primary` 必须等于 `{genre}`，不要翻译；
- 仅当确实涉及数值/资源体系时才输出 `numericalSystemOverrides`；
- 仅当本书属于同人时才输出 `fanficMode` / `allowedDeviations`。

## 模板（按此结构填充，括号注释替换为真实内容）
{book_rules_template}

## 业务约束
1. `personalityLock` 与 `behavioralConstraints` 必须互相印证：行为约束应当能"反推"出锁定的性格；
2. `prohibitions` 必须是**本书特有禁忌**，不要写"不抄袭/不政治敏感"这类通用条款；
3. `genreLock.forbidden` 写出 2-3 种**容易混入但本书绝对禁止**的文风（如玄幻文禁止穿越腔、都市文禁止修真术语）；
4. `## 叙事视角` 限 1-3 句，明确人称 / 视角 / 叙述距离 / 文风基调；
5. `## 核心冲突驱动` 限 2-4 句，写出贯穿全书的核心矛盾与驱动力，不要剧透具体桥段。"#
    );

    let mut user_msg = format!(
        r#"请为下列作品生成 `book_rules.md`，**只输出文件内容本身**（YAML frontmatter + Markdown 正文）：

- 书名：{title}
- 题材：{genre}
- 目标章数：{target_chapters} 章
- 单章目标字数：{chapter_word_goal} 字"#
    );
    push_kv(&mut user_msg, "一句话设定", &project.premise);
    push_kv(&mut user_msg, "主角与关键角色", &project.protagonists);
    push_kv(&mut user_msg, "世界观", &project.world_setting);
    push_kv(&mut user_msg, "文风基调", &project.writing_style);
    push_kv(&mut user_msg, "大纲 / 卷纲", &project.outline);
    push_kv(&mut user_msg, "额外指引", &project.extra_guidance);
    if let Some(ctx) = state_context {
        let trimmed = ctx.trim();
        if !trimmed.is_empty() {
            user_msg.push_str("\n\n## 长期记忆参考\n");
            user_msg.push_str(trimmed);
        }
    }
    if let Some(brief) = extra_user_brief {
        let t = brief.trim();
        if !t.is_empty() {
            user_msg.push_str("\n\n## 本次额外指令\n");
            user_msg.push_str(t);
        }
    }

    (system_prompt, user_msg)
}

fn pick<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    let t = s.trim();
    if t.is_empty() {
        fallback
    } else {
        t
    }
}

fn push_kv(buf: &mut String, label: &str, value: &str) {
    let v = value.trim();
    if v.is_empty() {
        return;
    }
    buf.push_str("\n\n## ");
    buf.push_str(label);
    buf.push('\n');
    buf.push_str(v);
}

fn looks_like_numerical_system(project: &NovelProject) -> bool {
    let hay = format!(
        "{} {} {} {} {}",
        project.genre,
        project.premise,
        project.world_setting,
        project.outline,
        project.extra_guidance
    )
    .to_lowercase();
    const NEEDLES: &[&str] = &[
        "修真", "修仙", "玄幻", "灵气", "灵力", "等级", "境界", "积分", "属性", "技能点",
        "战力", "资源", "储量", "点数", "经验值", "金币", "灵石", "符文",
        "level", "exp", "mana", "rune", "skill point",
    ];
    NEEDLES.iter().any(|n| hay.contains(n))
}

/// 在已有内容里裁掉 LLM 经常加上的"我已经为你生成…"前后缀，提取真正的文件内容。
///
/// 处理三类常见包裹：
/// 1. 整段被 ```` ``` ```` 包裹（含 ```` ```md ```` / ```` ```yaml ````）；
/// 2. 真正内容前有一段「好的，已为你生成…」的前言；
/// 3. 真正内容后有一段「以上就是…」的解释。
///
/// 与 `parse_book_rules` 不同：本函数返回的是**字符串**，可直接写入磁盘。
pub fn parse_generation_output(raw: &str) -> String {
    let mut text = strip_code_fence(raw).trim().to_string();

    // 若文本中**任意位置**有内嵌的 ```` ``` ```` 围栏，截取第一个围栏内的内容。
    if let Some(start) = text.find("```") {
        let after = &text[start..];
        let body_start = after
            .find('\n')
            .map(|n| start + n + 1)
            .unwrap_or(start + 3);
        let rest = &text[body_start..];
        if let Some(end_off) = rest.find("```") {
            text = rest[..end_off].trim_end().to_string();
        }
    }

    // 找到第一行起头是 `---` 的位置，把之前的前言全部丢掉。
    let mut frontmatter_start: Option<usize> = None;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        if line.trim_start().starts_with("---") && line.trim() == "---" {
            frontmatter_start = Some(offset);
            break;
        }
        offset += line.len();
    }
    if let Some(start) = frontmatter_start {
        return text[start..].trim().to_string();
    }
    text.trim().to_string()
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_frontmatter() {
        let raw = r#"---
version: "1.0"
protagonist:
  name: 苏寻
  personalityLock: [冷静, 务实, 寡言]
  behavioralConstraints:
    - 不主动伤害无辜
    - 不公开真实身份
genreLock:
  primary: 都市异能
  forbidden: [修真腔, 言情腔]
prohibitions:
  - 不写主角金手指来源
chapterTypesOverride: []
fatigueWordsOverride: ["突然", "瞬间"]
additionalAuditDimensions: [12, "节奏失衡"]
enableFullCastTracking: true
fanficMode: "au"
allowedDeviations: [角色性别可换]
---

## 叙事视角
冷静第三人称。

## 核心冲突驱动
都市秩序与异能黑市的对撞。
"#;
        let parsed = parse_book_rules(raw);
        assert_eq!(parsed.rules.version, "1.0");
        let p = parsed.rules.protagonist.unwrap();
        assert_eq!(p.name, "苏寻");
        assert_eq!(p.personality_lock, vec!["冷静", "务实", "寡言"]);
        assert_eq!(p.behavioral_constraints.len(), 2);
        let g = parsed.rules.genre_lock.unwrap();
        assert_eq!(g.primary, "都市异能");
        assert_eq!(g.forbidden, vec!["修真腔", "言情腔"]);
        assert_eq!(parsed.rules.prohibitions.len(), 1);
        assert_eq!(parsed.rules.fatigue_words_override, vec!["突然", "瞬间"]);
        assert_eq!(
            parsed.rules.additional_audit_dimensions,
            vec!["12".to_string(), "节奏失衡".to_string()]
        );
        assert!(parsed.rules.enable_full_cast_tracking);
        assert_eq!(parsed.rules.fanfic_mode.as_deref(), Some("au"));
        assert_eq!(parsed.rules.allowed_deviations, vec!["角色性别可换"]);
        assert!(parsed.body.contains("叙事视角"));
        assert!(parsed.body.contains("核心冲突驱动"));
    }

    #[test]
    fn parse_strips_codeblock_wrapper() {
        let raw = "```md\n---\nversion: \"1.0\"\nprohibitions:\n  - 单条\n---\n\n## 叙事视角\n第三人称\n```";
        let parsed = parse_book_rules(raw);
        assert_eq!(parsed.rules.version, "1.0");
        assert_eq!(parsed.rules.prohibitions, vec!["单条"]);
        assert!(parsed.body.contains("第三人称"));
    }

    #[test]
    fn parse_falls_back_on_missing_frontmatter() {
        let raw = "## 叙事视角\n第三人称";
        let parsed = parse_book_rules(raw);
        assert_eq!(parsed.rules.version, "1.0");
        assert!(parsed.rules.protagonist.is_none());
        assert!(parsed.body.contains("第三人称"));
    }

    #[test]
    fn render_roundtrip_preserves_essentials() {
        let raw = r#"---
version: "1.0"
protagonist:
  name: 苏寻
  personalityLock: [冷静, 务实]
  behavioralConstraints: [不主动伤害无辜]
genreLock:
  primary: 都市异能
  forbidden: [修真腔]
prohibitions: [不剧透金手指来源]
chapterTypesOverride: []
fatigueWordsOverride: []
additionalAuditDimensions: []
enableFullCastTracking: false
---

## 叙事视角
冷静第三人称。
"#;
        let parsed = parse_book_rules(raw);
        let rendered = render_book_rules(&parsed);
        let reparsed = parse_book_rules(&rendered);
        assert_eq!(reparsed.rules.protagonist.unwrap().name, "苏寻");
        assert_eq!(reparsed.rules.genre_lock.unwrap().primary, "都市异能");
        assert_eq!(reparsed.rules.prohibitions, vec!["不剧透金手指来源"]);
        assert!(reparsed.body.contains("叙事视角"));
    }

    #[test]
    fn build_prompts_inject_genre_and_meta() {
        let mut p = NovelProject::default();
        p.title = "破阵".into();
        p.genre = "都市异能".into();
        p.premise = "退役兵王进入异能黑市".into();
        p.target_chapters = 200;
        p.chapter_word_goal = 2800;
        let (sys, user) = build_generation_prompts(&p, None, None, None);
        assert!(sys.contains("都市异能"));
        assert!(sys.contains("`book_rules.md`"));
        assert!(user.contains("破阵"));
        assert!(user.contains("200 章"));
        assert!(user.contains("2800 字"));
        // 都市异能不命中 numerical system 关键字 —— 模板里不应出现
        // `numericalSystemOverrides:` 这一行（说明文里出现的 `numericalSystemOverrides`
        // 是叙述，不是字段）。
        assert!(!sys.contains("numericalSystemOverrides:\n  hardCap"));
    }

    #[test]
    fn build_prompts_enable_numerical_for_xianxia() {
        let mut p = NovelProject::default();
        p.title = "霜河传".into();
        p.genre = "修真".into();
        p.premise = "灵气复苏后的现代修真".into();
        p.world_setting = "等级体系：练气 → 筑基 → 金丹".into();
        let (sys, _user) = build_generation_prompts(&p, None, None, None);
        assert!(sys.contains("numericalSystemOverrides:\n  hardCap"));
    }

    #[test]
    fn build_prompts_inject_fanfic_mode() {
        let p = NovelProject::default();
        let (sys, _) = build_generation_prompts(&p, Some("au"), None, None);
        assert!(sys.contains("fanficMode"));
        assert!(sys.contains("\"au\""));
    }

    #[test]
    fn parse_generation_output_strips_preamble() {
        let raw = "好的，已为你生成：\n\n```\n---\nversion: \"1.0\"\nprohibitions: [a]\n---\n\n## 叙事视角\n第三人称\n```";
        let cleaned = parse_generation_output(raw);
        assert!(cleaned.starts_with("---"), "got: {cleaned}");
        let parsed = parse_book_rules(&cleaned);
        assert_eq!(parsed.rules.prohibitions, vec!["a"]);
    }
}
