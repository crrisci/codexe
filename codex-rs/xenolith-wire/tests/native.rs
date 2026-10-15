// Xenolith native integration: added or modified by Francesco Pio Crisci, 2026.
// Licensed under Apache-2.0; upstream copyright and notices are retained.

#![cfg(unix)]
use codex_xenolith_wire::WireConnection;
use codex_xenolith_wire::backend::{Output, Prompt, ReasoningOptions, run};
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
        context_window: None,
        instructions: "Be helpful".into(),
        items,
        tools,
        max_tokens: Some(128),
        reasoning: None,
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
    let mut record: Value =
        serde_json::from_slice(&fs::read(fake.state().join("thread-1.json")).unwrap()).unwrap();
    assert_eq!(record["in_flight"], false);
    // v1 bindings written before rebuild_required remain readable in place.
    record.as_object_mut().unwrap().remove("rebuild_required");
    fs::write(
        fake.state().join("thread-1.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
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
    assert_eq!(s, "0% · 0/10 tokens");
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
fn native_progress_separates_phase_completion_from_request_completion() {
    use codex_xenolith_wire::backend::presentation;
    let running = json!({"event":"inference_progress","phase":"prefill","state":"running","tokens":42,"total":100,"elapsed_ms":500});
    assert_eq!(
        presentation(&running),
        ("prefill".into(), "42% · 42/100 tokens · 84.0 tok/s".into())
    );
    let finished = json!({"event":"inference_progress","phase":"prefill","state":"finished","tokens":100,"total":100,"elapsed_ms":1000});
    assert_eq!(
        presentation(&finished),
        ("preparing".into(), "Prompt ready".into())
    );
    let decode = json!({"event":"inference_progress","phase":"decode","state":"finished","tokens":20,"elapsed_ms":1000});
    assert_eq!(
        presentation(&decode),
        ("finalizing".into(), "20 tokens · 20.0 tok/s".into())
    );
}

#[test]
fn native_progress_omits_unknown_rates_and_checkpoint_diagnostics() {
    use codex_xenolith_wire::backend::presentation;
    let cached = json!({"event":"inference_progress","phase":"prefill","state":"running","tokens":0,"total":0,"elapsed_ms":0});
    assert_eq!(
        presentation(&cached),
        ("prefill".into(), "Cached prompt".into())
    );
    let legacy = json!({"event":"progress","prefilled":3,"total":10});
    assert_eq!(
        presentation(&legacy),
        ("prefill".into(), "30% · 3/10 tokens".into())
    );
    let done = json!({"kind":"done","inference":{"prefill":{"tokens":100,"elapsed_ms":1000},"decode":{"tokens":20,"elapsed_ms":2000}},"usage":{"output":20,"cache_read":30},"checkpoint":{"saved":false,"reason":"PRIVATE_DIAGNOSTIC"},"resume":null});
    assert_eq!(
        presentation(&done),
        (
            "checkpoint".into(),
            "decode 10.0 tok/s · prefill 100.0 tok/s · 20 tokens · 30 cached".into()
        )
    );
    assert_eq!(
        presentation(&json!({"kind":"done"})),
        ("checkpoint".into(), String::new())
    );
    assert_eq!(
        presentation(&json!({"kind":"rewind_cost","reason":"PRIVATE_DIAGNOSTIC"})),
        ("rewind".into(), String::new())
    );
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

fn reasoning() -> ReasoningOptions {
    ReasoningOptions {
        effort: "high".into(),
        budget_tokens: Some(64),
        history: "preserve_tool_calls".into(),
    }
}

#[test]
fn output_default_and_reasoning_settings_follow_describe_without_reseeding() {
    let mut fake = Fake::new(vec![initial(vec![
        json!({"event":"start"}),
        done("stop", 10),
    ])]);
    let mut input = prompt(vec![user("hello")], vec![]);
    input.max_tokens = None;
    input.context_window = Some(65536);
    input.reasoning = Some(reasoning());
    run_capture(&fake, input).0.unwrap();
    let requests = fake.finish();
    let generate = requests.iter().find(|r| r["op"] == "generate").unwrap();
    assert_eq!(generate["max_tokens"], 4096);
    assert_eq!(
        generate["reasoning"],
        json!({"effort":"high","budget_tokens":64,"history":"preserve_tool_calls"})
    );
    for key in ["seed", "temperature", "top_p", "top_k"] {
        assert!(generate.get(key).is_none());
    }
}

#[test]
fn context_capacity_drift_requires_rediscovery_before_native_mutation() {
    let mut fake = Fake::new(vec![vec![("describe", vec![describe()])]]);
    let mut input = prompt(vec![user("hello")], vec![]);
    input.context_window = Some(32768);
    let (result, events) = run_capture(&fake, input);
    let message = result.unwrap_err().to_string();
    assert!(message.contains("context window changed since discovery"));
    assert!(message.contains("restart CodeXe to rediscover"));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Output::ItemAdded(_) | Output::Completed { .. }))
    );
    assert!(!fake.state().join("thread-1.json").exists());
    assert_eq!(fake.finish().len(), 1);
}

