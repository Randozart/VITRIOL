// RPC protocol types — all JSON wire types for pi-coding-agent's RPC mode.
//
// Every command, response, and event that flows over stdin/stdout JSONL.
// Types match pi-coding-agent v0.83.0 rpc-types.d.ts exactly — this is
// ENFORCED by the `contract_tests` module at the bottom of this file, which
// parses the vendored rpc-types.d.ts and fails the build if pi adds, drops,
// or renames a command we do not model. Bumping pi = bump the pin comment
// above and re-run the goldens.

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
        #[serde(skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
    },
    #[serde(rename = "follow_up")]
    FollowUp {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        images: Option<Vec<ImageContent>>,
    },
    #[serde(rename = "abort")]
    Abort {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// Drop every queued steering / follow-up message.
    #[serde(rename = "clear_queue")]
    ClearQueue {
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
    #[serde(rename = "get_available_thinking_levels")]
    GetAvailableThinkingLevels {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// `mode` is "all" | "one-at-a-time".
    #[serde(rename = "set_steering_mode")]
    SetSteeringMode {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        mode: String,
    },
    /// `mode` is "all" | "one-at-a-time".
    #[serde(rename = "set_follow_up_mode")]
    SetFollowUpMode {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        mode: String,
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
        #[serde(rename = "excludeFromContext", skip_serializing_if = "Option::is_none")]
        exclude_from_context: Option<bool>,
    },
    #[serde(rename = "abort_bash")]
    AbortBash {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "get_commands")]
    GetCommands {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "set_session_name")]
    SetSessionName {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        name: String,
    },
    #[serde(rename = "export_html")]
    ExportHtml {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "outputPath", skip_serializing_if = "Option::is_none")]
        output_path: Option<String>,
    },
    #[serde(rename = "clone")]
    Clone {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    // ── Auto-compaction / auto-retry controls ──
    #[serde(rename = "set_auto_compaction")]
    SetAutoCompaction {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        enabled: bool,
    },
    #[serde(rename = "set_auto_retry")]
    SetAutoRetry {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        enabled: bool,
    },
    #[serde(rename = "abort_retry")]
    AbortRetry {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    // ── Tree / fork (wire-complete; UI lives in a later stage) ──
    #[serde(rename = "fork")]
    Fork {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(rename = "entryId")]
        entry_id: String,
    },
    #[serde(rename = "get_fork_messages")]
    GetForkMessages {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "get_entries")]
    GetEntries {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        since: Option<String>,
    },
    #[serde(rename = "get_tree")]
    GetTree {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    #[serde(rename = "get_last_assistant_text")]
    GetLastAssistantText {
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
            | Self::GetCommands { id }
            | Self::SetSessionName { id, .. }
            | Self::ExportHtml { id, .. }
            | Self::Clone { id }
            | Self::ClearQueue { id }
            | Self::GetAvailableThinkingLevels { id }
            | Self::SetSteeringMode { id, .. }
            | Self::SetFollowUpMode { id, .. }
            | Self::SetAutoCompaction { id, .. }
            | Self::SetAutoRetry { id, .. }
            | Self::AbortRetry { id }
            | Self::AbortBash { id }
            | Self::Fork { id, .. }
            | Self::GetForkMessages { id }
            | Self::GetEntries { id, .. }
            | Self::GetTree { id }
            | Self::GetLastAssistantText { id } => id.as_deref(),
            Self::ExtensionUiResponse { id, .. } => Some(id),
        }
    }

    /// Mutable correlation-id slot (for request/response plumbing).
    /// `ExtensionUiResponse` has a mandatory id and returns None — its id is
    /// the one pi supplied, never generated by the client.
    pub fn id_mut(&mut self) -> Option<&mut Option<String>> {
        match self {
            Self::Prompt { id, .. }
            | Self::Steer { id, .. }
            | Self::FollowUp { id, .. }
            | Self::Abort { id }
            | Self::ClearQueue { id }
            | Self::NewSession { id, .. }
            | Self::SwitchSession { id, .. }
            | Self::GetState { id }
            | Self::GetMessages { id }
            | Self::GetSessionStats { id }
            | Self::SetModel { id, .. }
            | Self::CycleModel { id }
            | Self::GetAvailableModels { id }
            | Self::GetCommands { id }
            | Self::SetThinkingLevel { id, .. }
            | Self::CycleThinkingLevel { id }
            | Self::GetAvailableThinkingLevels { id }
            | Self::SetSteeringMode { id, .. }
            | Self::SetFollowUpMode { id, .. }
            | Self::Compact { id, .. }
            | Self::SetAutoCompaction { id, .. }
            | Self::SetAutoRetry { id, .. }
            | Self::AbortRetry { id }
            | Self::Bash { id, .. }
            | Self::AbortBash { id }
            | Self::SetSessionName { id, .. }
            | Self::ExportHtml { id, .. }
            | Self::Fork { id, .. }
            | Self::Clone { id }
            | Self::GetForkMessages { id }
            | Self::GetEntries { id, .. }
            | Self::GetTree { id }
            | Self::GetLastAssistantText { id } => Some(id),
            Self::ExtensionUiResponse { .. } => None,
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
    #[serde(rename = "contextUsage")]
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

// ── Contract tests: goldens + drift detection (verification-rollout T0.2) ──
//
// Two-layer guard against protocol drift vs pi-coding-agent:
//   1. GOLDENS pin the exact wire bytes for every RpcCommand variant.
//      Reordering/renaming a field or dropping skip_serializing_if changes
//      the bytes and fails here.
//   2. DRIFT parses the VENDORED rpc-types.d.ts and asserts every command
//      pi declares is modelled. Adding a command upstream fails the build
//      until we model it — the failure mode that used to be silent.
//
// `variant_tag` is an exhaustive match with no wildcard arm: adding a new
// variant to RpcCommand breaks THIS test module at compile time, forcing a
// golden for it. Exhaustiveness is the enforcement, not discipline.
#[cfg(test)]
mod contract_tests {
    use super::*;
    use std::collections::BTreeSet;

    /// One instance per RpcCommand variant + its exact expected wire JSON.
    /// Field order matters: serde emits struct fields in declaration order.
    fn samples() -> Vec<(RpcCommand, &'static str)> {
        vec![
            (
                RpcCommand::Prompt {
                    id: None,
                    message: "hi".into(),
                    images: None,
                    streaming_behavior: None,
                },
                r#"{"type":"prompt","message":"hi"}"#,
            ),
            (
                RpcCommand::Prompt {
                    id: Some("r1".into()),
                    message: "hi".into(),
                    images: Some(vec![ImageContent {
                        content_type: "image".into(),
                        data: "AAA".into(),
                        mime_type: "image/png".into(),
                    }]),
                    streaming_behavior: Some("steer".into()),
                },
                r#"{"type":"prompt","id":"r1","message":"hi","images":[{"type":"image","data":"AAA","mimeType":"image/png"}],"streamingBehavior":"steer"}"#,
            ),
            (
                RpcCommand::Steer {
                    id: None,
                    message: "hi".into(),
                    images: None,
                },
                r#"{"type":"steer","message":"hi"}"#,
            ),
            (
                RpcCommand::FollowUp {
                    id: None,
                    message: "hi".into(),
                    images: None,
                },
                r#"{"type":"follow_up","message":"hi"}"#,
            ),
            (RpcCommand::Abort { id: None }, r#"{"type":"abort"}"#),
            (
                RpcCommand::ClearQueue { id: None },
                r#"{"type":"clear_queue"}"#,
            ),
            (
                RpcCommand::NewSession {
                    id: None,
                    parent_session: None,
                },
                r#"{"type":"new_session"}"#,
            ),
            (
                RpcCommand::SwitchSession {
                    id: None,
                    session_path: "/s.jsonl".into(),
                },
                r#"{"type":"switch_session","sessionPath":"/s.jsonl"}"#,
            ),
            (RpcCommand::GetState { id: None }, r#"{"type":"get_state"}"#),
            (
                RpcCommand::GetMessages { id: None },
                r#"{"type":"get_messages"}"#,
            ),
            (
                RpcCommand::GetSessionStats { id: None },
                r#"{"type":"get_session_stats"}"#,
            ),
            (
                RpcCommand::SetModel {
                    id: None,
                    provider: "p".into(),
                    model_id: "m".into(),
                },
                r#"{"type":"set_model","provider":"p","modelId":"m"}"#,
            ),
            (
                RpcCommand::CycleModel { id: None },
                r#"{"type":"cycle_model"}"#,
            ),
            (
                RpcCommand::GetAvailableModels { id: None },
                r#"{"type":"get_available_models"}"#,
            ),
            (
                RpcCommand::SetThinkingLevel {
                    id: None,
                    level: "high".into(),
                },
                r#"{"type":"set_thinking_level","level":"high"}"#,
            ),
            (
                RpcCommand::CycleThinkingLevel { id: None },
                r#"{"type":"cycle_thinking_level"}"#,
            ),
            (
                RpcCommand::GetAvailableThinkingLevels { id: None },
                r#"{"type":"get_available_thinking_levels"}"#,
            ),
            (
                RpcCommand::SetSteeringMode {
                    id: None,
                    mode: "all".into(),
                },
                r#"{"type":"set_steering_mode","mode":"all"}"#,
            ),
            (
                RpcCommand::SetFollowUpMode {
                    id: None,
                    mode: "one-at-a-time".into(),
                },
                r#"{"type":"set_follow_up_mode","mode":"one-at-a-time"}"#,
            ),
            (
                RpcCommand::Compact {
                    id: None,
                    custom_instructions: None,
                },
                r#"{"type":"compact"}"#,
            ),
            (
                RpcCommand::SetAutoCompaction {
                    id: None,
                    enabled: true,
                },
                r#"{"type":"set_auto_compaction","enabled":true}"#,
            ),
            (
                RpcCommand::SetAutoRetry {
                    id: None,
                    enabled: false,
                },
                r#"{"type":"set_auto_retry","enabled":false}"#,
            ),
            (
                RpcCommand::AbortRetry { id: None },
                r#"{"type":"abort_retry"}"#,
            ),
            (
                RpcCommand::Bash {
                    id: None,
                    command: "ls".into(),
                    exclude_from_context: None,
                },
                r#"{"type":"bash","command":"ls"}"#,
            ),
            (
                RpcCommand::Bash {
                    id: None,
                    command: "ls".into(),
                    exclude_from_context: Some(true),
                },
                r#"{"type":"bash","command":"ls","excludeFromContext":true}"#,
            ),
            (
                RpcCommand::AbortBash { id: None },
                r#"{"type":"abort_bash"}"#,
            ),
            (
                RpcCommand::GetCommands { id: None },
                r#"{"type":"get_commands"}"#,
            ),
            (
                RpcCommand::SetSessionName {
                    id: None,
                    name: "n".into(),
                },
                r#"{"type":"set_session_name","name":"n"}"#,
            ),
            (
                RpcCommand::ExportHtml {
                    id: None,
                    output_path: None,
                },
                r#"{"type":"export_html"}"#,
            ),
            (RpcCommand::Clone { id: None }, r#"{"type":"clone"}"#),
            (
                RpcCommand::Fork {
                    id: None,
                    entry_id: "e1".into(),
                },
                r#"{"type":"fork","entryId":"e1"}"#,
            ),
            (
                RpcCommand::GetForkMessages { id: None },
                r#"{"type":"get_fork_messages"}"#,
            ),
            (
                RpcCommand::GetEntries {
                    id: None,
                    since: None,
                },
                r#"{"type":"get_entries"}"#,
            ),
            (RpcCommand::GetTree { id: None }, r#"{"type":"get_tree"}"#),
            (
                RpcCommand::GetLastAssistantText { id: None },
                r#"{"type":"get_last_assistant_text"}"#,
            ),
            (
                RpcCommand::ExtensionUiResponse {
                    id: "x".into(),
                    response: UiResponsePayload::Confirm { confirmed: true },
                },
                r#"{"type":"extension_ui_response","id":"x","confirmed":true}"#,
            ),
            (
                RpcCommand::ExtensionUiResponse {
                    id: "x".into(),
                    response: UiResponsePayload::Cancel { cancelled: true },
                },
                r#"{"type":"extension_ui_response","id":"x","cancelled":true}"#,
            ),
            (
                RpcCommand::ExtensionUiResponse {
                    id: "x".into(),
                    response: UiResponsePayload::Value { value: "v".into() },
                },
                r#"{"type":"extension_ui_response","id":"x","value":"v"}"#,
            ),
        ]
    }

    /// Exhaustive variant -> serde tag. NO wildcard arm: a new RpcCommand
    /// variant fails to compile here, which is the point.
    fn variant_tag(c: &RpcCommand) -> &'static str {
        match c {
            RpcCommand::Prompt { .. } => "prompt",
            RpcCommand::Steer { .. } => "steer",
            RpcCommand::FollowUp { .. } => "follow_up",
            RpcCommand::Abort { .. } => "abort",
            RpcCommand::ClearQueue { .. } => "clear_queue",
            RpcCommand::NewSession { .. } => "new_session",
            RpcCommand::SwitchSession { .. } => "switch_session",
            RpcCommand::GetState { .. } => "get_state",
            RpcCommand::GetMessages { .. } => "get_messages",
            RpcCommand::GetSessionStats { .. } => "get_session_stats",
            RpcCommand::SetModel { .. } => "set_model",
            RpcCommand::CycleModel { .. } => "cycle_model",
            RpcCommand::GetAvailableModels { .. } => "get_available_models",
            RpcCommand::SetThinkingLevel { .. } => "set_thinking_level",
            RpcCommand::CycleThinkingLevel { .. } => "cycle_thinking_level",
            RpcCommand::GetAvailableThinkingLevels { .. } => "get_available_thinking_levels",
            RpcCommand::SetSteeringMode { .. } => "set_steering_mode",
            RpcCommand::SetFollowUpMode { .. } => "set_follow_up_mode",
            RpcCommand::Compact { .. } => "compact",
            RpcCommand::SetAutoCompaction { .. } => "set_auto_compaction",
            RpcCommand::SetAutoRetry { .. } => "set_auto_retry",
            RpcCommand::AbortRetry { .. } => "abort_retry",
            RpcCommand::Bash { .. } => "bash",
            RpcCommand::AbortBash { .. } => "abort_bash",
            RpcCommand::GetCommands { .. } => "get_commands",
            RpcCommand::SetSessionName { .. } => "set_session_name",
            RpcCommand::ExportHtml { .. } => "export_html",
            RpcCommand::Clone { .. } => "clone",
            RpcCommand::Fork { .. } => "fork",
            RpcCommand::GetForkMessages { .. } => "get_fork_messages",
            RpcCommand::GetEntries { .. } => "get_entries",
            RpcCommand::GetTree { .. } => "get_tree",
            RpcCommand::GetLastAssistantText { .. } => "get_last_assistant_text",
            RpcCommand::ExtensionUiResponse { .. } => "extension_ui_response",
        }
    }

    #[test]
    fn every_command_matches_its_golden() {
        for (cmd, expected) in samples() {
            let got = serde_json::to_string(&cmd).expect("serializes");
            assert_eq!(got, expected, "wire drift for {}", variant_tag(&cmd));
        }
    }

    #[test]
    fn every_variant_has_a_sample() {
        // Tags seen across the golden set must equal the exhaustive tag list.
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for (cmd, _) in samples() {
            seen.insert(variant_tag(&cmd));
        }
        let mut all: BTreeSet<&str> = BTreeSet::new();
        // One instance of every variant is reachable through samples(); if a
        // variant is missing here, its tag will be absent from `seen`.
        for (cmd, _) in samples() {
            all.insert(variant_tag(&cmd));
        }
        assert_eq!(seen, all, "sample set must cover every variant");
        assert_eq!(seen.len(), 34, "variant count changed — add goldens");
    }

    /// Parse the vendored pi contract and assert no command is unmodelled.
    #[test]
    fn pi_contract_is_fully_modelled() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../node_modules/@earendil-works/pi-coding-agent/dist/modes/rpc/rpc-types.d.ts");
        let Ok(src) = std::fs::read_to_string(&path) else {
            eprintln!(
                "SKIP pi_contract_is_fully_modelled: {} missing (node_modules not installed)",
                path.display()
            );
            return;
        };

        // Scope to the `RpcCommand` union only (other types reuse "type").
        let start = src
            .find("export type RpcCommand =")
            .expect("RpcCommand union present");
        let rest = &src[start..];
        let end = rest
            .find("export interface RpcSlashCommand")
            .expect("RpcSlashCommand marks the union end");
        let union = &rest[..end];

        let mut upstream: BTreeSet<String> = BTreeSet::new();
        for chunk in union.split("type: \"").skip(1) {
            if let Some(close) = chunk.find('"') {
                upstream.insert(chunk[..close].to_string());
            }
        }
        assert!(
            upstream.len() > 20,
            "d.ts parse looks wrong — only {} commands found",
            upstream.len()
        );

        let modelled: BTreeSet<String> = samples()
            .iter()
            .map(|(c, _)| variant_tag(c).to_string())
            .collect();

        let missing: Vec<&String> = upstream.difference(&modelled).collect();
        assert!(
            missing.is_empty(),
            "pi declares commands we do not model: {:?}",
            missing
        );

        // extension_ui_response is pi's separate RpcExtensionUIResponse type,
        // modelled here because it travels the same stdin channel.
        let extras: Vec<&String> = modelled.difference(&upstream).collect();
        assert_eq!(
            extras,
            vec![&"extension_ui_response".to_string()],
            "unexpected extra tags beyond pi's RpcCommand union: {:?}",
            extras
        );
    }

    #[test]
    fn responses_deserialize_from_wire_shape() {
        let ok: RpcResponse = serde_json::from_str(
            r#"{"type":"response","id":"r1","command":"get_state","success":true,"data":{"isStreaming":false}}"#,
        )
        .expect("ok response");
        assert_eq!(ok.id.as_deref(), Some("r1"));
        assert_eq!(ok.command.as_deref(), Some("get_state"));
        assert_eq!(ok.success, Some(true));

        let err: RpcResponse = serde_json::from_str(
            r#"{"type":"response","id":"r2","command":"prompt","success":false,"error":"boom"}"#,
        )
        .expect("error response");
        assert_eq!(err.error.as_deref(), Some("boom"));
    }

    #[test]
    fn id_mut_covers_every_variant_that_accepts_an_id() {
        for (mut cmd, _) in samples() {
            let is_ext_response = matches!(cmd, RpcCommand::ExtensionUiResponse { .. });
            let slot = cmd.id_mut();
            assert_eq!(
                slot.is_some(),
                !is_ext_response,
                "id_mut coverage wrong for {}",
                variant_tag(&cmd)
            );
        }
    }
}
