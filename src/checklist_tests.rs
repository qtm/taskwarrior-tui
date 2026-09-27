//! Backend integration tests: every command uses a disposable TASKRC/TASKDATA.
#![cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

use task_hookrs::task::Task;
use uuid::Uuid;

use crate::checklist::{self, Checklist, Document};

struct Backend {
  directory: PathBuf,
  executable: String,
}

impl Backend {
  fn new() -> Self {
    let directory = std::env::temp_dir().join(format!("taskwarrior-checklist-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let taskrc = directory.join("taskrc");
    let taskdata = directory.join("tasks");
    fs::write(&taskrc, "confirmation=off\nrecurrence.limit=1\nreport.next.filter=status:pending\n").unwrap();
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
    assert!(
      output.status.success(),
      "{args:?}: {} {}",
      String::from_utf8_lossy(&output.stdout),
      String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
  }

  fn tasks(&self) -> Vec<Task> {
    let json = self.command(&[
      "rc.json.array=on",
      "rc.json.depends.array=on",
      "rc.context=",
      "rc.report.all.filter=",
      "export",
      "all",
    ]);
    task_hookrs::import::import(json.as_bytes()).unwrap()
  }
}

impl Drop for Backend {
  fn drop(&mut self) {
    let _ = fs::remove_dir_all(&self.directory);
  }
}

#[tokio::test]
async fn checklist_backend_roundtrip_undo_duplicate_delete_and_conflict() {
  let backend = Backend::new();
  backend.command(&["add", "Checklist parent", "project:work", "+original"]);
  let parent = backend.tasks().remove(0);
  let uuid = *parent.uuid();
  backend.command(&[&uuid.to_string(), "annotate", "Keep this annotation"]);
  let list = Checklist::from_markdown(include_str!("../tests/fixtures/checklist-ru.md")).unwrap();
  let doc = Document {
    version: 1,
    lists: vec![list],
  };
  checklist::save(&backend.executable, uuid, None, &doc).await.unwrap();
  let saved = checklist::load_task(&backend.executable, uuid).await.unwrap();
  assert_eq!(checklist::from_task(&saved).unwrap(), doc);
  assert_eq!(saved.description(), parent.description());
  assert_eq!(saved.tags(), parent.tags());
  assert_eq!(saved.project(), parent.project());
  assert_eq!(saved.annotations().unwrap().len(), 1);
  let raw = checklist::raw_value(&saved).unwrap();
  let mut changed = doc.clone();
  changed.lists[0].items[0].checked = false;
  changed.lists[0].items[0].text = "Quotes \"text\", backslash \\, shell $(never-run) +tag project:other 👩‍💻".into();
  checklist::save(&backend.executable, uuid, raw.as_deref(), &changed).await.unwrap();
  let current = checklist::load_task(&backend.executable, uuid).await.unwrap();
  assert_eq!(checklist::from_task(&current).unwrap(), changed);
  assert!(checklist::from_task(&current).unwrap().lists[0].items[1].checked);
  assert!(
    checklist::save(&backend.executable, uuid, raw.as_deref(), &doc)
      .await
      .unwrap_err()
      .to_string()
      .contains("changed outside")
  );
  assert_eq!(
    checklist::from_task(&checklist::load_task(&backend.executable, uuid).await.unwrap()).unwrap(),
    changed
  );
  backend.command(&["rc.confirmation=off", "undo"]);
  assert_eq!(
    checklist::from_task(&checklist::load_task(&backend.executable, uuid).await.unwrap()).unwrap(),
    doc
  );

  backend.command(&[&uuid.to_string(), "duplicate"]);
  let duplicate = backend.tasks().into_iter().find(|task| *task.uuid() != uuid).unwrap();
  assert_eq!(checklist::from_task(&duplicate).unwrap(), doc);
  let other = Backend::new();
  let exported = other.directory.join("export.json");
  fs::write(&exported, backend.command(&["rc.json.array=on", "export"])).unwrap();
  other.command(&["import", exported.to_str().unwrap()]);
  assert_eq!(
    checklist::from_task(&checklist::load_task(&other.executable, uuid).await.unwrap()).unwrap(),
    doc
  );
  backend.command(&[&uuid.to_string(), "rc.confirmation=off", "delete"]);
  backend.command(&["rc.confirmation=off", "undo"]);
  assert_eq!(
    checklist::from_task(&checklist::load_task(&backend.executable, uuid).await.unwrap()).unwrap(),
    doc
  );
  checklist::save(&backend.executable, uuid, raw.as_deref(), &Document::default())
    .await
    .unwrap();
  assert!(
    checklist::raw_value(&checklist::load_task(&backend.executable, uuid).await.unwrap())
      .unwrap()
      .is_none()
  );
  assert_eq!(
    checklist::from_task(&checklist::load_task(&backend.executable, *duplicate.uuid()).await.unwrap()).unwrap(),
    doc
  );
}

#[tokio::test]
async fn checklist_backend_refuses_corrupt_data_and_failed_commands() {
  let backend = Backend::new();
  backend.command(&["add", "Parent"]);
  let uuid = *backend.tasks()[0].uuid();
  backend.command(&["rc.uda.tuichecklist.type=string", &uuid.to_string(), "modify", "tuichecklist:broken"]);
  assert!(
    checklist::save(&backend.executable, uuid, Some("broken"), &Document::default())
      .await
      .is_err()
  );
  assert_eq!(
    checklist::raw_value(&checklist::load_task(&backend.executable, uuid).await.unwrap())
      .unwrap()
      .as_deref(),
    Some("broken")
  );
  assert!(checklist::load_task(&backend.executable, Uuid::new_v4()).await.is_err());
  assert!(checklist::save("/does/not/exist", uuid, None, &Document::default()).await.is_err());
  let failure = backend.directory.join("failure");
  fs::write(&failure, "#!/bin/sh\necho backend-failure >&2\nexit 1\n").unwrap();
  fs::set_permissions(&failure, fs::Permissions::from_mode(0o700)).unwrap();
  assert!(
    checklist::load_task(failure.to_str().unwrap(), uuid)
      .await
      .unwrap_err()
      .to_string()
      .contains("backend-failure")
  );
}

#[tokio::test]
async fn checklist_recurring_instance_does_not_modify_template() {
  let backend = Backend::new();
  backend.command(&["add", "Recurring checklist", "due:today", "recur:daily"]);
  backend.command(&["next"]); // Generate this occurrence in the disposable database.
  let tasks = backend.tasks();
  let template = tasks
    .iter()
    .find(|task| task_hookrs::status::TaskStatus::Recurring == *task.status())
    .unwrap();
  let instance = tasks.iter().find(|task| task.parent().is_some()).unwrap();
  let doc = Document {
    version: 1,
    lists: vec![Checklist::from_markdown("- [ ] Parent\n  - [ ] Child").unwrap()],
  };
  checklist::save(&backend.executable, *template.uuid(), None, &doc).await.unwrap();
  // Template changes do not unexpectedly overwrite existing occurrences either.
  assert!(
    checklist::raw_value(&checklist::load_task(&backend.executable, *instance.uuid()).await.unwrap())
      .unwrap()
      .is_none()
  );
  checklist::save(&backend.executable, *instance.uuid(), None, &doc).await.unwrap();
  let mut checked = doc.clone();
  checked.lists[0].items[0].checked = true;
  checklist::save(&backend.executable, *instance.uuid(), Some(&doc.encode().unwrap()), &checked)
    .await
    .unwrap();
  assert_eq!(
    checklist::from_task(&checklist::load_task(&backend.executable, *template.uuid()).await.unwrap()).unwrap(),
    doc
  );
  assert_eq!(
    checklist::from_task(&checklist::load_task(&backend.executable, *instance.uuid()).await.unwrap()).unwrap(),
    checked
  );
}
