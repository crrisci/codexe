# CodeXe offline package support, sourced by the upstream-derived install.sh.
# All activation, locking and symlink mechanics remain in install.sh.

codexe_fail() { printf 'CodeXe installation failed: %s\n' "$1" >&2; exit 1; }

codexe_probe() (
  # A basename-aware CLI may initialize its client home even for --version.
  # Keep package checks independent of the user's settings and conversations.
  HOME="$tmp_dir/probe-home"
  CODEX_HOME="$HOME/.codexe"
  export HOME CODEX_HOME
  mkdir -p "$CODEX_HOME"
  "$@"
)

codexe_compact_json() {
  # Preserve whitespace inside strings; canonical field values need no escapes.
  awk '
    { for (i=1; i<=length($0); i++) {
        c=substr($0,i,1)
        if (quoted) {
          printf "%s",c
          if (escaped) escaped=0
          else if (c=="\\") escaped=1
          else if (c=="\"") quoted=0
        } else if (c=="\"") { quoted=1; printf "%s",c }
        else if (c !~ /[ \t\r]/) printf "%s",c
      }
      if (quoted) exit 1
    }
    END { if (quoted || escaped) exit 1 }
  ' "$1"
}

codexe_metadata() {
  [ -f "$1/codex-package.json" ] || codexe_fail 'Missing codex-package.json.'
  [ "$(wc -c < "$1/codex-package.json")" -le 65536 ] || codexe_fail 'Package metadata exceeds 64 KiB.'
  native_json="$(codexe_compact_json "$1/codex-package.json")" || codexe_fail 'Invalid package metadata.'
  native_version="$(printf '%s' "$native_json" | sed -n 's/^{"layoutVersion":1,"version":"\([^"]*\)","target":.*/\1/p')"
  native_target="$(printf '%s' "$native_json" | sed -n 's/^.*,"target":"\([^"]*\)","variant":.*/\1/p')"
  printf '%s\n' "$native_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.+-][0-9A-Za-z.+-]+)?$' || codexe_fail 'Invalid package version.'
  case "$native_target" in
    "$arch-unknown-linux-gnu" | "$arch-unknown-linux-musl") ;;
    *) codexe_fail "Package target does not match this Linux device: $native_target" ;;
  esac
  native_expected="{\"layoutVersion\":1,\"version\":\"$native_version\",\"target\":\"$native_target\",\"variant\":\"codex\",\"entrypoint\":\"bin/codex\",\"execServer\":\"bin/codex-exec-server\",\"resourcesDir\":\"codex-resources\",\"pathDir\":\"codex-path\",\"xenolith\":{\"nativeConfig\":1}}"
  [ "$native_json" = "$native_expected" ] || codexe_fail 'Expected canonical CodeXe metadata with xenolith.nativeConfig=1; rebuild this package.'
}

codexe_validate_tree() {
  native_tree="$1"
  native_expected_hash="${2:-}"
  [ -d "$native_tree" ] && [ ! -L "$native_tree" ] || codexe_fail 'Package must be a regular directory.'
  # Only the compatibility link made by the upstream activation code is allowed.
  (cd "$native_tree" && find . ! -type f ! -type d -print) > "$tmp_dir/special"
  while IFS= read -r native_entry; do
    [ "$native_entry" = './codex' ] && [ "$(readlink "$native_tree/codex")" = 'bin/codex' ] || codexe_fail 'Package contains a symlink or special file.'
  done < "$tmp_dir/special"
  (cd "$native_tree" && find . -print) > "$tmp_dir/entries"
  if LC_ALL=C grep -Ev '^[A-Za-z0-9_./+-]+$' "$tmp_dir/entries" >/dev/null; then codexe_fail 'Package entry names must use canonical portable characters.'; fi
  native_manifest="$native_tree/codexe-package_SHA256SUMS"
  [ -f "$native_manifest" ] || codexe_fail 'Missing codexe-package_SHA256SUMS; use the native package builder.'
  LC_ALL=C awk 'NF!=2 || length($1)!=64 || $1~/[^0-9a-f]/ || $0!~/^[0-9a-f]+  / || $2!~/^[A-Za-z0-9_./+-]+$/ || $2~/^\// || $2~/(^|\/)\.\.?($|\/)/ || seen[$2]++ { bad=1 } END { exit (bad || NR==0) }' "$native_manifest" || codexe_fail 'Invalid package checksum manifest.'
  LC_ALL=C awk '{print $2}' "$native_manifest" | LC_ALL=C sort > "$tmp_dir/manifest-files"
  (cd "$native_tree" && find . -type f -print) | sed 's|^./||' | grep -v '^codexe-package_SHA256SUMS$' | LC_ALL=C sort > "$tmp_dir/tree-files"
  cmp -s "$tmp_dir/manifest-files" "$tmp_dir/tree-files" || codexe_fail 'Package files differ from its checksum manifest.'
  native_actual_hash="$(file_sha256 "$native_manifest")"
  [ -z "$native_expected_hash" ] || [ "$native_expected_hash" = "$native_actual_hash" ] || codexe_fail 'Installed package was modified; refusing to overwrite it.'
  while read -r native_digest native_entry; do
    [ "$(file_sha256 "$native_tree/$native_entry")" = "$native_digest" ] || codexe_fail "Checksum mismatch: $native_entry"
  done < "$native_manifest"
  codexe_metadata "$native_tree"
  for native_entry in LICENSE NOTICE third-party-notices/bubblewrap-COPYING third-party-notices/ripgrep-LICENSE-MIT third-party-notices/v8-LICENSE third-party-notices/rusty-v8-LICENSE install/install.sh install/codexe-local.sh; do
    [ -f "$native_tree/$native_entry" ] || codexe_fail "Missing package file: $native_entry"
  done
  for native_entry in bin/codex bin/codex-code-mode-host codex-resources/bwrap bin/codex-exec-server codex-path/rg; do
    [ -x "$native_tree/$native_entry" ] || codexe_fail "Package helper is not executable: $native_entry"
    native_flag=--version
    [ "$native_entry" != bin/codex-code-mode-host ] || native_flag=--help
    if ! codexe_probe timeout 15 "$native_tree/$native_entry" "$native_flag" > "$tmp_dir/probe.log" 2>&1; then
      cat "$tmp_dir/probe.log" >&2
      codexe_fail "Packaged $native_entry cannot start; previous command remains selected."
    fi
  done
}

codexe_unpack() {
  tar -tzf "$CODEXE_ARCHIVE" > "$tmp_dir/archive-entries" || codexe_fail 'Cannot list package archive.'
  LC_ALL=C awk '$0!~/^[A-Za-z0-9_./+-]+$/ || $0~/^\// || $0~/(^|\/)\.\.($|\/)/ || seen[$0]++ { bad=1 } END { exit (bad || NR==0) }' "$tmp_dir/archive-entries" || codexe_fail 'Unsafe or duplicate package archive path.'
  tar -tvzf "$CODEXE_ARCHIVE" > "$tmp_dir/archive-types" || codexe_fail 'Cannot inspect package archive.'
  LC_ALL=C awk 'substr($0,1,1)!="-" && substr($0,1,1)!="d" { bad=1 } END { exit bad }' "$tmp_dir/archive-types" || codexe_fail 'Package archives must not contain links or special files.'
  CODEXE_LOCAL_DIR="$tmp_dir/package"
  mkdir "$CODEXE_LOCAL_DIR"
  tar -xzf "$CODEXE_ARCHIVE" -C "$CODEXE_LOCAL_DIR" || codexe_fail 'Cannot extract package archive.'
}

codexe_preflight() {
  if [ -e "$STANDALONE_ROOT" ] || [ -L "$STANDALONE_ROOT" ]; then
    [ -d "$STANDALONE_ROOT" ] && [ ! -L "$STANDALONE_ROOT" ] && [ ! -L "$STANDALONE_ROOT/.codexe-local" ] && [ "$(cat "$STANDALONE_ROOT/.codexe-local" 2>/dev/null || true)" = codexe-local-v1 ] || codexe_fail "Unrecognized install root: $STANDALONE_ROOT"
  fi
  for native_path in "$RELEASES_DIR" "$LOCK_FILE" "$LOCK_DIR"; do
    [ ! -L "$native_path" ] || codexe_fail "Refusing symlinked install path: $native_path"
  done
  [ ! -e "$RELEASES_DIR" ] || [ -d "$RELEASES_DIR" ] || codexe_fail 'Release storage is not a directory.'
  [ ! -e "$LOCK_FILE" ] || [ -f "$LOCK_FILE" ] || codexe_fail 'Install lock is not a regular file.'
  [ ! -e "$LOCK_DIR" ] || [ -d "$LOCK_DIR" ] || codexe_fail 'Fallback lock is not a directory.'
  if [ -e "$CURRENT_LINK" ] || [ -L "$CURRENT_LINK" ]; then
    [ -L "$CURRENT_LINK" ] || codexe_fail 'Current selection is not an installer-owned link.'
    native_selection="$(readlink "$CURRENT_LINK")"
    case "$native_selection" in "$RELEASES_DIR/"*) ;; *) codexe_fail 'Current selection points outside this installation.';; esac
    native_selection="${native_selection#"$RELEASES_DIR/"}"
    case "$native_selection" in ''|*/*|.|..) codexe_fail 'Invalid current release link.';; esac
  fi
  if [ -e "$BIN_PATH" ] || [ -L "$BIN_PATH" ]; then
    [ -f "$STANDALONE_ROOT/.codexe-local" ] && [ -L "$BIN_PATH" ] && [ "$(readlink "$BIN_PATH")" = "$CURRENT_LINK/bin/codex" ] || codexe_fail "Refusing to replace unrelated command $BIN_PATH; choose another --bin-dir or --prefix."
  fi
}

codexe_local_install() {
  [ "$(uname -s)" = Linux ] || codexe_fail 'Local CodeXe installation supports Linux/WSL only.'
  os=linux
  case "$(uname -m)" in x86_64|amd64) arch=x86_64;; aarch64|arm64) arch=aarch64;; *) codexe_fail 'Unsupported architecture.';; esac
  [ "$DAEMON_ONLY" != 1 ] && [ "${CODEX_INSTALL_DEFER_SELECTION:-0}" != 1 ] || codexe_fail 'CodeXe local mode does not install or update an upstream daemon.'
  [ -n "$CODEXE_LOCAL_DIR" ] || [ -n "$CODEXE_ARCHIVE" ] || codexe_fail 'Supply --package-dir or --package-archive; CodeXe local mode never downloads upstream Codex.'
  [ -z "$CODEXE_LOCAL_DIR" ] || [ -z "$CODEXE_ARCHIVE" ] || codexe_fail 'Choose one package input.'
  case "$CODEXE_PREFIX" in /*) ;; *) codexe_fail '--prefix must be absolute.';; esac
  BIN_DIR="${CODEXE_BIN_DIR:-$CODEXE_PREFIX/bin}"
  case "$BIN_DIR" in /*) ;; *) codexe_fail '--bin-dir must be absolute.';; esac
  COMMAND_NAME=codexe
  BIN_PATH="$BIN_DIR/codexe"
  STANDALONE_ROOT="$CODEXE_PREFIX/lib/codexe/standalone"
  RELEASES_DIR="$STANDALONE_ROOT/releases"
  CURRENT_LINK="$STANDALONE_ROOT/current"
  LOCK_FILE="$STANDALONE_ROOT/install.lock"
  LOCK_DIR="$STANDALONE_ROOT/install.lock.d"
  CODE_MODE_HOST_BIN_PATH="$BIN_DIR/codex-code-mode-host"
  for native_command in mktemp tar awk sed grep cmp sort find timeout; do require_command "$native_command"; done
  codexe_preflight
  tmp_dir="$(mktemp -d)"
  trap 'release_install_lock; rm -rf "$tmp_dir"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  if [ -n "$CODEXE_ARCHIVE" ]; then codexe_unpack; fi
  CODEXE_LOCAL_DIR="$(cd "$CODEXE_LOCAL_DIR" && pwd -P)" || codexe_fail 'Package directory does not exist.'
  codexe_validate_tree "$CODEXE_LOCAL_DIR"
  resolved_version="$native_version"
  vendor_target="$native_target"
  native_source_hash="$(file_sha256 "$CODEXE_LOCAL_DIR/codexe-package_SHA256SUMS")"
  CODEXE_RELEASE_NAME="$resolved_version-$vendor_target-$native_source_hash"
  release_dir="$RELEASES_DIR/$CODEXE_RELEASE_NAME"
  install_layout=package
  acquire_install_lock
  if [ ! -f "$STANDALONE_ROOT/.codexe-local" ]; then printf '%s\n' codexe-local-v1 > "$STANDALONE_ROOT/.codexe-local"; fi
  codexe_preflight
  cleanup_stale_install_artifacts
  if [ ! -e "$release_dir" ] && [ ! -L "$release_dir" ]; then
    step "Installing local CodeXe package to $release_dir"
    install_package_release "$release_dir" ''
  fi
  release_dir_is_complete "$release_dir" "$resolved_version" "$vendor_target" package || codexe_fail 'Installed package is incomplete; previous command remains selected.'
  codexe_preflight
  if [ "$(readlink "$CURRENT_LINK" 2>/dev/null || true)" != "$release_dir" ]; then update_current_link "$release_dir"; fi
  update_visible_command "$release_dir"
  codexe_probe verify_visible_command
  release_install_lock
  printf 'CodeXe %s installed. Start: %s\n' "$resolved_version" "$BIN_PATH"
  # Do not modify profiles or start a client during package activation.
  printf 'For this terminal, if needed: export PATH='
  printf "'%s'" "$(printf '%s' "$BIN_DIR" | sed "s/'/'\\\\''/g")"
  printf ':"$PATH"\n'
}
