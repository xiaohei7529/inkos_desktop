//! Agent 维度的 vendor + model 路由：
//!
//! 让管线中不同阶段（plan / compose / draft / audit / revise / aigc / style）使用不同的 vendor / model。
//! 数据落到 `AppSettings.agent_routing`（HashMap<agent, AgentRoute>）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRoute {
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub model_override: String,
}

impl AgentRoute {
    pub fn is_set(&self) -> bool {
        !self.vendor.trim().is_empty() || !self.model_override.trim().is_empty()
    }
}

pub const AGENT_KEYS: &[(&str, &str)] = &[
    ("plan", "Plan · 规划"),
    ("compose", "Compose · 取材"),
    ("draft", "Draft · 起草"),
    ("audit", "Audit · 审计"),
    ("revise", "Revise · 改稿"),
    ("normalize", "Normalize · 字数归一"),
    ("aigc", "AIGC · 检测"),
    ("style", "Style · 文风分析"),
    ("rename", "Rename · 改名校正"),
    ("state_sync", "StateSync · 状态同步"),
];

pub fn label(key: &str) -> String {
    AGENT_KEYS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, l)| l.to_string())
        .unwrap_or_else(|| key.to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentRoutingTable {
    #[serde(default)]
    pub map: HashMap<String, AgentRoute>,
}

impl AgentRoutingTable {
    pub fn route_for(&self, agent: &str) -> Option<&AgentRoute> {
        self.map.get(agent).filter(|r| r.is_set())
    }

    pub fn ensure_keys(&mut self) {
        for (k, _) in AGENT_KEYS {
            self.map.entry(k.to_string()).or_default();
        }
    }
}
