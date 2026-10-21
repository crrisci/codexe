// Xenolith native integration: added or modified by Francesco Pio Crisci, 2026.
// Licensed under Apache-2.0; upstream copyright and notices are retained.

//! Offline session control: close Codex before applying a rewind.
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if !(3..=5).contains(&a.len()) {
        eprintln!(
            "usage: xenolith-session SOCKET STATE_DIR THREAD [MARKER [CODEX_ROLLOUT]]\nWithout a rollout: inspect/cost only. With rollout: apply rewind; close Codex first."
        );
        std::process::exit(2);
    }
    let marker = a
        .get(3)
        .map(|s| s.parse::<u64>())
        .transpose()
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2)
        });
    match codex_xenolith_wire::backend::rewind_control(
        Path::new(&a[0]),
        Path::new(&a[1]),
        &a[2],
        marker,
        a.get(4).map(Path::new),
    ) {
        Ok(report) => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("cannot serialize rewind report: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1)
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("Xenolith native sessions require Unix sockets; use WSL on Windows.");
    std::process::exit(2);
}
