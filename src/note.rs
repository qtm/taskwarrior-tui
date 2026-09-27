//! Markdown notes associated with tasks by a persistent path UDA.
use std::{
  fs::{self, OpenOptions},
  io::ErrorKind,
  path::{Path, PathBuf},
  process::Stdio,
  time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use chrono::Local;
use task_hookrs::{import::import, task::Task, uda::UDAValue};
use tokio::process::Command;
use uuid::Uuid;

pub const UDA: &str = "tuinote";

pub fn raw_value(task: &Task) -> Result<Option<&str>> {
  match task.uda().get(UDA) {
    None => Ok(None),
    Some(UDAValue::Str(value)) if value.is_empty() => Ok(None),
    Some(UDAValue::Str(value)) => Ok(Some(value)),
    _ => bail!("The {UDA} UDA must contain a note path as a string"),
  }
}

async fn run_task(task_exe: &str, args: &[String]) -> Result<Vec<u8>> {
  let output = tokio::time::timeout(
    Duration::from_secs(15),
    Command::new(task_exe)
      .args([
        "rc.confirmation=off",
        "rc.recurrence.confirmation=off",
        "rc.context=",
        "rc.json.array=on",
        "rc.json.depends.array=on",
        "rc.uda.tuinote.type=string",
        "rc.uda.tuinote.label=Note",
      ])
      .args(args)
      .stdin(Stdio::null())
      .kill_on_drop(true)
      .output(),
  )
  .await
  .context("Taskwarrior timed out; refresh before retrying the note")?
  .context("Unable to run Taskwarrior for the note")?;
  ensure!(
    output.status.success(),
    "Taskwarrior failed: {} {}",
    String::from_utf8_lossy(&output.stderr).trim(),
    String::from_utf8_lossy(&output.stdout).trim()
  );
  Ok(output.stdout)
}

async fn load_task(task_exe: &str, uuid: Uuid) -> Result<Task> {
  let bytes = run_task(task_exe, &[uuid.to_string(), "export".into()]).await?;
  let mut tasks: Vec<Task> = import(bytes.as_slice()).context("Unable to read note's task")?;
  ensure!(tasks.len() == 1 && *tasks[0].uuid() == uuid, "Task no longer exists; note was not opened");
  Ok(tasks.remove(0))
}

fn directory(setting: &str) -> Result<PathBuf> {
  ensure!(
    !setting.trim().is_empty(),
    "Set uda.taskwarrior-tui.notes-directory in your taskrc to a folder for task notes"
  );
  let expanded = shellexpand::full(setting).context("Unable to expand notes-directory")?;
  crate::absolute_path(expanded.as_ref()).context("Unable to resolve notes-directory")
}

fn associated_path(value: &str, setting: &str) -> Result<PathBuf> {
  // Saved paths are literal absolute paths. Do not expand '$' in their filenames.
  let path = Path::new(value);
  if path.is_absolute() {
    Ok(path.to_path_buf())
  } else {
    Ok(directory(setting)?.join(path))
  }
}

fn safe_title(title: &str) -> String {
  let title: String = title
    .chars()
    .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c })
    .collect();
  let mut title = title.trim().trim_matches('.').trim().to_string();
  // Leave room under the usual 255-byte filename limit for date, collision suffix and extension.
  let mut end = title.len().min(220);
  while !title.is_char_boundary(end) {
    end -= 1;
  }
  title.truncate(end);
  if title.is_empty() { "Untitled task".into() } else { title }
}

fn create_note(folder: &Path, title: &str, timestamp: &str) -> Result<PathBuf> {
  fs::create_dir_all(folder).with_context(|| format!("Unable to create notes folder {}", folder.display()))?;
  let title = safe_title(title);
  for number in 1..=10_000 {
    let suffix = if number == 1 { String::new() } else { format!(" ({number})") };
    let path = folder.join(format!("{timestamp} {title}{suffix}.md"));
    match OpenOptions::new().write(true).create_new(true).open(&path) {
      Ok(_) => return Ok(path),
      Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
      Err(error) => return Err(error).with_context(|| format!("Unable to create note {}", path.display())),
    }
  }
  bail!("Too many notes with the same timestamp and task title")
}

fn ensure_note(path: &Path) -> Result<()> {
  match fs::metadata(path) {
    Ok(metadata) => ensure!(metadata.is_file(), "Note path is not a file: {}", path.display()),
    Err(error) if error.kind() == ErrorKind::NotFound => {
      if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("Unable to create note folder {}", parent.display()))?;
      }
      match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => (),
        // Another caller may have just created it; never truncate their file.
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
          ensure!(path.is_file(), "Note path is not a file: {}", path.display());
        }
        Err(error) => return Err(error).with_context(|| format!("Unable to create note {}", path.display())),
      }
    }
    Err(error) => return Err(error).with_context(|| format!("Unable to read note {}", path.display())),
  }
  Ok(())
}

