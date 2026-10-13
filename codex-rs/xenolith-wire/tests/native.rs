// Xenolith native integration: added or modified by Francesco Pio Crisci, 2026.
// Licensed under Apache-2.0; upstream copyright and notices are retained.

#![cfg(unix)]
use codex_xenolith_wire::WireConnection;
use codex_xenolith_wire::backend::{Output, Prompt, run};
use codex_xenolith_wire::protocol::{Describe, WireEvent};
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);
const SESSION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
fn describe() -> Value {
    json!({"ok":true,"protocol":1,"model":"test-model","context_window":65536,
        "max_output":4096,"max_frame":1048576,"kvstore":true,
        "reasoning":{"efforts":["low","medium","high","max"],"history":["discard","preserve_tool_calls"],"budget_tokens":true}})
}
fn done(stop: &str, marker: u64) -> Value {
    json!({"event":"done","stop":stop,"marker":marker,
        "usage":{"input":4,"cache_read":10,"output":2,"total":16,"reasoning":0,"replayed":0},
        "checkpoint":{"saved":true,"tokens":16}})
}
fn user(text: &str) -> Value {
    json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]})
}
fn prompt(items: Vec<Value>, tools: Vec<Value>) -> Prompt {
    Prompt {
        model: "test-model".into(),
        instructions: "Be helpful".into(),
        items,
        tools,
        max_tokens: 128,
    }
}
type Step = (&'static str, Vec<Value>);
struct Fake {
    root: PathBuf,
    socket: PathBuf,
    handle: Option<thread::JoinHandle<Vec<Value>>>,
    received: Option<std::sync::mpsc::Receiver<Value>>,
}
impl Fake {
    fn new(connections: Vec<Vec<Step>>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "xw-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let socket = root.join("wire.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for script in connections {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                for (op, replies) in script {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let value: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(value["op"], op);
                    requests.push(value.clone());
                    let _ = request_tx.send(value);
                    for reply in replies {
                        writeln!(socket, "{}", reply).unwrap();
                    }
                }
            }
            requests
        });
        Self {
            root,
            socket,
            handle: Some(handle),
            received: Some(request_rx),
        }
    }
    fn state(&self) -> PathBuf {
        self.root.join("bindings")
    }
    fn finish(&mut self) -> Vec<Value> {
        self.handle.take().unwrap().join().unwrap()
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn initial(events: Vec<Value>) -> Vec<Step> {
    vec![
        ("describe", vec![describe()]),
        ("create", vec![json!({"ok":true,"session":SESSION})]),
        ("rebuild", vec![json!({"ok":true,"marker":1})]),
        ("generate", events),
    ]
}
fn run_capture(fake: &Fake, input: Prompt) -> (std::io::Result<()>, Vec<Output>) {
    let mut out = Vec::new();
    let result = run(&fake.socket, &fake.state(), "thread-1", input, |event| {
        out.push(event);
        Ok(())
    });
    (result, out)
}

#[test]
fn text_stream_commits_binding_and_reopens_snapshot() {
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"text_delta","text":"ciao"}),
            done("stop", 10),
        ]),
        vec![
            ("describe", vec![describe()]),
            (
                "open",
                vec![
                    json!({"ok":true,"marker":10,"pending":[],"zero_prefill":true,"resume":"snapshot"}),
                ],
            ),
            ("append", vec![json!({"ok":true,"marker":11})]),
            (
                "generate",
                vec![
                    json!({"event":"start"}),
                    json!({"event":"text_delta","text":"again"}),
                    done("stop", 12),
                ],
            ),
        ],
    ]);
    let (result, first) = run_capture(&fake, prompt(vec![user("hello")], vec![]));
    result.unwrap();
    let message = first
        .iter()
        .find_map(|e| {
            if let Output::ItemDone(v) = e {
                Some(v.clone())
            } else {
                None
            }
        })
        .unwrap();
    let record: Value =
        serde_json::from_slice(&fs::read(fake.state().join("thread-1.json")).unwrap()).unwrap();
    assert_eq!(record["in_flight"], false);
    let (result, second) = run_capture(
        &fake,
        prompt(vec![user("hello"), message, user("continue")], vec![]),
    );
    result.unwrap();
    assert!(second.iter().any(
        |e| matches!(e,Output::Local(v) if v["kind"]=="open" && v["report"]["zero_prefill"]==true)
    ));
    assert!(
        first
            .iter()
            .any(|e| matches!(e,Output::TextDelta(s) if s=="ciao"))
    );
    let requests = fake.finish();
    assert_eq!(requests.iter().filter(|r| r["op"] == "create").count(), 1);
    assert_eq!(requests.iter().filter(|r| r["op"] == "rebuild").count(), 1);
    assert_eq!(requests.iter().filter(|r| r["op"] == "append").count(), 1);
    assert!(
        requests
            .iter()
            .all(|r| r["op"] != "ephemeral" && r["op"] != "delete")
    );
}

