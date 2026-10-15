// Xenolith native integration: added or modified by Francesco Pio Crisci, 2026.
// Licensed under Apache-2.0; upstream copyright and notices are retained.

//! Small native entry point: no HTTP, SSE parser or Responses request builder.
mod config;

use crate::client_common::{Prompt, ResponseEvent, ResponseStream};
use codex_protocol::error::{CodexErrorDetails, Result};
use codex_protocol::openai_models::{ModelInfo, ReasoningEffort};
use codex_protocol::protocol::TokenUsage;
use codex_xenolith_wire::backend::{self, Output};
use std::io;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) fn stream(
    prompt: &Prompt,
    model: &ModelInfo,
    thread: String,
    base_url: Option<&str>,
    effort: Option<ReasoningEffort>,
) -> Result<ResponseStream> {
    if prompt.output_schema.is_some() {
        return Err(CodexErrorDetails::UnsupportedOperation(
            "Xenolith constrained JSON unsupported".into(),
        )
        .into());
    }
    let generation = config::GenerationSettings::from_env(effort.as_ref())
        .map_err(|error| CodexErrorDetails::InvalidRequest(error.to_string()))?;
    let socket = base_url
        .and_then(|url| url.strip_prefix("unix://"))
        .filter(|path| path.starts_with('/'))
        .ok_or_else(|| {
            CodexErrorDetails::InvalidRequest(
                "Xenolith requires base_url=unix:///absolute/socket/path".into(),
            )
        })?;
    let socket = PathBuf::from(socket);
    let state_dir = std::env::var_os("XENOLITH_STATE_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            CodexErrorDetails::InvalidRequest(
                "set XENOLITH_STATE_DIR to an isolated absolute directory".into(),
            )
        })?;
    let input = backend::Prompt {
        model: model.slug.clone(),
        instructions: prompt.base_instructions.text.clone(),
        items: prompt
            .input
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<_, _>>()?,
        tools: prompt
            .tools
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<_, _>>()?,
        max_tokens: generation.max_tokens,
        context_window: None,
        reasoning: generation.reasoning,
    };
    let (tx, rx_event) = mpsc::channel(128);
    let consumer_dropped = CancellationToken::new();
    let cancel = consumer_dropped.clone();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancelled_for_watch = cancelled.clone();
    let watch_finished = CancellationToken::new();
    let finished = watch_finished.clone();
    let dropped = cancel.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = dropped.cancelled() => cancelled_for_watch.store(true, std::sync::atomic::Ordering::Release),
            _ = finished.cancelled() => {},
        }
    });
    tokio::task::spawn_blocking(move || {
        let result =
            backend::run_cancellable(&socket, &state_dir, &thread, input, cancelled, |output| {
                if cancel.is_cancelled() {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "native consumer cancelled",
                    ));
                }
                let event = match output {
                    Output::Local(value) => {
                        tx.blocking_send(Ok(ResponseEvent::XenolithInference(value)))
                            .map_err(|_| {
                                io::Error::new(
                                    io::ErrorKind::BrokenPipe,
                                    "native consumer disconnected",
                                )
                            })?;
                        return Ok(());
                    }
                    Output::ItemAdded(value) => {
                        ResponseEvent::OutputItemAdded(serde_json::from_value(value)?)
                    }
                    Output::ItemDone(value) => {
                        ResponseEvent::OutputItemDone(serde_json::from_value(value)?)
                    }
                    Output::TextDelta(text) => ResponseEvent::OutputTextDelta(text),
                    Output::ReasoningDelta(delta) => ResponseEvent::ReasoningContentDelta {
                        delta,
                        content_index: 0,
                    },
                    Output::Completed {
                        id,
                        usage,
                        end_turn,
                    } => ResponseEvent::Completed {
                        response_id: id,
                        token_usage: Some(TokenUsage {
                            input_tokens: (usage.input + usage.cache_read) as i64,
                            cached_input_tokens: usage.cache_read as i64,
                            cache_write_input_tokens: 0,
                            output_tokens: usage.output as i64,
                            reasoning_output_tokens: usage.reasoning as i64,
                            total_tokens: (usage.input + usage.cache_read + usage.output) as i64,
                            codex_rollout_budget_units: None,
                        }),
                        usage_metadata: None,
                        end_turn: Some(end_turn),
                    },
                };
                tx.blocking_send(Ok(event)).map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "native consumer disconnected")
                })
            });
        watch_finished.cancel();
        if let Err(error) = result {
            // Native mutations must not use Codex's automatic HTTP retries.
            let details = if cancel.is_cancelled() {
                CodexErrorDetails::TurnAborted
            } else if error
                .to_string()
                .contains("Xenolith context_length_exceeded:")
            {
                CodexErrorDetails::ContextWindowExceeded
            } else {
                CodexErrorDetails::Fatal(format!("Xenolith: {error}"))
            };
            let _ = tx.blocking_send(Err(details.into()));
        }
    });
    Ok(ResponseStream {
        rx_event,
        interrupt: None,
        consumer_dropped,
    })
}