#[test]
fn invalid_output_and_reasoning_options_fail_before_session_mutation() {
    for case in 0..8 {
        let mut info = describe();
        let mut input = prompt(vec![user("hello")], vec![]);
        input.reasoning = Some(reasoning());
        match case {
            0 => input.max_tokens = Some(0),
            1 => input.max_tokens = Some(4097),
            2 => input.reasoning.as_mut().unwrap().effort = "xhigh".into(),
            3 => input.reasoning.as_mut().unwrap().history = "all".into(),
            4 => input.reasoning.as_mut().unwrap().budget_tokens = Some(i32::MAX as u64 + 1),
            5 => info["reasoning"]["efforts"] = json!(["low"]),
            6 => info["reasoning"]["history"] = json!(["discard"]),
            7 => info["reasoning"]["budget_tokens"] = json!(false),
            _ => unreachable!(),
        }
        let mut fake = Fake::new(vec![vec![("describe", vec![info])]]);
        assert!(run_capture(&fake, input).0.is_err(), "case {case}");
        assert_eq!(fake.finish().len(), 1);
    }
}

#[test]
fn reasoning_roundtrips_through_tool_resume_and_transcript_rebuild() {
    let tools = vec![json!({"type":"function","name":"echo","parameters":{}})];
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"reasoning_delta","text":"consider "}),
            json!({"event":"reasoning_delta","text":"the tool"}),
            json!({"event":"text_delta","text":"checking"}),
            json!({"event":"toolcall_start","id":7,"name":"echo"}),
            json!({"event":"toolcall_end","id":7,"name":"echo","arguments":{"text":"hello"}}),
            done("tool_use", 10),
        ]),
        vec![
            ("describe", vec![describe()]),
            (
                "open",
                vec![json!({"ok":true,"marker":10,"pending":[7],"resume":"snapshot"})],
            ),
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
        vec![
            ("describe", vec![describe()]),
            ("open", vec![json!({"ok":true,"marker":12,"pending":[]})]),
            ("rebuild", vec![json!({"ok":true})]),
            ("generate", vec![json!({"event":"start"}), done("stop", 14)]),
        ],
    ]);
    let mut first = prompt(vec![user("hello")], tools.clone());
    first.reasoning = Some(reasoning());
    let (result, events) = run_capture(&fake, first);
    result.unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Output::ReasoningDelta(text) if text == "consider "))
    );
    let mut items = vec![user("hello")];
    items.extend(events.into_iter().filter_map(|e| match e {
        Output::ItemDone(item) => Some(item),
        _ => None,
    }));
    assert_eq!(items[1]["type"], "reasoning");
    assert_eq!(items[1]["summary"], json!([]));
    assert_eq!(
        items[1]["content"][0],
        json!({"type":"reasoning_text","text":"consider the tool"})
    );
    items.push(
        json!({"type":"function_call_output","call_id":items[3]["call_id"],"output":"hello"}),
    );
    let mut second = prompt(items.clone(), tools.clone());
    second.reasoning = Some(reasoning());
    let (result, events) = run_capture(&fake, second);
    result.unwrap();
    items.extend(events.into_iter().filter_map(|e| match e {
        Output::ItemDone(item) => Some(item),
        _ => None,
    }));
    items.push(user("continue"));
    let mut compacted = prompt(items, tools);
    compacted.reasoning = Some(reasoning());
    compacted.instructions = "Current compaction instructions".into();
    run_capture(&fake, compacted).0.unwrap();
    let requests = fake.finish();
    let rebuilt = requests.iter().rfind(|r| r["op"] == "rebuild").unwrap();
    assert_eq!(rebuilt["messages"][1]["role"], "assistant");
    assert_eq!(rebuilt["messages"][1]["reasoning"], "consider the tool");
    assert_eq!(rebuilt["messages"][1]["text"], "checking");
    assert_eq!(rebuilt["messages"][1]["calls"][0]["name"], "echo");
    assert_eq!(rebuilt["messages"][2]["role"], "tool");
    assert_eq!(requests.iter().filter(|r| r["op"] == "append").count(), 1);
}