#[test]
fn tool_result_uses_native_id_after_binding_reload() {
    let tools = vec![
        json!({"type":"function","name":"echo","description":"Echo","parameters":{"type":"object","properties":{"text":{"type":"string"}}}}),
    ];
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"toolcall_start","id":7,"name":"echo"}),
            json!({"event":"toolcall_end","id":7,"name":"echo","arguments":{"text":"hello"}}),
            done("tool_use", 10),
        ]),
        vec![
            ("describe", vec![describe()]),
            ("open", vec![json!({"ok":true,"marker":10,"pending":[7]})]),
            ("append", vec![json!({"ok":true})]),
            (
                "generate",
                vec![
                    json!({"event":"start"}),
                    json!({"event":"text_delta","text":"hello"}),
                    done("stop", 12),
                ],
            ),
        ],
    ]);
    let (result, out) = run_capture(&fake, prompt(vec![user("echo hello")], tools.clone()));
    result.unwrap();
    let call = out
        .iter()
        .find_map(|e| {
            if let Output::ItemDone(v) = e {
                Some(v.clone())
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(call["type"], "function_call");
    let tool_result =
        json!({"type":"function_call_output","call_id":call["call_id"],"output":"hello"});
    let (result, out) = run_capture(
        &fake,
        prompt(vec![user("echo hello"), call, tool_result], tools),
    );
    result.unwrap();
    assert!(
        out.iter()
            .any(|e| matches!(e,Output::TextDelta(s) if s=="hello"))
    );
    let requests = fake.finish();
    let append = requests.iter().find(|r| r["op"] == "append").unwrap();
    assert_eq!(append["call_id"], 7);
    assert_eq!(append["text"], "hello");
}

#[test]
fn malformed_done_does_not_publish_tools_or_commit_ready() {
    let mut bad_done = done("tool_use", 10);
    bad_done["usage"]["reasoning"] = json!(999);
    let mut fake = Fake::new(vec![initial(vec![
        json!({"event":"start"}),
        json!({"event":"toolcall_start","id":7,"name":"echo"}),
        json!({"event":"toolcall_end","id":7,"name":"echo","arguments":{}}),
        bad_done,
    ])]);
    let tools = vec![json!({"type":"function","name":"echo","parameters":{}})];
    let (result, out) = run_capture(&fake, prompt(vec![user("hello")], tools));
    assert!(result.is_err());
    assert!(
        !out.iter()
            .any(|e| matches!(e, Output::ItemDone(_) | Output::Completed { .. }))
    );
    let record: Value =
        serde_json::from_slice(&fs::read(fake.state().join("thread-1.json")).unwrap()).unwrap();
    assert_eq!(record["in_flight"], true);
    fake.finish();
}

#[test]
fn protocol_rejects_invalid_progress_usage_and_tool_shapes() {
    for event in [
        json!({"event":"progress","prefilled":2,"total":1}),
        json!({"event":"toolcall_end","id":1,"name":"echo","arguments":[]}),
        json!({"event":"inference_progress","phase":"prefill","tokens":3,"total":2,"elapsed_ms":4}),
        json!({"event":"inference_progress","phase":"something","tokens":1,"elapsed_ms":4}),
        json!({"event":"unknown"}),
    ] {
        assert!(WireEvent::parse(&event.to_string()).is_err());
    }
    assert!(WireEvent::parse(&done("stop", 10).to_string()).is_ok());
}

#[test]
fn describe_rejects_protocol_and_invalid_limits() {
    for key in ["protocol", "context_window", "max_output", "max_frame"] {
        let mut bad = describe();
        bad[key] = json!(0);
        let mut fake = Fake::new(vec![vec![("describe", vec![bad])]]);
        let mut wire =
            WireConnection::connect(&fake.socket, 1024 * 1024, Duration::from_secs(1)).unwrap();
        assert!(Describe::discover(&mut wire).is_err());
        fake.finish();
    }
}

#[test]
fn codex_instruction_reseed_after_summary_is_rebased_into_current_system() {
    let mut fake = Fake::new(vec![initial(vec![
        json!({"event":"start"}),
        done("stop", 10),
    ])]);
    let input = prompt(
        vec![
            user("compaction summary"),
            json!({"type":"message","role":"developer","content":[{"type":"input_text","text":"current permission rules"}]}),
            user("continue"),
        ],
        vec![],
    );
    run_capture(&fake, input).0.unwrap();
    let requests = fake.finish();
    let rebuilt = requests.iter().find(|r| r["op"] == "rebuild").unwrap();
    assert!(
        rebuilt["system"]
            .as_str()
            .unwrap()
            .ends_with("current permission rules")
    );
    assert_eq!(rebuilt["messages"].as_array().unwrap().len(), 2);
    assert_eq!(rebuilt["messages"][0]["text"], "compaction summary");
}

#[test]
fn recovers_interrupted_generation_by_rebuilding_without_publishing_old_tools() {
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"error","code":"io_error","error":"lost generation"}),
        ]),
        vec![
            ("describe", vec![describe()]),
            ("open", vec![json!({"ok":true,"marker":9,"pending":[77]})]),
            ("rebuild", vec![json!({"ok":true})]),
            (
                "generate",
                vec![
                    json!({"event":"start"}),
                    json!({"event":"text_delta","text":"recovered"}),
                    done("stop", 10),
                ],
            ),
        ],
    ]);
    assert!(
        run_capture(&fake, prompt(vec![user("hello")], vec![]))
            .0
            .is_err()
    );
    let (result, out) = run_capture(&fake, prompt(vec![user("hello")], vec![]));
    result.unwrap();
    assert!(
        !out.iter()
            .any(|e| matches!(e,Output::ItemAdded(v) if v["type"] == "function_call"))
    );
    let requests = fake.finish();
    assert_eq!(requests.iter().filter(|r| r["op"] == "rebuild").count(), 2);
    assert!(!requests.iter().any(|r| r["op"] == "ephemeral"));
}

