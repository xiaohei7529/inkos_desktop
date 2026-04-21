//! 写作管线 per-chapter runtime artifacts：
//!
//! - `story/runtime/chapter-XXXX.intent.md`     本章意图（plan 阶段产出）
//! - `story/runtime/chapter-XXXX.context.json`  prompt 上下文 / vendor / model / token 估算
//! - `story/runtime/chapter-XXXX.rule-stack.yaml` 本章命中的 rule 与 governance 注入
//! - `story/runtime/chapter-XXXX.trace.json`    各阶段时序、耗时、字符增量
//!
//! 灵感来源：Narcooo/inkos `packages/core/runtime`。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub fn runtime_dir(novel_root: &Path) -> PathBuf {
    novel_root.join("story").join("runtime")
}

pub fn chapter_stem(novel_root: &Path, chapter_no: i32) -> PathBuf {
    runtime_dir(novel_root).join(format!("chapter-{chapter_no:04}"))
}

pub fn intent_path(novel_root: &Path, chapter_no: i32) -> PathBuf {
    chapter_stem(novel_root, chapter_no).with_extension("intent.md")
}

pub fn context_path(novel_root: &Path, chapter_no: i32) -> PathBuf {
    chapter_stem(novel_root, chapter_no).with_extension("context.json")
}

pub fn rule_stack_path(novel_root: &Path, chapter_no: i32) -> PathBuf {
    chapter_stem(novel_root, chapter_no).with_extension("rule-stack.yaml")
}

pub fn trace_path(novel_root: &Path, chapter_no: i32) -> PathBuf {
    chapter_stem(novel_root, chapter_no).with_extension("trace.json")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineContext {
    pub vendor: String,
    pub model: String,
    pub temperature: String,
    pub max_tokens: String,
    pub word_goal: i32,
    pub word_tolerance: i32,
    pub state_files: Vec<String>,
    pub author_intent_chars: usize,
    pub current_focus_chars: usize,
    pub style_fingerprint_loaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuleStack {
    pub from_book_rules: Vec<String>,
    pub from_author_intent: Vec<String>,
    pub from_current_focus: Vec<String>,
    pub from_style_fingerprint: Vec<String>,
    pub effective_max_words: i32,
    pub effective_min_words: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineTraceEntry {
    pub stage: String,
    pub started_at: String,
    pub ended_at: String,
    pub elapsed_secs: f64,
    pub vendor: String,
    pub model: String,
    pub chars: usize,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineTrace {
    pub chapter_no: i32,
    pub entries: Vec<PipelineTraceEntry>,
}

pub fn ensure_runtime_dir(novel_root: &Path) -> Result<()> {
    fs::create_dir_all(runtime_dir(novel_root))
        .with_context(|| format!("create_dir_all {:?}", runtime_dir(novel_root)))?;
    Ok(())
}

pub fn write_intent(novel_root: &Path, chapter_no: i32, body: &str) -> Result<()> {
    ensure_runtime_dir(novel_root)?;
    fs::write(intent_path(novel_root, chapter_no), body)?;
    Ok(())
}

pub fn read_intent(novel_root: &Path, chapter_no: i32) -> String {
    fs::read_to_string(intent_path(novel_root, chapter_no)).unwrap_or_default()
}

pub fn write_context(novel_root: &Path, chapter_no: i32, ctx: &PipelineContext) -> Result<()> {
    ensure_runtime_dir(novel_root)?;
    let txt = serde_json::to_string_pretty(ctx)?;
    fs::write(context_path(novel_root, chapter_no), txt)?;
    Ok(())
}

pub fn read_context(novel_root: &Path, chapter_no: i32) -> Option<PipelineContext> {
    let raw = fs::read_to_string(context_path(novel_root, chapter_no)).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_rule_stack(novel_root: &Path, chapter_no: i32, rs: &RuleStack) -> Result<()> {
    ensure_runtime_dir(novel_root)?;
    let yaml = rule_stack_to_yaml(rs);
    fs::write(rule_stack_path(novel_root, chapter_no), yaml)?;
    Ok(())
}

pub fn read_rule_stack_text(novel_root: &Path, chapter_no: i32) -> String {
    fs::read_to_string(rule_stack_path(novel_root, chapter_no)).unwrap_or_default()
}

pub fn append_trace(novel_root: &Path, chapter_no: i32, entry: PipelineTraceEntry) -> Result<()> {
    ensure_runtime_dir(novel_root)?;
    let path = trace_path(novel_root, chapter_no);
    let mut trace: PipelineTrace = if let Ok(raw) = fs::read_to_string(&path) {
        serde_json::from_str(&raw).unwrap_or(PipelineTrace {
            chapter_no,
            entries: Vec::new(),
        })
    } else {
        PipelineTrace {
            chapter_no,
            entries: Vec::new(),
        }
    };
    trace.chapter_no = chapter_no;
    trace.entries.push(entry);
    fs::write(&path, serde_json::to_string_pretty(&trace)?)?;
    Ok(())
}

pub fn read_trace(novel_root: &Path, chapter_no: i32) -> Option<PipelineTrace> {
    let raw = fs::read_to_string(trace_path(novel_root, chapter_no)).ok()?;
    serde_json::from_str(&raw).ok()
}

/// 极简 YAML 序列化，避免引入 serde_yaml。
fn rule_stack_to_yaml(rs: &RuleStack) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "effective_min_words: {}\neffective_max_words: {}\n",
        rs.effective_min_words, rs.effective_max_words
    ));
    let push_list = |out: &mut String, key: &str, items: &[String]| {
        out.push_str(&format!("{key}:\n"));
        if items.is_empty() {
            out.push_str("  []\n");
        } else {
            for it in items {
                out.push_str(&format!("  - {}\n", yaml_escape(it)));
            }
        }
    };
    push_list(&mut out, "from_book_rules", &rs.from_book_rules);
    push_list(&mut out, "from_author_intent", &rs.from_author_intent);
    push_list(&mut out, "from_current_focus", &rs.from_current_focus);
    push_list(
        &mut out,
        "from_style_fingerprint",
        &rs.from_style_fingerprint,
    );
    out
}

fn yaml_escape(s: &str) -> String {
    if s.contains(':') || s.contains('#') || s.starts_with('-') || s.contains('\n') {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}