#[test]
fn length_closes_partial_reasoning_without_automatic_followup() {
    let mut terminal = done("length", 10);
    terminal["reasoning_close"] = json!("length");
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"reasoning_delta","text":"partial thought"}),
            terminal,
        ]),
        vec![
            ("describe", vec![describe()]),
            ("open", vec![json!({"ok":true,"marker":10,"pending":[]})]),
        ],
    ]);
    let mut input = prompt(vec![user("hello")], vec![]);
    input.reasoning = Some(reasoning());
    let (result, events) = run_capture(&fake, input);
    result.unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Output::Completed { end_turn: true, .. }))
    );
    let item = events
        .into_iter()
        .find_map(|e| match e {
            Output::ItemDone(item) => Some(item),
            _ => None,
        })
        .unwrap();
    let mut repeated = prompt(vec![user("hello"), item], vec![]);
    repeated.reasoning = Some(reasoning());
    assert!(
        run_capture(&fake, repeated)
            .0
            .unwrap_err()
            .to_string()
            .contains("repeats a completed transcript")
    );
    assert_eq!(
        fake.finish()
            .iter()
            .filter(|r| r["op"] == "generate")
            .count(),
        1
    );
}

#[test]
fn length_never_publishes_tool_calls_and_next_user_turn_rebuilds_safely() {
    for complete in [false, true] {
        let mut events = vec![
            json!({"event":"start"}),
            json!({"event":"text_delta","text":"partial"}),
            json!({"event":"toolcall_start","id":7,"name":"echo"}),
        ];
        if complete {
            events.push(json!({"event":"toolcall_end","id":7,"name":"echo","arguments":{}}));
        }
        events.push(done("length", 10));
        let mut fake = Fake::new(vec![
            initial(events),
            vec![
                ("describe", vec![describe()]),
                ("open", vec![json!({"ok":true,"marker":10,"pending":[7]})]),
            ],
            vec![
                ("describe", vec![describe()]),
                ("open", vec![json!({"ok":true,"marker":10,"pending":[7]})]),
                ("rebuild", vec![json!({"ok":true})]),
                ("generate", vec![json!({"event":"start"}), done("stop", 12)]),
            ],
        ]);
        let tools = vec![json!({"type":"function","name":"echo","parameters":{}})];
        let (result, events) = run_capture(&fake, prompt(vec![user("hello")], tools.clone()));
        result.unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Output::Completed { end_turn: true, .. }))
        );
        assert!(!events.iter().any(|e| matches!(e, Output::ItemAdded(item) | Output::ItemDone(item) if item["type"] == "function_call")));
        let assistant = events
            .into_iter()
            .find_map(|e| match e {
                Output::ItemDone(item) => Some(item),
                _ => None,
            })
            .unwrap();
        let binding: Value =
            serde_json::from_slice(&fs::read(fake.state().join("thread-1.json")).unwrap()).unwrap();
        assert_eq!(binding["in_flight"], false);
        assert_eq!(binding["rebuild_required"], true);
        assert!(
            run_capture(
                &fake,
                prompt(vec![user("hello"), assistant.clone()], tools.clone())
            )
            .0
            .unwrap_err()
            .to_string()
            .contains("repeats a completed transcript")
        );
        run_capture(
            &fake,
            prompt(vec![user("hello"), assistant, user("continue")], tools),
        )
        .0
        .unwrap();
        let requests = fake.finish();
        assert_eq!(requests.iter().filter(|r| r["op"] == "rebuild").count(), 2);
        let rebuilt = requests.iter().rfind(|r| r["op"] == "rebuild").unwrap();
        assert!(
            !rebuilt["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message.get("calls").is_some())
        );
    }
}

#[test]
fn ordinary_stop_with_unfinished_tool_is_rejected_without_publication() {
    let mut fake = Fake::new(vec![initial(vec![
        json!({"event":"start"}),
        json!({"event":"toolcall_start","id":7,"name":"echo"}),
        done("stop", 10),
    ])]);
    let (result, events) = run_capture(
        &fake,
        prompt(
            vec![user("hello")],
            vec![json!({"type":"function","name":"echo","parameters":{}})],
        ),
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("incomplete native tool turn")
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Output::ItemDone(_) | Output::Completed { .. }))
    );
    fake.finish();
}