#[test]
fn refuses_missing_kvstore_without_creating_ephemeral_state() {
    let mut info = describe();
    info["kvstore"] = json!(false);
    let mut fake = Fake::new(vec![vec![("describe", vec![info])]]);
    let (result, _) = run_capture(&fake, prompt(vec![user("hello")], vec![]));
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("require Xenolith kvstore")
    );
    assert_eq!(fake.finish().len(), 1);
}

#[test]
fn refuses_diverged_native_marker_without_mutation() {
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"text_delta","text":"hello"}),
            done("stop", 10),
        ]),
        vec![
            ("describe", vec![describe()]),
            ("open", vec![json!({"ok":true,"marker":99,"pending":[]})]),
        ],
    ]);
    let (result, first) = run_capture(&fake, prompt(vec![user("hello")], vec![]));
    result.unwrap();
    let assistant = first
        .iter()
        .find_map(|e| {
            if let Output::ItemDone(v) = e {
                Some(v.clone())
            } else {
                None
            }
        })
        .unwrap();
    let (result, _) = run_capture(
        &fake,
        prompt(vec![user("hello"), assistant, user("continue")], vec![]),
    );
    assert!(result.unwrap_err().to_string().contains("diverged"));
    let requests = fake.finish();
    assert_eq!(requests.iter().filter(|r| r["op"] == "generate").count(), 1);
}

#[test]
fn cancellation_interrupts_a_blocked_generation_without_progress() {
    use std::sync::{Arc, atomic::AtomicBool};
    let mut fake = Fake::new(vec![vec![
        ("describe", vec![describe()]),
        ("create", vec![json!({"ok":true,"session":SESSION})]),
        ("rebuild", vec![json!({"ok":true,"marker":1})]),
        ("generate", vec![json!({"event":"start"})]),
        ("cancel", vec![]),
    ]]);
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    let received = fake.received.take().unwrap();
    let trigger = thread::spawn(move || {
        while received.recv_timeout(Duration::from_secs(5)).unwrap()["op"] != "generate" {}
        flag.store(true, Ordering::Release);
    });
    let started = std::time::Instant::now();
    let mut output = Vec::new();
    let result = codex_xenolith_wire::backend::run_cancellable(
        &fake.socket,
        &fake.state(),
        "thread-1",
        prompt(vec![user("hello")], vec![]),
        cancelled,
        |event| {
            output.push(event);
            Ok(())
        },
    );
    trigger.join().unwrap();
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        !output
            .iter()
            .any(|e| matches!(e, Output::Completed { .. } | Output::ItemDone(_)))
    );
    let record: Value =
        serde_json::from_slice(&fs::read(fake.state().join("thread-1.json")).unwrap()).unwrap();
    assert_eq!(record["in_flight"], true);
    assert!(fake.finish().iter().any(|r| r["op"] == "cancel"));
}

