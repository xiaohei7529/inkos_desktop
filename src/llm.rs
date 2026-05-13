//! OpenAI 兼容 chat 客户端，支持流式（SSE）；后台线程通过事件向主线程推送增量。

use std::io::{BufRead, BufReader};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::VendorConfig;

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(s: impl Into<String>) -> Self {
        Self { role: "system".into(), content: s.into() }
    }
    pub fn user(s: impl Into<String>) -> Self {
        Self { role: "user".into(), content: s.into() }
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: s.into() }
    }
}

#[derive(Debug)]
pub enum LlmEvent {
    Delta(String),
    Done,
    Error(String),
}

pub struct LlmTask {
    rx: Receiver<LlmEvent>,
    pub accumulated: String,
    pub done: bool,
    pub error: Option<String>,
    #[allow(dead_code)]
    pub vendor_id: String,
    #[allow(dead_code)]
    pub model: String,
    pub streaming: bool,
    pub started_at: Instant,
    pub elapsed_when_done: Option<Duration>,
}

impl LlmTask {
    /// 拉取所有可用事件，返回是否有更新（用于驱动 repaint）。
    pub fn drain(&mut self) -> bool {
        let mut updated = false;
        loop {
            match self.rx.try_recv() {
                Ok(ev) => {
                    updated = true;
                    match ev {
                        LlmEvent::Delta(s) => self.accumulated.push_str(&s),
                        LlmEvent::Done => {
                            if !self.done {
                                self.elapsed_when_done = Some(self.started_at.elapsed());
                            }
                            self.done = true;
                        }
                        LlmEvent::Error(e) => {
                            if !self.done {
                                self.elapsed_when_done = Some(self.started_at.elapsed());
                            }
                            self.error = Some(e);
                            self.done = true;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        updated
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.elapsed_when_done
            .unwrap_or_else(|| self.started_at.elapsed())
            .as_secs_f64()
    }

    pub fn char_count(&self) -> usize {
        self.accumulated.chars().count()
    }

    pub fn chars_per_sec(&self) -> f64 {
        let s = self.elapsed_secs().max(0.001);
        self.char_count() as f64 / s
    }

    /// 简短的统计字串，用于在 UI 中显示流式速率。
    pub fn stats_label(&self) -> String {
        let chars = self.char_count();
        let secs = self.elapsed_secs();
        let rate = self.chars_per_sec();
        if self.done {
            format!("{chars} 字 · {:.1}s · {rate:.1} 字/秒", secs)
        } else if self.streaming {
            format!("{chars} 字 · {:.1}s · {rate:.1} 字/秒 · 流式中…", secs)
        } else {
            format!("等待响应 · {:.1}s", secs)
        }
    }
}

#[derive(Serialize)]
struct ReqMsg<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: Vec<ReqMsg<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: RespMsg,
}

#[derive(Deserialize, Default)]
struct RespMsg {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: String,
}

pub fn spawn_chat(
    cfg: VendorConfig,
    vendor_id: String,
    model: String,
    messages: Vec<ChatMessage>,
    streaming: bool,
) -> LlmTask {
    let (tx, rx) = mpsc::channel();
    let v_id = vendor_id.clone();
    let m_id = model.clone();
    thread::spawn(move || {
        if streaming {
            if let Err(stream_err) = stream_chat(&cfg, &model, &messages, &tx) {
                match call_chat(&cfg, &model, &messages) {
                    Ok(c) => {
                        let _ = tx.send(LlmEvent::Delta(c));
                        let _ = tx.send(LlmEvent::Done);
                    }
                    Err(non_stream_err) => {
                        let _ = tx.send(LlmEvent::Error(format!(
                            "{stream_err}；非流式兜底失败：{non_stream_err}"
                        )));
                    }
                }
            } else {
                let _ = tx.send(LlmEvent::Done);
            }
        } else {
            match call_chat(&cfg, &model, &messages) {
                Ok(c) => {
                    let _ = tx.send(LlmEvent::Delta(c));
                    let _ = tx.send(LlmEvent::Done);
                }
                Err(e) => {
                    let _ = tx.send(LlmEvent::Error(e));
                }
            }
        }
    });
    LlmTask {
        rx,
        accumulated: String::new(),
        done: false,
        error: None,
        vendor_id: v_id,
        model: m_id,
        streaming,
        started_at: Instant::now(),
        elapsed_when_done: None,
    }
}

/// 测试连接：发送最小 chat 请求验证 base_url + api_key + model 是否可用。
pub fn spawn_ping(cfg: VendorConfig, model: String) -> LlmTask {
    let messages = vec![ChatMessage::user("请回复一个字 ok。")];
    spawn_chat(cfg, "test".into(), model, messages, false)
}

/// 后台拉取 OpenAI 兼容 `GET /v1/models` 列表（在 UI 线程外阻塞网络）。
pub struct ModelsFetchTask {
    rx: Receiver<Result<Vec<String>, String>>,
    pub done: bool,
    pub models: Vec<String>,
    pub error: Option<String>,
}

impl ModelsFetchTask {
    pub fn drain(&mut self) -> bool {
        if self.done {
            return false;
        }
        match self.rx.try_recv() {
            Ok(Ok(list)) => {
                self.models = list;
                self.done = true;
                true
            }
            Ok(Err(e)) => {
                self.error = Some(e);
                self.done = true;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.error = Some("请求中断".into());
                self.done = true;
                true
            }
        }
    }
}

pub fn spawn_fetch_models(cfg: VendorConfig) -> ModelsFetchTask {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let r = fetch_models_list(&cfg);
        let _ = tx.send(r);
    });
    ModelsFetchTask {
        rx,
        done: false,
        models: Vec::new(),
        error: None,
    }
}

/// `GET {base_url}/models`，`Authorization: Bearer {api_key}`（与 chat 一致：Key 为空则不带头）。
fn fetch_models_list(cfg: &VendorConfig) -> Result<Vec<String>, String> {
    let base = cfg.base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("Base URL 未设置".into());
    }
    let url = format!("{base}/models");
    let mut last_err = String::new();
    let mut resp = None;
    for attempt in 0..=2 {
        let agent = build_agent();
        let mut req = agent.get(&url);
        let key = cfg.api_key.trim();
        if !key.is_empty() {
            req = req.set("Authorization", &format!("Bearer {key}"));
        }
        match req.call() {
            Ok(r) => {
                resp = Some(r);
                break;
            }
            Err(ureq::Error::Status(code, body)) => {
                let txt = body.into_string().unwrap_or_default();
                return Err(format!("HTTP {code}: {}", truncate(&txt, 600)));
            }
            Err(e) => {
                let msg = e.to_string();
                if attempt < 2 && retryable_error(&msg) {
                    thread::sleep(Duration::from_millis(700 * (attempt + 1) as u64));
                    continue;
                }
                last_err = classify_error(&msg);
                break;
            }
        }
    }
    let Some(resp) = resp else {
        return Err(last_err);
    };
    let txt = resp
        .into_string()
        .map_err(|e| format!("读取响应失败：{e}"))?;
    parse_models_json(&txt)
}

fn parse_models_json(txt: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(txt).map_err(|e| format!("JSON 解析失败：{e}"))?;

    if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
        let mut ids: Vec<String> = data
            .iter()
            .filter_map(|item| item.get("id").and_then(|x| x.as_str()).map(str::to_string))
            .collect();
        if !ids.is_empty() {
            ids.sort();
            ids.dedup();
            return Ok(ids);
        }
    }

