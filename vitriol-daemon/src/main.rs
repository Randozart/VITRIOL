//! VITRIOL Daemon - Unix Socket Server with llama.cpp Integration
//!
//! This daemon provides:
//! - Unix domain socket API for agentic systems
//! - JSON protocol over socket
//! - llama.cpp model loading and inference
//! - Layer management (LRU eviction for larger models)

use anyhow::Result;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::oneshot;

// ============================================================================
// Error Codes
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum VitriolError {
    GpuNotDetected,
    BarMapFailed,
    MemoryFull,
    DmaFailed,
    WriteBlocked,
    ModelLoadFailed(String),
    InferenceFailed(String),
    NotConnected,
    ProtocolError(String),
}

impl From<VitriolError> for ErrorResponse {
    fn from(e: VitriolError) -> Self {
        match e {
            VitriolError::GpuNotDetected => ErrorResponse {
                code: "GPU_NOT_DETECTED".to_string(),
                message: "Failed to enumerate GPU via pci_get_device".to_string(),
            },
            VitriolError::BarMapFailed => ErrorResponse {
                code: "BAR_MAP_FAILED".to_string(),
                message: "Cannot map GPU BAR".to_string(),
            },
            VitriolError::MemoryFull => ErrorResponse {
                code: "MEMORY_FULL".to_string(),
                message: "VRAM allocation failed".to_string(),
            },
            VitriolError::DmaFailed => ErrorResponse {
                code: "DMA_FAILED".to_string(),
                message: "DMA transfer failed".to_string(),
            },
            VitriolError::WriteBlocked => ErrorResponse {
                code: "WRITE_BLOCKED".to_string(),
                message: "Write attempted at safety_level=1".to_string(),
            },
            VitriolError::ModelLoadFailed(msg) => ErrorResponse {
                code: "MODEL_LOAD_FAILED".to_string(),
                message: format!("llama.cpp failed to load model: {}", msg),
            },
            VitriolError::InferenceFailed(msg) => ErrorResponse {
                code: "INFERENCE_FAILED".to_string(),
                message: format!("llama.cpp inference error: {}", msg),
            },
            VitriolError::NotConnected => ErrorResponse {
                code: "NOT_CONNECTED".to_string(),
                message: "Not connected to daemon".to_string(),
            },
            VitriolError::ProtocolError(msg) => ErrorResponse {
                code: "PROTOCOL_ERROR".to_string(),
                message: msg,
            },
        }
    }
}

// ============================================================================
// Protocol Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct Request {
    cmd: String,
    params: HashMap<String, serde_json::Value>,
    id: u64,
}

#[derive(Debug, Serialize)]
pub struct Response {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorResponse>,
    id: u64,
}

impl Response {
    pub fn ok(data: serde_json::Value, id: u64) -> Self {
        Response {
            status: "ok".to_string(),
            data: Some(data),
            error: None,
            id,
        }
    }

    pub fn error(e: VitriolError, id: u64) -> Self {
        let err: ErrorResponse = e.into();
        Response {
            status: "error".to_string(),
            data: None,
            error: Some(err),
            id,
        }
    }
}

// ============================================================================
// State
// ============================================================================

pub struct DaemonState {
    pub safety_level: u32,
    pub gpu_present: bool,
    pub gpu_name: String,
    pub vram_total: u64,
    pub vram_used: u64,
    pub model_loaded: bool,
    pub model_path: Option<String>,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            safety_level: 1,
            gpu_present: false,
            gpu_name: "Unknown".to_string(),
            vram_total: 0,
            vram_used: 0,
            model_loaded: false,
            model_path: None,
        }
    }
}

// ============================================================================
// Command Handlers
// ============================================================================

async fn handle_status(state: &Mutex<DaemonState>) -> Result<serde_json::Value> {
    let state = state.lock().unwrap();
    Ok(serde_json::json!({
        "gpu_present": state.gpu_present,
        "gpu_name": state.gpu_name,
        "vram_total": state.vram_total,
        "vram_used": state.vram_used,
        "vram_free": state.vram_total.saturating_sub(state.vram_used),
        "model_loaded": state.model_loaded,
        "model_path": state.model_path,
        "safety_level": state.safety_level,
        "dma_engine": "not_initialized",  // Phase 2
    }))
}