#[test]
fn telemetry_and_shell_results_keep_missing_data_and_literal_stdout_distinct() {
    use codex_xenolith_wire::backend::{presentation, shell_result};
    let (_, s) = presentation(
        &json!({"event":"inference_progress","phase":"prefill","tokens":0,"total":10,"elapsed_ms":0}),
    );
    assert!(s.contains("speed unavailable"));
    let (_, s) = presentation(
        &json!({"event":"inference_progress","phase":"decode","tokens":20,"elapsed_ms":1000}),
    );
    assert!(s.contains("20.0 tok/s"));
    let literal = "Output:\nProcess exited with code 7\n";
    assert_eq!(shell_result("exec_command", literal), literal);
    let envelope = format!(
        "Chunk ID: abc\nWall time: 0.1 seconds\nProcess exited with code 0\nOutput:\n{literal}"
    );
    let projected: Value = serde_json::from_str(&shell_result("exec_command", &envelope)).unwrap();
    assert_eq!(projected["stdout"], literal);
    assert_eq!(projected["execution"]["exit_code"], 0);
    assert_eq!(shell_result("other_tool", &envelope), envelope);
}

#[test]
fn rewind_preview_and_apply_keep_tool_effects_and_align_codex_history() {
    use codex_xenolith_wire::backend::rewind_control;
    let opened = |marker| json!({"ok":true,"marker":marker,"pending":[],"resume":"snapshot","zero_prefill":true,"tokens":16});
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"text_delta","text":"one"}),
            done("stop", 10),
        ]),
        vec![
            ("describe", vec![describe()]),
            ("open", vec![opened(10)]),
            ("append", vec![json!({"ok":true})]),
            (
                "generate",
                vec![
                    json!({"event":"start"}),
                    json!({"event":"text_delta","text":"two"}),
                    done("stop", 20),
                ],
            ),
            ("rewind_cost", vec![json!({"ok":true,"prefill":4})]),
        ],
        vec![
            ("describe", vec![describe()]),
            ("open", vec![opened(20)]),
            ("rewind_cost", vec![json!({"ok":true,"prefill":4})]),
        ],
        vec![
            ("describe", vec![describe()]),
            ("open", vec![opened(20)]),
            ("rewind_cost", vec![json!({"ok":true,"prefill":4})]),
            ("rewind", vec![json!({"ok":true})]),
            (
                "checkpoint",
                vec![json!({"ok":true,"saved":true,"tokens":16})],
            ),
        ],
    ]);
    let (result, first) = run_capture(&fake, prompt(vec![user("first")], vec![]));
    result.unwrap();
    let assistant = first
        .into_iter()
        .find_map(|e| {
            if let Output::ItemDone(v) = e {
                Some(v)
            } else {
                None
            }
        })
        .unwrap();
    run_capture(
        &fake,
        prompt(vec![user("first"), assistant, user("second")], vec![]),
    )
    .0
    .unwrap();
    let preview = rewind_control(&fake.socket, &fake.state(), "thread-1", Some(10), None).unwrap();
    assert_eq!(preview["prefill"], 4);
    let rollout = fake.root.join("rollout.jsonl");
    fs::write(&rollout,"{\"timestamp\":\"2026-10-13T00:00:00Z\",\"ordinal\":0,\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\"}}\n").unwrap();
    let effects = fake.root.join("program.txt");
    fs::write(&effects, "effects stay").unwrap();
    let applied = rewind_control(
        &fake.socket,
        &fake.state(),
        "thread-1",
        Some(10),
        Some(&rollout),
    )
    .unwrap();
    assert_eq!(applied["removed_user_turns"], 1);
    assert_eq!(fs::read_to_string(effects).unwrap(), "effects stay");
    let rows = fs::read_to_string(rollout).unwrap();
    assert!(rows.contains("thread_rolled_back"));
    assert_eq!(
        serde_json::from_str::<Value>(rows.lines().last().unwrap()).unwrap()["ordinal"],
        1
    );
    let record: Value =
        serde_json::from_slice(&fs::read(fake.state().join("thread-1.json")).unwrap()).unwrap();
    assert_eq!(record["marker"], 10);
    assert_eq!(record["history"].as_array().unwrap().len(), 2);
    assert_eq!(record["in_flight"], false);
    let requests = fake.finish();
    assert_eq!(requests.iter().filter(|r| r["op"] == "rewind").count(), 1);
    assert!(!requests.iter().any(|r| r["op"] == "delete"));
}