/// Re-read by UUID so a stale report/title cannot redirect the association.
pub async fn prepare(task_exe: &str, uuid: Uuid, setting: &str) -> Result<PathBuf> {
  let task = load_task(task_exe, uuid).await?;
  if let Some(value) = raw_value(&task)? {
    let path = associated_path(value, setting)?;
    ensure_note(&path)?;
    return Ok(path);
  }
  let path = create_note(&directory(setting)?, task.description(), &Local::now().format("%y%m%d-%H%M").to_string())?;
  let value = path.to_str().context("Note path is not valid UTF-8")?;
  // Keep the file on backend failure: a hook/timeout may have saved the association.
  run_task(task_exe, &[uuid.to_string(), "modify".into(), format!("{UDA}:{value}")])
    .await
    .with_context(|| format!("Unable to associate note; file retained at {}", path.display()))?;
  Ok(path)
}

/// Match the normal terminal editor preference, while honoring Taskwarrior's editor setting.
pub fn editor_command(configured: &str, visual: Option<&str>, editor: Option<&str>) -> Result<Vec<String>> {
  let command = [Some(configured), visual, editor]
    .into_iter()
    .flatten()
    .find(|value| !value.trim().is_empty())
    .unwrap_or("vi");
  let mut words = shlex::split(command).context("Invalid editor command: unmatched quote")?;
  ensure!(!words.is_empty() && !words[0].is_empty(), "Editor command is empty");
  words[0] = shellexpand::tilde(&words[0]).into_owned();
  Ok(words)
}

/// Inherit the terminal, passing the path as one argument, never through a shell.
pub fn open_editor(command: &[String], path: &Path) -> Result<()> {
  let status = std::process::Command::new(&command[0])
    .args(&command[1..])
    .arg(path)
    .status()
    .with_context(|| format!("Unable to start note editor {}", command[0]))?;
  ensure!(status.success(), "Note editor exited with {status}; note retained at {}", path.display());
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn filename_keeps_spaces_and_unicode_but_not_path_separators() {
    assert_eq!(safe_title("A task Привет 猫"), "A task Привет 猫");
    assert_eq!(safe_title("../bad/\\name\n:*?\"<>|"), "_bad__name________");
    assert_eq!(safe_title(" ... "), "Untitled task");
    let title = safe_title(&"猫".repeat(200));
    assert!(title.len() <= 220);
    assert!(title.chars().all(|c| c == '猫'));
  }

  #[test]
  fn editor_precedence_and_quoted_arguments() {
    assert_eq!(
      editor_command("'my editor' --wait", Some("visual"), Some("editor")).unwrap(),
      ["my editor", "--wait"]
    );
    assert_eq!(editor_command("", Some("visual -f"), Some("editor")).unwrap(), ["visual", "-f"]);
    assert_eq!(editor_command(" ", Some(""), Some("editor")).unwrap(), ["editor"]);
    assert_eq!(editor_command("", None, None).unwrap(), ["vi"]);
    assert!(editor_command("'broken", None, None).is_err());
    assert!(editor_command("''", None, None).is_err());
  }

  #[test]
  fn files_are_unique_and_existing_content_is_preserved() {
    let folder = std::env::temp_dir().join(format!("taskwarrior-note-{}", Uuid::new_v4()));
    let first = create_note(&folder, "Task title", "240104-2021").unwrap();
    assert_eq!(first.file_name().unwrap(), "240104-2021 Task title.md");
    fs::write(&first, "Existing note").unwrap();
    let second = create_note(&folder, "Task title", "240104-2021").unwrap();
    assert_eq!(second.file_name().unwrap(), "240104-2021 Task title (2).md");
    ensure_note(&first).unwrap();
    assert_eq!(fs::read_to_string(&first).unwrap(), "Existing note");
    fs::remove_file(&first).unwrap();
    ensure_note(&first).unwrap();
    assert!(first.is_file());
    assert!(ensure_note(&folder).is_err());
    assert!(create_note(&second, "Task", "240104-2021").is_err());
    fs::remove_dir_all(folder).unwrap();
  }

  #[test]
  fn folder_and_association_paths() {
    assert!(directory("").unwrap_err().to_string().contains("notes-directory"));
    assert!(directory("relative notes").unwrap().is_absolute());
    assert_eq!(directory("~/notes").unwrap(), dirs::home_dir().unwrap().join("notes"));
    assert_eq!(associated_path("/notes/$literal.md", "").unwrap(), Path::new("/notes/$literal.md"));
    assert_eq!(associated_path("note.md", "/notes").unwrap(), Path::new("/notes/note.md"));
  }
}
