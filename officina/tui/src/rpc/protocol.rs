// RPC protocol types — all JSON wire types for pi-coding-agent's RPC mode.
//
// Every command, response, and event that flows over stdin/stdout JSONL.
// Types match pi-coding-agent v0.83.0 rpc-types.d.ts exactly.

use serde::{Deserialize, Serialize};

// ── Commands (client → agent via stdin) ───────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum RpcCommand {
    #[serde(rename = "prompt")]
    Prompt {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
        #[serde(rename = "streamingBehavior", skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<String>,
    },
    #[serde(rename = "steer")]
    Steer {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    #[serde(rename = "follow_up")]
    FollowUp {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    #[serde(rename = "abort")]
    Abort {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "new_session")]
    NewSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "parentSession", skip_serializing_if = "Option::is_none")]
        parent_session: Option<String>,
    },
    #[serde(rename = "switch_session")]
    SwitchSession {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "sessionPath")]
        session_path: String,
    },
    #[serde(rename = "get_state")]
    GetState {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "get_messages")]
    GetMessages {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "get_session_stats")]
    GetSessionStats {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "set_model")]
    SetModel {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        provider: String,
        #[serde(rename = "modelId")]
        model_id: String,
    },
    #[serde(rename = "cycle_model")]
    CycleModel {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "get_available_models")]
    GetAvailableModels {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "set_thinking_level")]
    SetThinkingLevel {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        level: String,
    },
    #[serde(rename = "cycle_thinking_level")]
    CycleThinkingLevel {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "compact")]
    Compact {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "customInstructions", skip_serializing_if = "Option::is_none")]
        custom_instructions: Option<String>,
    },
    #[serde(rename = "bash")]
    Bash {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        command: String,
    },
    #[serde(rename = "get_commands")]
    GetCommands {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    // Extension UI response (client → agent)
    #[serde(rename = "extension_ui_response")]
    ExtensionUiResponse {
        id: String,
        #[serde(flatten)]
        response: UiResponsePayload,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UiResponsePayload {
    Value { value: String },
    Confirm { confirmed: bool },
    Cancel { cancelled: bool },
}

impl RpcCommand {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Prompt { id, .. }
            | Self::Steer { id, .. }
            | Self::FollowUp { id, .. }
            | Self::Abort { id }
            | Self::NewSession { id, .. }
            | Self::SwitchSession { id, .. }
            | Self::GetState { id }
            | Self::GetMessages { id }
            | Self::GetSessionStats { id }
            | Self::SetModel { id, .. }
            | Self::CycleModel { id }
            | Self::GetAvailableModels { id }
            | Self::SetThinkingLevel { id, .. }
            | Self::CycleThinkingLevel { id }
            | Self::Compact { id, .. }
            | Self::Bash { id, .. }
            | Self::GetCommands { id } => id.as_deref(),
            Self::ExtensionUiResponse { id, .. } => Some(id),
        }
    }
}

// ── Responses (agent → client via stdout) ─────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RpcResponse {
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub command: Option<String>,
    pub success: Option<bool>,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

// ── Events (agent → client via stdout, no "response" type) ───────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RpcEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(flatten)]
    pub fields: serde_json::Value,
}

// ── Extension UI requests (agent → client) ────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionUiRequest {
    pub id: String,
    pub method: String,
    #[serde(flatten)]
    pub fields: serde_json::Value,
}

// ── Model type ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Model {
    pub id: String,
    pub name: Option<String>,
    pub provider: String,
    #[serde(rename = "contextWindow")]
    pub context_window: Option<u64>,
    #[serde(rename = "maxTokens")]
    pub max_tokens: Option<u64>,
    pub reasoning: Option<bool>,
}

// ── Session state ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RpcSessionState {
    pub model: Option<Model>,
    #[serde(rename = "thinkingLevel")]
    pub thinking_level: Option<String>,
    #[serde(rename = "isStreaming")]
    pub is_streaming: bool,
    #[serde(rename = "isCompacting")]
    pub is_compacting: bool,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(rename = "sessionName")]
    pub session_name: Option<String>,
    #[serde(rename = "messageCount")]
    pub message_count: Option<u64>,
}

// ── Session stats ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionStats {
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(rename = "userMessages")]
    pub user_messages: u64,
    #[serde(rename = "assistantMessages")]
    pub assistant_messages: u64,
    #[serde(rename = "toolCalls")]
    pub tool_calls: u64,
    #[serde(rename = "totalMessages")]
    pub total_messages: u64,
    pub tokens: Option<TokenUsage>,
    pub context_usage: Option<ContextUsage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ContextUsage {
    pub tokens: Option<u64>,
    #[serde(rename = "contextWindow")]
    pub context_window: u64,
    pub percent: Option<f64>,
}

// ── Agent message ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AgentMessage {
    pub role: Option<String>,
    pub content: Option<serde_json::Value>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
}

// ── Streaming events ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct MessageUpdateEvent {
    pub message: Option<AgentMessage>,
    #[serde(rename = "assistantMessageEvent")]
    pub assistant_event: Option<AssistantMessageEvent>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssistantMessageEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub delta: Option<String>,
    pub content: Option<String>,
    pub reason: Option<String>,
}

// ── Slash command ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub source: Option<String>,
}
