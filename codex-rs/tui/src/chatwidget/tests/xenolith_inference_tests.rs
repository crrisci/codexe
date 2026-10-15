// Copyright (c) 2026 CodeXe contributors. Licensed under Apache-2.0.

use super::*;

fn native(thread: ThreadId, turn: &str, phase: &str, details: &str) -> ServerNotification {
    ServerNotification::XenolithInference(
        codex_app_server_protocol::XenolithInferenceNotification {
            thread_id: thread.to_string(),
            turn_id: turn.into(),
            phase: phase.into(),
            details: details.into(),
        },
    )
}

fn history_text(events: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>) -> String {
    drain_insert_history(events)
        .into_iter()
        .flatten()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn native_decode_remains_visible_during_stream_commits_and_reasoning_updates() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(None).await;
    let thread = ThreadId::new();
    chat.thread_id = Some(thread);
    handle_turn_started(&mut chat, "turn-1");
    chat.handle_server_notification(
        native(thread, "turn-1", "decode", "NATIVE_LIVE_23_TOKENS"),
        None,
    );
    assert!(render_bottom_popup(&chat, 80).contains("NATIVE_LIVE_23_TOKENS"));

    handle_agent_message_delta(&mut chat, "A streamed paragraph.\n\n");
    for _ in 0..10 {
        chat.run_commit_tick();
    }
    assert!(render_bottom_popup(&chat, 80).contains("NATIVE_LIVE_23_TOKENS"));
    handle_agent_reasoning_delta(&mut chat, "**Inspecting the file**");
    let rendered = render_bottom_popup(&chat, 80);
    assert!(rendered.contains("Xenolith"), "{rendered}");
    assert!(rendered.contains("NATIVE_LIVE_23_TOKENS"), "{rendered}");
    assert!(!history_text(&mut events).contains("NATIVE_LIVE_23_TOKENS"));
}

#[tokio::test]
async fn native_panel_ignores_foreign_threads_stale_turns_and_replayed_telemetry() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(None).await;
    let thread = ThreadId::new();
    chat.thread_id = Some(thread);
    handle_turn_started(&mut chat, "turn-1");
    chat.handle_server_notification(
        native(thread, "turn-1", "prefill", "CURRENT_NATIVE_PROGRESS"),
        None,
    );
    chat.handle_server_notification(
        native(ThreadId::new(), "turn-1", "checkpoint", "FOREIGN_THREAD"),
        None,
    );
    chat.handle_server_notification(native(thread, "old-turn", "checkpoint", "STALE_TURN"), None);
    chat.handle_server_notification(
        native(thread, "turn-1", "checkpoint", "REPLAYED_TELEMETRY"),
        Some(ReplayKind::ResumeInitialMessages),
    );
    let rendered = render_bottom_popup(&chat, 80);
    assert!(rendered.contains("CURRENT_NATIVE_PROGRESS"), "{rendered}");
    for ignored in ["FOREIGN_THREAD", "STALE_TURN", "REPLAYED_TELEMETRY"] {
        assert!(!rendered.contains(ignored), "{rendered}");
    }
    let history = history_text(&mut events);
    assert!(!history.contains("CURRENT_NATIVE_PROGRESS"));
    assert!(!history.contains("TELEMETRY"));
    assert!(!history.contains("STALE_TURN"));
    assert!(!history.contains("FOREIGN_THREAD"));
}

#[tokio::test]
async fn native_live_progress_clears_on_cancel_error_and_turn_completion() {
    for ending in ["cancel", "error", "complete", "reset", "disconnect"] {
        let (mut chat, _events, _ops) = make_chatwidget_manual(None).await;
        let thread = ThreadId::new();
        chat.thread_id = Some(thread);
        handle_turn_started(&mut chat, "turn-1");
        chat.handle_server_notification(
            native(thread, "turn-1", "decode", "EPHEMERAL_NATIVE_PROGRESS"),
            None,
        );
        assert!(render_bottom_popup(&chat, 80).contains("EPHEMERAL_NATIVE_PROGRESS"));
        match ending {
            "cancel" => handle_turn_interrupted(&mut chat, "turn-1"),
            "error" => handle_error(&mut chat, "Native generation failed", None),
            "complete" => handle_turn_completed(&mut chat, "turn-1", None),
            "reset" => chat.reset_after_prompt_revert(None, &[]),
            "disconnect" => chat.pause_for_disconnect(),
            _ => unreachable!(),
        }
        assert!(
            !render_bottom_popup(&chat, 80).contains("EPHEMERAL_NATIVE_PROGRESS"),
            "{ending}"
        );
    }
}

#[tokio::test]
async fn native_checkpoint_summary_is_transient_panel_content_never_answer_history() {
    let (mut chat, mut events, _ops) = make_chatwidget_manual(None).await;
    let thread = ThreadId::new();
    chat.thread_id = Some(thread);
    handle_turn_started(&mut chat, "turn-1");
    handle_agent_message_delta(&mut chat, "Only one assistant answer.\n");
    chat.run_commit_tick();
    chat.handle_server_notification(
        native(thread, "turn-1", "checkpoint", "NATIVE_SAVED_SUMMARY"),
        None,
    );
    chat.handle_server_notification(
        native(thread, "turn-1", "rewind", "NATIVE_REWIND_DETAIL"),
        None,
    );
    complete_assistant_message(
        &mut chat,
        "msg-1",
        "Only one assistant answer.",
        Some(MessagePhase::FinalAnswer),
    );
    handle_turn_completed(&mut chat, "turn-1", None);
    let rendered = render_bottom_popup(&chat, 80);
    assert!(rendered.contains("NATIVE_SAVED_SUMMARY"), "{rendered}");
    assert!(!rendered.contains("NATIVE_REWIND_DETAIL"), "{rendered}");
    let history = history_text(&mut events);
    assert!(!history.contains("NATIVE_SAVED_SUMMARY"), "{history}");
    assert!(!history.contains("NATIVE_REWIND_DETAIL"), "{history}");
    handle_turn_started(&mut chat, "turn-2");
    assert!(!render_bottom_popup(&chat, 80).contains("NATIVE_SAVED_SUMMARY"));
    chat.handle_server_notification(
        native(thread, "turn-1", "checkpoint", "STALE_SUMMARY"),
        None,
    );
    assert!(!render_bottom_popup(&chat, 80).contains("STALE_SUMMARY"));
}