#[test]
fn reasoning_and_text_stream_with_one_active_item_and_no_duplicate_completion() {
    let mut fake = Fake::new(vec![initial(vec![
        json!({"event":"start"}),
        json!({"event":"reasoning_delta","text":"a thought"}),
        json!({"event":"text_delta","text":"an answer"}),
        done("stop", 10),
    ])]);
    let mut input = prompt(vec![user("hello")], vec![]);
    input.reasoning = Some(reasoning());
    let (result, events) = run_capture(&fake, input);
    result.unwrap();
    let mut active: Option<String> = None;
    let mut completed = Vec::new();
    for event in events {
        match event {
            Output::ItemAdded(item) => {
                assert!(active.is_none(), "a new item replaced the active item");
                active = Some(item["type"].as_str().unwrap().to_owned());
            }
            Output::ReasoningDelta(_) => assert_eq!(active.as_deref(), Some("reasoning")),
            Output::TextDelta(_) => assert_eq!(active.as_deref(), Some("message")),
            Output::ItemDone(item) => {
                assert_eq!(active.take().as_deref(), item["type"].as_str());
                completed.push(item["type"].as_str().unwrap().to_owned());
            }
            _ => {}
        }
    }
    assert!(active.is_none());
    assert_eq!(completed, ["reasoning", "message"]);
    fake.finish();
}

#[test]
fn failure_after_reasoning_transition_recovers_committed_thought_without_replaying_tools() {
    let mut fake = Fake::new(vec![
        initial(vec![
            json!({"event":"start"}),
            json!({"event":"reasoning_delta","text":"committed thought"}),
            json!({"event":"text_delta","text":"uncommitted answer"}),
            json!({"event":"toolcall_start","id":7,"name":"echo"}),
            json!({"event":"toolcall_end","id":7,"name":"echo","arguments":{}}),
            json!({"event":"error","code":"io_error","error":"failed persistence"}),
        ]),
        vec![
            ("describe", vec![describe()]),
            ("open", vec![json!({"ok":true,"marker":9,"pending":[7]})]),
            ("rebuild", vec![json!({"ok":true})]),
            ("generate", vec![json!({"event":"start"}), done("stop", 12)]),
        ],
    ]);
    let tools = vec![json!({"type":"function","name":"echo","parameters":{}})];
    let mut input = prompt(vec![user("hello")], tools.clone());
    input.reasoning = Some(reasoning());
    let (result, events) = run_capture(&fake, input);
    assert!(result.is_err());
    let committed = events
        .into_iter()
        .filter_map(|event| match event {
            Output::ItemDone(item) => Some(item),
            Output::Completed { .. } => panic!("failed generation completed"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0]["type"], "reasoning");
    let record: Value =
        serde_json::from_slice(&fs::read(fake.state().join("thread-1.json")).unwrap()).unwrap();
    assert_eq!(record["in_flight"], true);
    let mut input = prompt(
        vec![user("hello"), committed[0].clone(), user("continue")],
        tools,
    );
    input.reasoning = Some(reasoning());
    run_capture(&fake, input).0.unwrap();
    let requests = fake.finish();
    let rebuilt = requests.iter().rfind(|r| r["op"] == "rebuild").unwrap();
    assert_eq!(
        rebuilt["messages"][1],
        json!({"role":"assistant","text":"","reasoning":"committed thought"})
    );
    assert_eq!(rebuilt["messages"][2]["text"], "continue");
    assert!(requests.iter().all(|r| r["op"] != "append"));
}

#[test]
fn reasoning_after_visible_text_is_rejected_instead_of_using_message_channel() {
    let mut fake = Fake::new(vec![initial(vec![
        json!({"event":"start"}),
        json!({"event":"text_delta","text":"answer"}),
        json!({"event":"reasoning_delta","text":"late thought"}),
    ])]);
    let mut input = prompt(vec![user("hello")], vec![]);
    input.reasoning = Some(reasoning());
    let (result, events) = run_capture(&fake, input);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("reasoning after visible text")
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        Output::ReasoningDelta(_) | Output::ItemDone(_) | Output::Completed { .. }
    )));
    fake.finish();
}
