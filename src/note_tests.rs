//! Note backend tests use only disposable Taskwarrior data and configuration.
#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

use task_hookrs::task::Task;
use uuid::Uuid;

use crate::note;

struct Backend {
  directory: PathBuf,
  executable: String,
}

impl Backend {
  fn new() -> Self {
    let directory = std::env::temp_dir().join(format!("taskwarrior-note-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let taskrc = directory.join("taskrc");
    let taskdata = directory.join("tasks");
    fs::write(&taskrc, "confirmation=off\n").unwrap();
    let task = std::env::var("TASKWARRIOR_TUI_TASKWARRIOR_CLI").unwrap_or_else(|_| "task".into());
    let wrapper = directory.join("task");
    fs::write(
      &wrapper,
      format!(
        "#!/bin/sh\nexport TASKRC={} TASKDATA={}\nexec {} \"$@\"\n",
        shlex::try_quote(taskrc.to_str().unwrap()).unwrap(),
        shlex::try_quote(taskdata.to_str().unwrap()).unwrap(),
        shlex::try_quote(&task).unwrap()
      ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    Self {
      directory,
      executable: wrapper.to_string_lossy().into_owned(),
    }
  }

  fn command(&self, args: &[&str]) -> String {
    let output = Command::new(&self.executable).args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
  }

  fn tasks(&self) -> Vec<Task> {
    let data = self.command(&["rc.json.array=on", "rc.json.depends.array=on", "rc.context=", "export"]);
    task_hookrs::import::import(data.as_bytes()).unwrap()
  }
}

impl Drop for Backend {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.directory);
  }
}

#[tokio::test]
async fn note_association_roundtrip_reopen_rename_and_missing_file() {
  let backend = Backend::new();
  backend.command(&["add", "Task title Привет / 猫 $(not-run)", "project:work", "+original"]);
  let original = backend.tasks().remove(0);
  let uuid = *original.uuid();
  backend.command(&[&uuid.to_string(), "annotate", "Keep annotation"]);
  // Spaces, quotes and '$' in a configured folder must survive the Taskwarrior CLI verbatim.
  let folder = backend.directory.join("My 'notes' \"quoted\" $literal");
  let setting = folder.to_str().unwrap().replace('$', "$$");
  let path = note::prepare(&backend.executable, uuid, &setting).await.unwrap();
  assert_eq!(path.parent().unwrap(), folder);
  let name = path.file_name().unwrap().to_str().unwrap();
  assert!(
    regex::Regex::new(r"^\d{6}-\d{4} Task title Привет _ 猫 \$\(not-run\)\.md$")
      .unwrap()
      .is_match(name),
    "{name}"
  );
  assert!(path.is_file());
  let saved = backend.tasks().remove(0);
  assert_eq!(note::raw_value(&saved).unwrap(), path.to_str());
  assert_eq!(saved.description(), original.description());
  assert_eq!(saved.project(), original.project());
  assert_eq!(saved.tags(), original.tags());
  assert_eq!(saved.annotations().unwrap().len(), 1);
  fs::write(&path, "# My note\nKeep this content\n").unwrap();

  backend.command(&[&uuid.to_string(), "modify", "description:Renamed task"]);
  // An existing absolute association needs no folder setting and survives a title/config change.
  assert_eq!(note::prepare(&backend.executable, uuid, "").await.unwrap(), path);
  assert_eq!(fs::read_to_string(&path).unwrap(), "# My note\nKeep this content\n");
  assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
  fs::remove_file(&path).unwrap();
  assert_eq!(note::prepare(&backend.executable, uuid, "/unused").await.unwrap(), path);
  assert!(path.is_file());
}

#[tokio::test]
async fn note_failures_do_not_change_task_or_overwrite_existing_files() {
  let backend = Backend::new();
  backend.command(&["add", "Task"]);
  let uuid = *backend.tasks()[0].uuid();
  assert!(
    note::prepare(&backend.executable, uuid, "")
      .await
      .unwrap_err()
      .to_string()
      .contains("notes-directory")
  );
  let folder = backend.directory.join("notes");
  assert!(
    note::prepare(&backend.executable, Uuid::new_v4(), folder.to_str().unwrap())
      .await
      .is_err()
  );
  assert!(!folder.exists());
  assert!(note::raw_value(&backend.tasks()[0]).unwrap().is_none());
  backend.command(&[
    "rc.uda.tuinote.type=string",
    &uuid.to_string(),
    "modify",
    &format!("tuinote:{}", backend.directory.display()),
  ]);
  assert!(
    note::prepare(&backend.executable, uuid, "")
      .await
      .unwrap_err()
      .to_string()
      .contains("not a file")
  );
  assert!(note::prepare("/nonexistent-task-for-notes", uuid, "").await.is_err());

  let failure = backend.directory.join("failure");
  fs::write(&failure, "#!/bin/sh\necho note-backend-failure >&2\nexit 1\n").unwrap();
  fs::set_permissions(&failure, fs::Permissions::from_mode(0o700)).unwrap();
  assert!(
    note::prepare(failure.to_str().unwrap(), uuid, "")
      .await
      .unwrap_err()
      .to_string()
      .contains("note-backend-failure")
  );
}

#[tokio::test]
async fn note_failed_association_retains_file_and_reports_its_path() {
  let backend = Backend::new();
  backend.command(&["add", "Task"]);
  let uuid = *backend.tasks()[0].uuid();
  let wrapper = backend.directory.join("reject-modify");
  fs::write(
    &wrapper,
    format!(
      "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = modify ]; then echo rejected-association >&2; exit 1; fi\ndone\nexec {} \"$@\"\n",
      shlex::try_quote(&backend.executable).unwrap()
    ),
  )
  .unwrap();
  fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
  let folder = backend.directory.join("notes");
  let error = note::prepare(wrapper.to_str().unwrap(), uuid, folder.to_str().unwrap())
    .await
    .unwrap_err();
  let files: Vec<_> = fs::read_dir(&folder).unwrap().collect();
  assert_eq!(files.len(), 1);
  let path = files[0].as_ref().unwrap().path();
  assert!(path.is_file());
  assert!(format!("{error:#}").contains(path.to_str().unwrap()));
  assert!(format!("{error:#}").contains("rejected-association"));
  assert!(note::raw_value(&backend.tasks()[0]).unwrap().is_none());
}

#[test]
fn note_editor_receives_one_literal_path_argument_and_reports_failures() {
  let backend = Backend::new();
  let script = backend.directory.join("fake editor");
  let output = backend.directory.join("args");
  fs::write(
    &script,
    "#!/bin/sh\noutput=$1\nshift\nprintf '%s\\n' \"$#\" \"$1\" \"$2\" > \"$output\"\n",
  )
  .unwrap();
  fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
  let path = backend.directory.join("240104-2021 Title $(not-run) 'quote'.md");
  let command = vec![script.to_str().unwrap().into(), output.to_str().unwrap().into(), "--wait".into()];
  note::open_editor(&command, &path).unwrap();
  assert_eq!(fs::read_to_string(&output).unwrap(), format!("2\n--wait\n{}\n", path.display()));
  assert!(note::open_editor(&["/nonexistent-note-editor".into()], &path).is_err());
  assert!(
    note::open_editor(&["/bin/false".into()], &path)
      .unwrap_err()
      .to_string()
      .contains("note retained")
  );
}
