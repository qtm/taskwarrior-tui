#!/usr/bin/env bash
# Install build tools for taskwarrior-tui on Ubuntu/Debian x86-64.
# Run as your normal user, not with sudo. No builds or tests are run.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./scripts/setup-linux.sh

Installs system packages via sudo apt-get and stable Rust via rustup:
  - GCC/G++, make, libc development files, Git, curl, CA certificates
  - musl tools for x86_64-unknown-linux-musl builds
  - Pandoc for man-page generation and file for inspecting binaries
  - Rust/Cargo, rustfmt, Clippy, GNU/Linux and musl Rust targets

Run as your normal user on Ubuntu/Debian x86-64. Requires internet access
and sudo privileges. Existing stable Rust installations are updated.
Shell startup files, Taskwarrior data, and project files are not modified.
Taskwarrior itself, documentation-site tooling, and package builders are
not installed: they are not needed to compile the TUI binaries.
EOF
}

fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

if [[ $# -gt 0 ]]; then
  if [[ $# -eq 1 && ( $1 == --help || $1 == -h ) ]]; then
    usage
    exit 0
  fi
  usage >&2
  exit 2
fi

[[ $(uname -s) == Linux ]] || fail 'This script only supports Linux.'
[[ $(uname -m) == x86_64 ]] || fail 'This script targets x86-64 hosts only.'
[[ -r /etc/os-release ]] || fail 'Cannot identify the Linux distribution.'
# shellcheck source=/dev/null
source /etc/os-release
case "${ID:-}" in
  ubuntu|debian) ;;
  *) fail "Unsupported distribution: ${ID:-unknown}. Expected Ubuntu or Debian." ;;
esac

[[ $EUID -ne 0 ]] || fail 'Run without sudo; only system package installation uses sudo.'
command -v sudo >/dev/null 2>&1 || fail 'sudo is required to install system packages.'
command -v apt-get >/dev/null 2>&1 || fail 'apt-get is required.'

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

printf '\nInstalling system packages on %s...\n' "${PRETTY_NAME:-Linux}"
sudo apt-get update
sudo apt-get install -y \
  build-essential git curl ca-certificates \
  musl-tools pandoc file

# Respect custom Cargo/Rustup homes and leave shell startup files alone.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export PATH="$CARGO_HOME/bin:$PATH"

if ! command -v rustup >/dev/null 2>&1; then
  printf '\nInstalling rustup from https://sh.rustup.rs...\n'
  installer=$(mktemp)
  trap 'rm -f -- "$installer"' EXIT
  curl --proto '=https' --tlsv1.2 --fail --show-error --silent --location \
    https://sh.rustup.rs --output "$installer"
  sh "$installer" -y --profile minimal --default-toolchain stable --no-modify-path
  rm -f -- "$installer"
  trap - EXIT
fi

printf '\nInstalling/updating the stable Rust toolchain and Linux targets...\n'
rustup toolchain install stable --profile minimal
rustup component add --toolchain stable rustfmt clippy
rustup target add --toolchain stable \
  x86_64-unknown-linux-gnu \
  x86_64-unknown-linux-musl

printf '\nInstalled toolchain:\n'
rustup run stable rustc --version
rustup run stable cargo --version

printf '\nSetup complete. Run these commands in your shell:\n\n'
printf 'export PATH=%q:"$PATH"\n' "$CARGO_HOME/bin"
printf 'cd %q\n' "$repo_root"
cat <<'EOF'

# Native GNU/Linux binary:
cargo +stable build --release --locked --target x86_64-unknown-linux-gnu

# Portable musl binary:
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
  cargo +stable build --release --locked --target x86_64-unknown-linux-musl

# Outputs:
# target/x86_64-unknown-linux-gnu/release/taskwarrior-tui
# target/x86_64-unknown-linux-musl/release/taskwarrior-tui

Add the export PATH line above to ~/.bashrc to persist it.

Taskwarrior 3.x is required to RUN the TUI, but not to compile it.
This script does not replace your system Taskwarrior or prepare test data.
EOF
