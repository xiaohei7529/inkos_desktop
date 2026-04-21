//! 内置服务商预设（OpenAI 兼容协议）。

#[derive(Clone, Debug)]
pub struct VendorPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub default_model: &'static str,
    pub note: &'static str,
}

pub const VENDORS: &[VendorPreset] = &[
    VendorPreset {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-4o-mini",
        note: "ChatGPT 官方 API",
    },
    VendorPreset {
        id: "anthropic",
        name: "Anthropic",
        base_url: "https://api.anthropic.com/v1",
        default_model: "claude-sonnet-4-5",
        note: "Claude 官方（暂用 OpenAI 兼容路径）",
    },
    VendorPreset {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-chat",
        note: "DeepSeek 官方",
    },
    VendorPreset {
        id: "moonshot",
        name: "Moonshot (Kimi)",
        base_url: "https://api.moonshot.cn/v1",
        default_model: "moonshot-v1-32k",
        note: "Kimi 官方 API",
    },
    VendorPreset {
        id: "minimax",
        name: "MiniMax",
        base_url: "https://api.minimax.chat/v1",
        default_model: "abab6.5s-chat",
        note: "MiniMax 官方",
    },
    VendorPreset {
        id: "dashscope",
        name: "百炼 (通义千问)",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        default_model: "qwen-plus",
        note: "阿里百炼 OpenAI 兼容端点",
    },
    VendorPreset {
        id: "zhipu",
        name: "智谱 GLM",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        default_model: "glm-4-plus",
        note: "智谱 BigModel",
    },
    VendorPreset {
        id: "siliconflow",
        name: "硅基流动",
        base_url: "https://api.siliconflow.cn/v1",
        default_model: "Qwen/Qwen2.5-72B-Instruct",
        note: "SiliconFlow 模型聚合",
    },
    VendorPreset {
        id: "ppio",
        name: "PPIO",
        base_url: "https://api.ppinfra.com/v3/openai",
        default_model: "deepseek/deepseek-v3",
        note: "PPIO 派欧云",
    },
    VendorPreset {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        default_model: "openai/gpt-4o-mini",
        note: "全球模型聚合",
    },
    VendorPreset {
        id: "ollama",
        name: "Ollama (本地)",
        base_url: "http://localhost:11434/v1",
        default_model: "qwen2.5:7b",
        note: "本地推理（无需 API Key）",
    },
    VendorPreset {
        id: "custom",
        name: "自定义服务",
        base_url: "",
        default_model: "",
        note: "任意 OpenAI 兼容服务",
    },
];

pub fn find(id: &str) -> Option<&'static VendorPreset> {
    VENDORS.iter().find(|v| v.id == id)
}
