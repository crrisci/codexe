// Copyright (c) 2026 CodeXe contributors. Licensed under Apache-2.0.

use super::*;
use pretty_assertions::assert_eq;

fn apply_native_history_events(
    app: &mut App,
    tui: &mut crate::tui::Tui,
    events: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
) -> Result<usize> {
    let mut completed = 0;
    while let Ok(event) = events.try_recv() {
        match event {
            AppEvent::InsertHistoryCell(cell) => app.insert_history_cell(tui, cell),
            AppEvent::ConsolidateAgentMessage {
                source,
                copy_source,
                cwd,
                inline_visualization_context,
                scrollback_reflow,
                deferred_history_cell,
            } => {
                completed += 1;
                app.handle_consolidate_agent_message(
                    tui,
                    history_cell::AgentMarkdownCell::new_with_inline_visualizations(
                        source,
                        &cwd,
                        inline_visualization_context,
                    )
                    .with_copy_source(copy_source),
                    scrollback_reflow,
                    deferred_history_cell,
                )?;
                app.chat_widget.note_stream_consolidation_completed();
            }
            _ => {}
        }
    }
    Ok(completed)
}

#[tokio::test]
async fn xenolith_checkpoint_never_splits_or_duplicates_a_streamed_answer() -> Result<()> {
    const PREFIX: &str = "NATIVE_PREFIX_7f1.\n\n";
    const TAIL: &str = "NATIVE_TAIL_e9c.";
    let answer = format!("{PREFIX}{TAIL}");
    for phase in ["checkpoint", "rewind"] {
        for (before, after) in [(TAIL, ""), ("", TAIL), ("", "")] {
            let (mut app, mut events, _ops) = make_test_app_with_channels().await;
            let mut tui = crate::tui::test_support::make_test_tui()?;
            let thread = ThreadId::new();
            app.chat_widget
                .handle_thread_session_quiet(test_thread_session(
                    thread,
                    app.config.cwd.to_path_buf(),
                ));
            while events.try_recv().is_ok() {}
            app.chat_widget
                .handle_server_notification(turn_started_notification(thread, "native-turn"), None);
            app.chat_widget.handle_server_notification(
                agent_message_delta_notification(thread, "native-turn", "native-answer", PREFIX),
                None,
            );
            for _ in 0..10 {
                app.chat_widget.on_commit_tick();
            }
            apply_native_history_events(&mut app, &mut tui, &mut events)?;
            assert!(
                app.transcript_cells
                    .iter()
                    .any(|cell| { cell.as_any().is::<history_cell::AgentMessageCell>() }),
                "The prefix must already be committed before native telemetry arrives"
            );
            if !before.is_empty() {
                app.chat_widget.handle_server_notification(
                    agent_message_delta_notification(
                        thread,
                        "native-turn",
                        "native-answer",
                        before,
                    ),
                    None,
                );
            }
            app.chat_widget.handle_server_notification(
                ServerNotification::XenolithInference(
                    codex_app_server_protocol::XenolithInferenceNotification {
                        thread_id: thread.to_string(),
                        turn_id: "native-turn".into(),
                        phase: phase.into(),
                        details: "NATIVE_TELEMETRY_ONLY".into(),
                    },
                ),
                None,
            );
            if !after.is_empty() {
                app.chat_widget.handle_server_notification(
                    agent_message_delta_notification(thread, "native-turn", "native-answer", after),
                    None,
                );
            }
            app.chat_widget.handle_server_notification(
                ServerNotification::ItemCompleted(
                    codex_app_server_protocol::ItemCompletedNotification {
                        thread_id: thread.to_string(),
                        turn_id: "native-turn".into(),
                        completed_at_ms: 0,
                        item: ThreadItem::AgentMessage {
                            id: "native-answer".into(),
                            text: answer.clone(),
                            phase: Some(codex_protocol::models::MessagePhase::FinalAnswer),
                            memory_citation: None,
                            delivery: None,
                            questions: None,
                        },
                    },
                ),
                None,
            );
            app.chat_widget.handle_server_notification(
                turn_completed_notification(thread, "native-turn", TurnStatus::Completed),
                None,
            );
            assert_eq!(
                apply_native_history_events(&mut app, &mut tui, &mut events)?,
                1
            );
            for width in [80, 24, 80] {
                let rendered = app
                    .render_transcript_lines_for_reflow(width)
                    .lines
                    .iter()
                    .map(rendered_line_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_eq!(
                    rendered.matches("NATIVE_PREFIX_7f1.").count(),
                    1,
                    "{rendered}"
                );
                assert_eq!(rendered.matches(TAIL).count(), 1, "{rendered}");
                assert!(!rendered.contains("NATIVE_TELEMETRY_ONLY"), "{rendered}");
                assert!(!rendered.contains("Xenolith"), "{rendered}");
            }
            assert_eq!(
                app.transcript_cells
                    .iter()
                    .filter(|cell| { cell.as_any().is::<history_cell::AgentMarkdownCell>() })
                    .count(),
                1
            );
        }
    }
    Ok(())
}
