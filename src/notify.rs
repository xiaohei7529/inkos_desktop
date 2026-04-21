//! 通知通道：Telegram、飞书、企业微信、Webhook。
//!
//! 灵感来源：Narcooo/inkos `inkos up` 守护进程的通知钩子。
//!
//! 通知内容由调用方拼装；本模块只负责发送和把结果回写日志。

use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use ureq::AgentBuilder;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NotifyConfig {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub feishu: FeishuConfig,
    #[serde(default)]
    pub wecom: WeComConfig,
    #[serde(default)]
    pub webhook: WebhookConfig,
    #[serde(default)]
    pub on_chapter_done: bool,
    #[serde(default)]
    pub on_audit_done: bool,
    #[serde(default)]
    pub on_error: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub chat_id: String,
}
impl TelegramConfig {
    pub fn enabled(&self) -> bool {
        !self.bot_token.trim().is_empty() && !self.chat_id.trim().is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeishuConfig {
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default)]
    pub secret: String,
}
impl FeishuConfig {
    pub fn enabled(&self) -> bool {
        !self.webhook_url.trim().is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WeComConfig {
    #[serde(default)]
    pub webhook_url: String,
}
impl WeComConfig {
    pub fn enabled(&self) -> bool {
        !self.webhook_url.trim().is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub method: String, // POST | GET
}
impl WebhookConfig {
    pub fn enabled(&self) -> bool {
        !self.url.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NotifyEvent {
    pub kind: String, // chapter_done | audit_done | error
    pub title: String,
    pub body: String,
    pub novel: String,
    pub chapter_no: Option<i32>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NotifyResult {
    pub channel: String,
    pub ok: bool,
    pub note: String,
}

pub fn notify_all(cfg: &NotifyConfig, evt: &NotifyEvent) -> Vec<NotifyResult> {
    let mut out = Vec::new();
    let allowed = match evt.kind.as_str() {
        "chapter_done" => cfg.on_chapter_done,
        "audit_done" => cfg.on_audit_done,
        "error" => cfg.on_error,
        _ => true,
    };
    if !allowed {
        out.push(NotifyResult {
            channel: "（按事件过滤）".into(),
            ok: true,
            note: format!("事件 {} 未启用通知", evt.kind),
        });
        return out;
    }
    if cfg.telegram.enabled() {
        out.push(send_telegram(&cfg.telegram, evt));
    }
    if cfg.feishu.enabled() {
        out.push(send_feishu(&cfg.feishu, evt));
    }
    if cfg.wecom.enabled() {
        out.push(send_wecom(&cfg.wecom, evt));
    }
    if cfg.webhook.enabled() {
        out.push(send_webhook(&cfg.webhook, evt));
    }
    if out.is_empty() {
        out.push(NotifyResult {
            channel: "（无）".into(),
            ok: false,
            note: "未配置任何通道".into(),
        });
    }
    out
}

fn agent() -> ureq::Agent {
    AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent("inkos-desktop/0.1")
        .build()
}

fn send_telegram(cfg: &TelegramConfig, evt: &NotifyEvent) -> NotifyResult {
    let url = format!(
        "https://api.telegram.org/bot{}/sendMessage",
        cfg.bot_token.trim()
    );
    let text = format!("[{}] {}\n\n{}", evt.kind, evt.title, evt.body);
    let body = serde_json::json!({
        "chat_id": cfg.chat_id.trim(),
        "text": text,
        "parse_mode": "Markdown",
    });
    match agent().post(&url).send_json(body) {
        Ok(_) => NotifyResult {
            channel: "Telegram".into(),
            ok: true,
            note: "已发送".into(),
        },
        Err(e) => NotifyResult {
            channel: "Telegram".into(),
            ok: false,
            note: e.to_string(),
        },
    }
}

fn send_feishu(cfg: &FeishuConfig, evt: &NotifyEvent) -> NotifyResult {
    use sha2::Sha256;
    use hmac::{Hmac, Mac};
    type HmacSha256 = Hmac<Sha256>;

    let mut payload = serde_json::json!({
        "msg_type": "interactive",
        "card": {
            "header": {
                "title": { "tag": "plain_text", "content": format!("InkOS · {}", evt.title) }
            },
            "elements": [
                { "tag": "div", "text": { "tag": "lark_md", "content": format!("**事件**：{}\n**小说**：{}\n**章节**：{}\n\n{}", evt.kind, evt.novel, evt.chapter_no.map(|n| n.to_string()).unwrap_or_else(|| "-".into()), evt.body) } }
            ]
        }
    });

    if !cfg.secret.trim().is_empty() {
        let ts = chrono::Local::now().timestamp().to_string();
        let to_sign = format!("{ts}\n{}", cfg.secret.trim());
        let Ok(mut mac) = HmacSha256::new_from_slice(to_sign.as_bytes()) else {
            return NotifyResult {
                channel: "飞书".into(),
                ok: false,
                note: "HMAC 初始化失败".into(),
            };
        };
        mac.update(b"");
        let sig = base64_encode(&mac.finalize().into_bytes());
        if let serde_json::Value::Object(ref mut m) = payload {
            m.insert("timestamp".into(), serde_json::json!(ts));
            m.insert("sign".into(), serde_json::json!(sig));
        }
    }

    match agent().post(cfg.webhook_url.trim()).send_json(payload) {
        Ok(_) => NotifyResult {
            channel: "飞书".into(),
            ok: true,
            note: "已发送".into(),
        },
        Err(e) => NotifyResult {
            channel: "飞书".into(),
            ok: false,
            note: e.to_string(),
        },
    }
}

fn send_wecom(cfg: &WeComConfig, evt: &NotifyEvent) -> NotifyResult {
    let body = serde_json::json!({
        "msgtype": "markdown",
        "markdown": {
            "content": format!(
                "# InkOS · {title}\n> 事件：{kind}\n> 小说：{novel}\n> 章节：{ch}\n\n{body}",
                title = evt.title,
                kind = evt.kind,
                novel = evt.novel,
                ch = evt.chapter_no.map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
                body = evt.body
            )
        }
    });
    match agent().post(cfg.webhook_url.trim()).send_json(body) {
        Ok(_) => NotifyResult {
            channel: "企业微信".into(),
            ok: true,
            note: "已发送".into(),
        },
        Err(e) => NotifyResult {
            channel: "企业微信".into(),
            ok: false,
            note: e.to_string(),
        },
    }
}

fn send_webhook(cfg: &WebhookConfig, evt: &NotifyEvent) -> NotifyResult {
    let method = cfg.method.trim().to_uppercase();
    let req = if method == "GET" {
        agent().get(cfg.url.trim())
    } else {
        agent().post(cfg.url.trim())
    };
    let res = if method == "GET" {
        req.call().map_err(|e| anyhow!(e.to_string()))
    } else {
        req.send_json(evt).map_err(|e| anyhow!(e.to_string()))
    };
    match res {
        Ok(_) => NotifyResult {
            channel: "Webhook".into(),
            ok: true,
            note: "已发送".into(),
        },
        Err(e) => NotifyResult {
            channel: "Webhook".into(),
            ok: false,
            note: e.to_string(),
        },
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHA: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as u32;
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(ALPHA[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}
