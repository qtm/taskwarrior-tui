//! Read-only clipboard access. No shell, clipboard logging, or implicit task writes.
use std::{process::Stdio, time::Duration};

use anyhow::{Context, Result, anyhow, ensure};
use tokio::{io::AsyncReadExt, process::Command};

use crate::checklist::MAX_BYTES;

async fn read_command(executable: &str, args: &[&str], timeout: Duration) -> Result<String> {
  let read = async {
    let mut child = Command::new(executable)
      .args(args)
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .stderr(Stdio::null())
      .kill_on_drop(true)
      .spawn()
      .with_context(|| format!("Unable to start {executable}"))?;
    let mut bytes = Vec::new();
    child
      .stdout
      .take()
      .context("Clipboard reader has no output")?
      .take(MAX_BYTES as u64 + 1)
      .read_to_end(&mut bytes)
      .await?;
    ensure!(bytes.len() <= MAX_BYTES, "Clipboard exceeds {MAX_BYTES} bytes");
    ensure!(child.wait().await?.success(), "Clipboard reader failed");
    let text = String::from_utf8(bytes).context("Clipboard is not UTF-8 text")?;
    ensure!(!text.trim().is_empty(), "Clipboard is empty");
    Ok(text)
  };
  tokio::time::timeout(timeout, read).await.context("Clipboard reader timed out")?
}

pub async fn read() -> Result<String> {
  let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
    &[("pbpaste", &[])]
  } else {
    &[
      ("wl-paste", &["--no-newline"]),
      ("xclip", &["-selection", "clipboard", "-out"]),
      ("xsel", &["--clipboard", "--output"]),
    ]
  };
  let mut errors = Vec::new();
  for &(executable, args) in candidates {
    match read_command(executable, args, Duration::from_secs(2)).await {
      Ok(text) => return Ok(text),
      Err(error) => errors.push(format!("{executable}: {error:#}")),
    }
  }
  Err(anyhow!(
    "{}\nPaste Markdown here using your terminal's paste shortcut, then Enter to import.",
    errors.join("; ")
  ))
}

#[cfg(all(test, unix))]
mod tests {
  use super::*;

  #[tokio::test]
  async fn reader_preserves_newlines_and_detects_failures() {
    let timeout = Duration::from_secs(2);
    let text = read_command("/bin/sh", &["-c", "printf '%s\\n' '- [x] Привет' '  - [ ] 子'"], timeout)
      .await
      .unwrap();
    assert_eq!(text, "- [x] Привет\n  - [ ] 子\n");
    assert!(read_command("/bin/sh", &["-c", "exit 1"], timeout).await.is_err());
    assert!(read_command("/bin/sh", &["-c", "printf '%70000s' x"], timeout).await.is_err());
    assert!(read_command("/bin/sh", &["-c", "printf '\\377'"], timeout).await.is_err());
    assert!(read_command("/bin/sh", &["-c", "exec sleep 2"], Duration::from_millis(20)).await.is_err());
    assert!(read_command("/does/not/exist", &[], timeout).await.is_err());
  }
}
