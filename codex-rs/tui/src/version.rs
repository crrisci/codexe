/// The current Codex CLI version as embedded at compile time.
#[cfg(not(test))]
pub const CODEX_CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

// Unit snapshots use a fixed version so release bumps cannot change wrapping or padding.
// Production and integration-test binaries retain the actual package version above.
#[cfg(test)]
pub const CODEX_CLI_VERSION: &str = "0.0.0";
