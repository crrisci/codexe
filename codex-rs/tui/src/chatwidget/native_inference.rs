//! Native measurements have their own footer, never transcript or reasoning status.

use super::*;
use codex_app_server_protocol::XenolithInferenceNotification;

#[derive(Debug)]
pub(super) struct NativeInferenceState {
    thread_id: String,
    turn_id: String,
    summary: Option<String>,
    completed: bool,
    image_started: Option<std::time::Instant>,
}

impl ChatWidget {
    pub(super) fn on_native_inference(&mut self, notification: XenolithInferenceNotification) {
        if self.thread_id().map(|id| id.to_string()).as_deref()
            != Some(notification.thread_id.as_str())
            || self.turn_lifecycle.last_turn_id.as_deref() != Some(notification.turn_id.as_str())
            || !self.turn_lifecycle.agent_turn_running
        {
            return;
        }
        let label = match notification.phase.as_str() {
            "prefill" => "Prefill",
            "decode" => "Generating",
            "preparing" => "Preparing",
            "preparing_image" => "Preparing image",
            "finalizing" => "Finalizing",
            "checkpoint" => "",
            // Connection, session, rebuild and rewind reports are diagnostics.
            // They must not displace measured progress or enter conversation history.
            _ => return,
        };
        let details = notification.details.trim();
        if notification.phase == "checkpoint" && details.is_empty() {
            self.clear_native_inference();
            return;
        }
        let message = match (label.is_empty(), details.is_empty()) {
            (true, true) => return,
            (true, false) => details.to_owned(),
            (false, true) => label.to_owned(),
            (false, false) => format!("{label} · {details}"),
        };
        self.status_state.native_inference = Some(NativeInferenceState {
            thread_id: notification.thread_id,
            turn_id: notification.turn_id,
            summary: (notification.phase == "checkpoint").then(|| details.to_owned()),
            completed: false,
            image_started: (notification.phase == "preparing_image").then(std::time::Instant::now),
        });
        self.bottom_pane.set_native_inference(Some(message));
        self.refresh_native_image_time_at(std::time::Instant::now());
        self.request_redraw();
    }

    pub(super) fn refresh_native_image_time_at(&mut self, now: std::time::Instant) {
        let Some(started) = self
            .status_state
            .native_inference
            .as_ref()
            .and_then(|state| state.image_started)
        else {
            return;
        };
        let seconds = now.saturating_duration_since(started).as_secs();
        self.bottom_pane
            .set_native_inference(Some(format!("Preparing image · {seconds}s")));
        self.frame_requester
            .schedule_frame_in(std::time::Duration::from_secs(1));
    }

    pub(super) fn clear_native_inference(&mut self) {
        self.status_state.native_inference = None;
        self.bottom_pane.set_native_inference(None);
    }

    pub(super) fn clear_active_native_inference(&mut self) {
        if self
            .status_state
            .native_inference
            .as_ref()
            .is_some_and(|state| !state.completed)
        {
            self.clear_native_inference();
        }
    }

    pub(super) fn finish_native_inference(
        &mut self,
        thread_id: &str,
        turn_id: &str,
        success: bool,
    ) {
        let Some(state) = self.status_state.native_inference.as_mut() else {
            return;
        };
        if state.thread_id != thread_id
            || state.turn_id != turn_id
            || self.turn_lifecycle.last_turn_id.as_deref() != Some(turn_id)
        {
            return;
        }
        if success && let Some(summary) = &state.summary {
            state.completed = true;
            self.bottom_pane
                .set_native_inference(Some(format!("Completed · {summary}")));
        } else {
            self.clear_native_inference();
        }
    }
}
