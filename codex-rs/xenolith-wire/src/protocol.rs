// Xenolith native integration: added or modified by Francesco Pio Crisci, 2026.
// Licensed under Apache-2.0; upstream copyright and notices are retained.

//! Validated wire v1/v2 replies and local inference events. No Responses transport.
use crate::WireConnection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn request(wire: &mut WireConnection, value: Value) -> io::Result<Value> {
    wire.send_frame(&serde_json::to_vec(&value)?)?;
    let reply: Value = serde_json::from_str(&wire.receive_frame()?)?;
    match reply.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(reply),
        Some(false) => Err(invalid(format!(
            "Xenolith {}: {}",
            reply
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
            reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unspecified error")
        ))),
        None => Err(invalid("expected Xenolith operation reply")),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReasoningCapabilities {
    pub efforts: Vec<String>,
    pub history: Vec<String>,
    pub budget_tokens: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VisionCapabilities {
    pub formats: Vec<String>,
    pub budgets: Vec<u32>,
    pub default_budget: u32,
    pub max_bytes: u64,
    pub max_context_media_bytes: u64,
    pub media_refs: bool,
}

impl VisionCapabilities {
    fn validate(&self) -> io::Result<()> {
        if self.formats.is_empty()
            || self
                .formats
                .iter()
                .any(|format| !matches!(format.as_str(), "png" | "jpeg"))
            || self.budgets.is_empty()
            || self
                .budgets
                .iter()
                .any(|budget| *budget == 0 || *budget > i32::MAX as u32)
            || !self.budgets.contains(&self.default_budget)
            || self.max_bytes == 0
            || self.max_context_media_bytes < self.max_bytes
        {
            return Err(invalid("invalid Xenolith vision capabilities"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Describe {
    pub protocol: u32,
    pub model: String,
    pub context_window: u64,
    pub max_output: u64,
    pub max_frame: usize,
    pub kvstore: bool,
    pub reasoning: ReasoningCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<VisionCapabilities>,
}

impl Describe {
    pub fn discover(wire: &mut WireConnection) -> io::Result<Self> {
        let info: Self =
            serde_json::from_value(request(wire, serde_json::json!({"op":"describe"}))?)?;
        if !matches!(info.protocol, 1 | 2)
            || info.model.trim().is_empty()
            || info.model.contains('\0')
            || info.context_window == 0
            || info.context_window > i64::MAX as u64
            || info.max_output == 0
            || info.max_output > i32::MAX as u64
            || info
                .reasoning
                .efforts
                .iter()
                .any(|e| !["low", "medium", "high", "max"].contains(&e.as_str()))
            || info
                .reasoning
                .history
                .iter()
                .any(|h| !["discard", "preserve_tool_calls"].contains(&h.as_str()))
        {
            return Err(invalid("incompatible Xenolith describe"));
        }
        match (info.protocol, &info.vision) {
            (1, None) => {}
            (2, Some(vision)) => vision.validate()?,
            _ => return Err(invalid("Xenolith vision capabilities require protocol v2")),
        }
        // Wire max_frame includes the terminating LF (unlike our framing API).
        if info.max_frame < 2 {
            return Err(invalid("invalid max_frame"));
        }
        wire.set_max_frame(info.max_frame - 1)?;
        Ok(info)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub cache_read: u64,
    pub output: u64,
    pub total: u64,
    pub reasoning: u64,
    pub replayed: u64,
}

impl Usage {
    pub fn validate(&self) -> io::Result<()> {
        let total = self
            .input
            .checked_add(self.cache_read)
            .and_then(|n| n.checked_add(self.output));
        // Xenolith total is the projected conversation length, not per-request
        // accounting: template closure and replay can make these totals differ.
        if total.is_none_or(|n| n > i64::MAX as u64)
            || self.total > i64::MAX as u64
            || self.reasoning > self.output
            || self.replayed > self.input
        {
            return Err(invalid("invalid wire token usage"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Checkpoint {
    pub saved: bool,
    pub tokens: u64,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Resume {
    pub loaded: bool,
    pub tokens: u64,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Measurement {
    pub tokens: u64,
    pub elapsed_ms: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Inference {
    pub prefill: Measurement,
    pub decode: Measurement,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WireEvent {
    Start,
    Progress {
        prefilled: u64,
        total: u64,
    },
    InferenceProgress {
        phase: String,
        state: Option<String>,
        tokens: u64,
        total: Option<u64>,
        elapsed_ms: u64,
    },
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolcallStart {
        id: u64,
        name: String,
    },
    ToolcallEnd {
        id: u64,
        name: String,
        arguments: Value,
    },
    Done {
        stop: String,
        reasoning_close: Option<String>,
        usage: Usage,
        marker: Option<u64>,
        checkpoint: Option<Checkpoint>,
        resume: Option<Resume>,
        inference: Option<Inference>,
    },
    Error {
        code: String,
        error: String,
    },
}

impl WireEvent {
    pub fn parse(line: &str) -> io::Result<Self> {
        let event: Self = serde_json::from_str(line)?;
        match &event {
            Self::Progress { prefilled, total } if prefilled > total => {
                return Err(invalid("invalid prefill progress"));
            }
            Self::InferenceProgress {
                phase,
                state,
                tokens,
                total,
                ..
            } => {
                if !["prefill", "decode"].contains(&phase.as_str())
                    || state
                        .as_deref()
                        .is_some_and(|s| !["running", "finished"].contains(&s))
                    || (phase == "prefill" && total.is_none_or(|n| n < *tokens))
                {
                    return Err(invalid("invalid inference progress"));
                }
            }
            Self::ToolcallEnd { arguments, .. } if !arguments.is_object() => {
                return Err(invalid("non-object tool arguments"));
            }
            Self::Done {
                stop,
                reasoning_close,
                usage,
                inference,
                ..
            } => {
                if inference
                    .as_ref()
                    .is_some_and(|i| i.prefill.total.is_none_or(|n| n < i.prefill.tokens))
                {
                    return Err(invalid("invalid final inference measurement"));
                }
                if !["stop", "tool_use", "length", "aborted"].contains(&stop.as_str()) {
                    return Err(invalid("unknown generation stop"));
                }
                if reasoning_close.as_deref().is_some_and(|close| {
                    !["natural", "soft", "hard", "length", "aborted", "eos"].contains(&close)
                }) {
                    return Err(invalid("unknown reasoning close"));
                }
                usage.validate()?;
            }
            _ => {}
        }
        Ok(event)
    }
}
