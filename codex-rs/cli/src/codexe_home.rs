// Copyright (c) 2026 CodeXe contributors. Licensed under Apache-2.0.

use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

fn alias_home(arg0: &OsStr, explicit_home: Option<&OsStr>, default_home: &Path) -> Option<PathBuf> {
    if explicit_home.is_some_and(|value| !value.is_empty())
        || Path::new(arg0).file_name() != Some(OsStr::new("codexe"))
    {
        return None;
    }
    default_home.parent().map(|parent| parent.join(".codexe"))
}

/// Called only from synchronous main, before the runtime or worker threads.
/// Selecting the home in the environment also preserves it when this binary
/// re-executes under a helper name or starts the embedded server's children.
pub(super) fn initialize() -> std::io::Result<()> {
    let Some(arg0) = std::env::args_os().next() else {
        return Ok(());
    };
    let explicit_home = std::env::var_os("CODEX_HOME");
    if explicit_home
        .as_ref()
        .is_some_and(|value| !value.is_empty())
        || Path::new(&arg0).file_name() != Some(OsStr::new("codexe"))
    {
        return Ok(());
    }
    let default_home = codex_core::config::find_codex_home()?;
    let Some(home) = alias_home(&arg0, explicit_home.as_deref(), default_home.as_path()) else {
        return Ok(());
    };
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&home)?;
    // SAFETY: the CLI calls this at the start of main, before creating threads.
    unsafe {
        std::env::set_var("CODEX_HOME", home);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codexe_alias_has_a_separate_default_home() {
        assert_eq!(
            alias_home(
                OsStr::new("/opt/bin/codexe"),
                None,
                Path::new("/home/user/.codex")
            ),
            Some(PathBuf::from("/home/user/.codexe")),
        );
    }

    #[test]
    fn codex_and_helper_names_keep_the_existing_default() {
        for name in [
            "codex",
            "/opt/bin/codex",
            "codex-linux-sandbox",
            "apply_patch",
        ] {
            assert!(alias_home(OsStr::new(name), None, Path::new("/home/user/.codex")).is_none());
        }
    }

    #[test]
    fn explicit_home_always_wins_including_inherited_child_selection() {
        for name in ["codex", "codexe", "apply_patch"] {
            assert!(
                alias_home(
                    OsStr::new(name),
                    Some(OsStr::new("/chosen/home")),
                    Path::new("/home/user/.codex")
                )
                .is_none()
            );
        }
    }
}