    if let Some(data) = v.get("models").and_then(|d| d.as_array()) {
        let mut ids: Vec<String> = Vec::new();
        for item in data {
            if let Some(id) = item.get("id").and_then(|x| x.as_str()) {
                ids.push(id.to_string());
            } else if let Some(name) = item.get("name").and_then(|x| x.as_str()) {
                ids.push(name.to_string());
            }
        }
        if !ids.is_empty() {
            ids.sort();
            ids.dedup();
            return Ok(ids);
        }
    }

    Err("响应中未找到模型列表（期望 `data[].id` 或 `models[]` 含 id/name）".into())
}

fn build_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(180))
        .build()
}

fn retryable_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("os error 10060")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
}

fn classify_error(err: &str) -> String {
    let lower = err.to_ascii_lowercase();
    if lower.contains("os error 10060") || lower.contains("timed out") || lower.contains("timeout") {
        format!("连接超时（10060）：{err}")
    } else if lower.contains("connection reset") {
        format!("连接中断：{err}")
    } else {
        format!("请求失败：{err}")
    }
}

fn build_request<'a>(
    agent: &'a ureq::Agent,
    cfg: &VendorConfig,
) -> Result<ureq::Request, String> {
    let base = cfg.base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("Base URL 未设置".into());
    }
    let url = format!("{base}/chat/completions");
    let mut req = agent.post(&url).set("Content-Type", "application/json");
    let key = cfg.api_key.trim();
    if !key.is_empty() {
        req = req.set("Authorization", &format!("Bearer {key}"));
    }
    Ok(req)
}