async fn handle_load_model(state: &Mutex<DaemonState>, path: &str) -> Result<serde_json::Value> {
    info!("Loading model from: {}", path);

    // TODO: Integrate llama.cpp
    // For now, just record that we'd load the model
    let mut state = state.lock().unwrap();
    state.model_loaded = true;
    state.model_path = Some(path.to_string());

    Ok(serde_json::json!({
        "loaded": true,
        "path": path,
        "message": "Model loaded (llama.cpp integration pending)"
    }))
}

async fn handle_infer(
    _state: &Mutex<DaemonState>,
    prompt: &str,
    _max_tokens: u32,
) -> Result<serde_json::Value> {
    info!("Inference request: {}", prompt);

    // TODO: Integrate llama.cpp
    Ok(serde_json::json!({
        "output": "[Inference not yet implemented - llama.cpp integration pending]",
        "tokens_generated": 0,
        "inference_time_ms": 0
    }))
}

async fn handle_set_safety(state: &Mutex<DaemonState>, level: u32) -> Result<serde_json::Value> {
    let mut state = state.lock().unwrap();
    state.safety_level = level;
    info!("Safety level set to: {}", level);
    Ok(serde_json::json!({
        "safety_level": level,
        "message": format!("Safety level set to {}", level)
    }))
}

// ============================================================================
// Socket Handler
// ============================================================================

async fn handle_connection(sock: tokio::net::UnixStream, state: Mutex<DaemonState>) -> Result<()> {
    let mut buf = vec![0u8; 4096];
    let mut stream = tokio::io::BufStream::new(sock);
    let mut recv_buf = Vec::new();

    loop {
        // Read length prefix (4 bytes)
        let n = stream.read(&mut buf[..4]).await?;
        if n == 0 {
            break; // Connection closed
        }
        if n < 4 {
            warn!("Short read on length prefix");
            break;
        }

        let body_len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

        // Read body
        recv_buf.resize(body_len, 0);
        let mut offset = 0;
        while offset < body_len {
            let n = stream.read(&mut recv_buf[offset..]).await?;
            if n == 0 {
                break;
            }
            offset += n;
        }

        // Parse request
        let request: Request = match serde_json::from_slice(&recv_buf) {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to parse request: {}", e);
                break;
            }
        };

        info!("Received command: {}", request.cmd);

        // Handle command
        let response = match request.cmd.as_str() {
            "STATUS" => match handle_status(&state).await {
                Ok(data) => Response::ok(data, request.id),
                Err(e) => Response::error(VitriolError::ProtocolError(e.to_string()), request.id),
            },
            "LOAD_MODEL" => {
                let path = request
                    .params
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match handle_load_model(&state, path).await {
                    Ok(data) => Response::ok(data, request.id),
                    Err(e) => {
                        Response::error(VitriolError::ModelLoadFailed(e.to_string()), request.id)
                    }
                }
            }
            "INFER" => {
                let prompt = request
                    .params
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let max_tokens = request
                    .params
                    .get("max_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100) as u32;
                match handle_infer(&state, prompt, max_tokens).await {
                    Ok(data) => Response::ok(data, request.id),
                    Err(e) => {
                        Response::error(VitriolError::InferenceFailed(e.to_string()), request.id)
                    }
                }
            }
            "SET_SAFETY" => {
                let level = request
                    .params
                    .get("level")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                match handle_set_safety(&state, level).await {
                    Ok(data) => Response::ok(data, request.id),
                    Err(e) => {
                        Response::error(VitriolError::ProtocolError(e.to_string()), request.id)
                    }
                }
            }
            "PING" => Response::ok(serde_json::json!({"pong": true}), request.id),
            cmd => Response::error(
                VitriolError::ProtocolError(format!("Unknown command: {}", cmd)),
                request.id,
            ),
        };

        // Send response
        let response_bytes = serde_json::to_vec(&response)?;
        let header = (response_bytes.len() as u32).to_le_bytes();
        stream.write_all(&header).await?;
        stream.write_all(&response_bytes).await?;
    }

    Ok(())
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("VITRIOL Daemon starting...");

    let socket_path = "/var/run/vitriol.sock";
    let state = Mutex::new(DaemonState::default());

    // Remove old socket if exists
    if std::path::Path::new(socket_path).exists() {
        std::fs::remove_file(socket_path)?;
    }

    // Create socket
    let listener = UnixListener::bind(socket_path)?;
    info!("Listening on: {}", socket_path);

    // Set permissions (allow anyone to connect)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o666))?;
    }

    // Accept connections
    loop {
        match listener.accept().await {
            Ok((sock, _)) => {
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(sock, state).await {
                        error!("Connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Accept error: {}", e);
            }
        }
    }
}