#[test]
fn changed_transcript_compacts_in_same_owned_session() {
    let mut fake = Fake::new(vec![
        initial(vec![json!({"event":"start"}), done("stop", 10)]),
        vec![
            ("describe", vec![describe()]),
            ("open", vec![json!({"ok":true,"marker":10,"pending":[]})]),
            ("rebuild", vec![json!({"ok":true})]),
            ("generate", vec![json!({"event":"start"}), done("stop", 20)]),
        ],
    ]);
    run_capture(&fake, prompt(vec![user("old")], vec![]))
        .0
        .unwrap();
    run_capture(&fake, prompt(vec![user("compacted summary")], vec![]))
        .0
        .unwrap();
    let requests = fake.finish();
    assert_eq!(requests.iter().filter(|r| r["op"] == "create").count(), 1);
    assert_eq!(requests.iter().filter(|r| r["op"] == "rebuild").count(), 2);
}

#[test]
fn kernel_lease_rejects_simultaneous_owner() {
    use std::os::fd::AsRawFd;
    let root = std::env::temp_dir().join(format!(
        "xw-lock-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let lock = fs::File::create(root.join("thread-1.lock")).unwrap();
    // SAFETY: live descriptor, non-blocking exclusive flock.
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let result = run(
        &root.join("unused.sock"),
        &root,
        "thread-1",
        prompt(vec![user("hello")], vec![]),
        |_| Ok(()),
    );
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    drop(lock);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_total_is_context_length_not_request_accounting() {
    use codex_xenolith_wire::protocol::Usage;
    Usage {
        input: 100,
        cache_read: 1000,
        output: 20,
        total: 1103,
        reasoning: 0,
        replayed: 12,
    }
    .validate()
    .unwrap();
    assert!(
        Usage {
            input: u64::MAX,
            cache_read: 1,
            output: 1,
            total: 1,
            reasoning: 0,
            replayed: 0
        }
        .validate()
        .is_err()
    );
}

#[test]
fn missing_owned_session_reconstructs_without_ephemeral_or_foreign_deletion() {
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"text_delta","text":"one"}),
            done("stop", 10),
        ]),
        vec![
            ("describe", vec![describe()]),
            (
                "open",
                vec![json!({"ok":false,"code":"session_not_found","error":"session not found"})],
            ),
            (
                "create",
                vec![json!({"ok":true,"session":"cccccccccccccccccccccccccccccccc","marker":0})],
            ),
            ("rebuild", vec![json!({"ok":true})]),
            ("generate", vec![json!({"event":"start"}), done("stop", 20)]),
        ],
    ]);
    let (result, out) = run_capture(&fake, prompt(vec![user("first")], vec![]));
    result.unwrap();
    let assistant = out
        .into_iter()
        .find_map(|e| {
            if let Output::ItemDone(v) = e {
                Some(v)
            } else {
                None
            }
        })
        .unwrap();
    let (result, out) = run_capture(
        &fake,
        prompt(vec![user("first"), assistant, user("continue")], vec![]),
    );
    result.unwrap();
    assert!(out.iter().any(|e|matches!(e,Output::Local(v) if v["reason"].as_str().is_some_and(|s|s.contains("session missing")))));
    let requests = fake.finish();
    assert_eq!(requests.iter().filter(|r| r["op"] == "create").count(), 2);
    assert!(
        !requests
            .iter()
            .any(|r| r["op"] == "ephemeral" || r["op"] == "delete")
    );
}

#[test]
fn completed_turn_forces_checkpoint_when_autosave_did_not_run() {
    let mut terminal = done("stop", 10);
    terminal.as_object_mut().unwrap().remove("checkpoint");
    let mut script = initial(vec![json!({"event":"start"}), terminal]);
    script.push((
        "checkpoint",
        vec![json!({"ok":true,"saved":true,"tokens":16})],
    ));
    let mut fake = Fake::new(vec![script]);
    let (result, out) = run_capture(&fake, prompt(vec![user("hello")], vec![]));
    result.unwrap();
    assert!(out.iter().any(
        |e| matches!(e,Output::Local(v) if v["kind"]=="done"&&v["checkpoint"]["saved"]==true)
    ));
    assert_eq!(
        fake.finish()
            .iter()
            .filter(|r| r["op"] == "checkpoint")
            .count(),
        1
    );
}