fn build_body<'a>(
    cfg: &'a VendorConfig,
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
) -> ChatReq<'a> {
    ChatReq {
        model,
        messages: messages
            .iter()
            .map(|m| ReqMsg { role: &m.role, content: &m.content })
            .collect(),
        temperature: cfg.temperature.trim().parse::<f32>().ok(),
        max_tokens: cfg.max_tokens.trim().parse::<u32>().ok(),
        stream,
    }
}

fn call_chat(cfg: &VendorConfig, model: &str, messages: &[ChatMessage]) -> Result<String, String> {
    if model.trim().is_empty() {
        return Err("Model 未设置".into());
    }
    let body = build_body(cfg, model, messages, false);
    let mut last_err = String::new();
    let mut resp = None;
    for attempt in 0..=2 {
        let agent = build_agent();
        let req = build_request(&agent, cfg)?;
        match req.send_json(&body) {
            Ok(r) => {
                resp = Some(r);
                break;
            }
            Err(ureq::Error::Status(code, body)) => {
                let txt = body.into_string().unwrap_or_default();
                return Err(format!("HTTP {code}: {}", truncate(&txt, 600)));
            }
            Err(e) => {
                let msg = e.to_string();
                if attempt < 2 && retryable_error(&msg) {
                    thread::sleep(Duration::from_millis(700 * (attempt + 1) as u64));
                    continue;
                }
                last_err = classify_error(&msg);
                break;
            }
        }
    }
    let Some(resp) = resp else {
        return Err(last_err);
    };

    let data: ChatResp = resp.into_json().map_err(|e| format!("解析响应失败：{e}"))?;
    let content = data
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .unwrap_or_default();
    if content.trim().is_empty() {
        Err("响应为空".into())
    } else {
        Ok(content)
    }
}

fn stream_chat(
    cfg: &VendorConfig,
    model: &str,
    messages: &[ChatMessage],
    tx: &std::sync::mpsc::Sender<LlmEvent>,
) -> Result<(), String> {
    if model.trim().is_empty() {
        return Err("Model 未设置".into());
    }
    let body = build_body(cfg, model, messages, true);
    let mut last_err = String::new();
    let mut resp = None;
    for attempt in 0..=1 {
        let agent = build_agent();
        let req = build_request(&agent, cfg)?.set("Accept", "text/event-stream");
        match req.send_json(&body) {
            Ok(r) => {
                resp = Some(r);
                break;
            }
            Err(ureq::Error::Status(code, body)) => {
                let txt = body.into_string().unwrap_or_default();
                return Err(format!("HTTP {code}: {}", truncate(&txt, 600)));
            }
            Err(e) => {
                let msg = e.to_string();
                if attempt < 1 && retryable_error(&msg) {
                    thread::sleep(Duration::from_millis(800));
                    continue;
                }
                last_err = classify_error(&msg);
                break;
            }
        }
    }
    let Some(resp) = resp else {
        return Err(last_err);
    };

    let reader = BufReader::new(resp.into_reader());
    let mut got_any = false;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => return Err(format!("读取流失败：{e}")),
        };
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let data = if let Some(rest) = trimmed.strip_prefix("data:") {
            rest.trim()
        } else {
            // 兼容某些服务商直接返回 JSON 行（非标准 SSE）
            trimmed
        };
        if data == "[DONE]" {
            break;
        }
        match serde_json::from_str::<StreamChunk>(data) {
            Ok(chunk) => {
                if let Some(c) = chunk.choices.into_iter().next() {
                    if !c.delta.content.is_empty() {
                        got_any = true;
                        let _ = tx.send(LlmEvent::Delta(c.delta.content));
                    }
                }
            }
            Err(_) => {
                // 跳过 keep-alive 或非 JSON 行
                continue;
            }
        }
    }
    if !got_any {
        return Err("流式响应为空".into());
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push('…');
    out
}
