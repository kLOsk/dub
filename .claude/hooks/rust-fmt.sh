#!/usr/bin/env bash
#
# Claude Code PostToolUse hook: format Rust files after the agent edits them.
# Mirrors .cursor/hooks/rust-fmt.sh so Cursor and Claude behave identically.
# Failures fail open so they never block agent work; lint cleanliness is
# enforced by CI.

set -uo pipefail

input=$(cat)

# Claude passes {"tool_input":{"file_path":"..."}, ...}; the greedy match grabs
# the edited file's path regardless of nesting.
file_path=$(printf '%s' "$input" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

if [[ -z "${file_path}" ]]; then
  echo '{}'
  exit 0
fi

case "${file_path}" in
  *.rs)
    ;;
  *)
    echo '{}'
    exit 0
    ;;
esac

if ! command -v rustfmt >/dev/null 2>&1; then
  if [[ -x "${HOME}/.cargo/bin/rustfmt" ]]; then
    PATH="${HOME}/.cargo/bin:${PATH}"
  else
    echo '{}'
    exit 0
  fi
fi

rustfmt --edition 2021 "${file_path}" >/dev/null 2>&1 || true

echo '{}'
exit 0
