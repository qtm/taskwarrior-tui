//! Task-attached checklists. Depth is a validated preorder tree, not visual-only indentation.
use std::{collections::HashSet, ops::Range, process::Stdio, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use task_hookrs::{import::import, task::Task, uda::UDAValue};
use tokio::process::Command;
use uuid::Uuid;

pub const UDA: &str = "tuichecklist";
pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_ITEMS: usize = 2000;
pub const MAX_DEPTH: u8 = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
  pub id: Uuid,
  pub text: String,
  pub checked: bool,
  pub depth: u8,
}

impl Item {
  pub fn new(text: String, depth: u8) -> Self {
    Self {
      id: Uuid::new_v4(),
      text,
      checked: false,
      depth,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checklist {
  pub id: Uuid,
  pub title: String,
  pub items: Vec<Item>,
}

impl Checklist {
  pub fn new(title: String) -> Self {
    Self {
      id: Uuid::new_v4(),
      title,
      items: Vec::new(),
    }
  }

  pub fn progress(&self) -> (usize, usize) {
    (self.items.iter().filter(|item| item.checked).count(), self.items.len())
  }

  pub fn subtree(&self, index: usize) -> Range<usize> {
    let depth = self.items[index].depth;
    let end = (index + 1..self.items.len())
      .find(|&i| self.items[i].depth <= depth)
      .unwrap_or(self.items.len());
    index..end
  }

  pub fn insert(&mut self, selected: Option<usize>, child: bool, text: String) -> Result<Uuid> {
    let (at, depth) = match selected {
      Some(index) => (self.subtree(index).end, self.items[index].depth + u8::from(child)),
      None => (self.items.len(), 0),
    };
    ensure!(depth <= MAX_DEPTH, "Maximum nesting depth is {MAX_DEPTH}");
    valid_text(&text)?;
    let item = Item::new(text, depth);
    let id = item.id;
    self.items.insert(at, item);
    Ok(id)
  }

  pub fn indent(&mut self, index: usize) -> Result<()> {
    let range = self.subtree(index);
    let depth = self.items[index].depth;
    ensure!(index > 0 && self.items[index - 1].depth >= depth, "No preceding sibling to nest under");
    ensure!(
      self.items[range.clone()].iter().all(|item| item.depth < MAX_DEPTH),
      "Maximum nesting depth is {MAX_DEPTH}"
    );
    for item in &mut self.items[range] {
      item.depth += 1;
    }
    Ok(())
  }

  pub fn outdent(&mut self, index: usize) -> Result<()> {
    let depth = self.items[index].depth;
    ensure!(depth > 0, "Item is already at the top level");
    let parent = (0..index).rev().find(|&i| self.items[i].depth < depth).context("Missing parent")?;
    let range = self.subtree(index);
    // Place after the entire parent subtree, not before the selected item's old siblings.
    let destination = self.subtree(parent).end - range.len();
    let mut moved: Vec<_> = self.items.drain(range).collect();
    for item in &mut moved {
      item.depth -= 1;
    }
    self.items.splice(destination..destination, moved);
    Ok(())
  }

  pub fn move_item(&mut self, index: usize, down: bool) -> Result<()> {
    let range = self.subtree(index);
    let depth = self.items[index].depth;
    if down {
      ensure!(range.end < self.items.len() && self.items[range.end].depth == depth, "No next sibling");
      let end = self.subtree(range.end).end;
      self.items[index..end].rotate_left(range.len());
    } else {
      let previous = (0..index).rev().find(|&i| self.items[i].depth <= depth).context("No previous sibling")?;
      ensure!(self.items[previous].depth == depth, "No previous sibling");
      self.items[previous..range.end].rotate_right(range.len());
    }
    Ok(())
  }

  pub fn markdown(&self) -> String {
    self
      .items
      .iter()
      .map(|item| {
        format!(
          "{}- [{}] {}",
          "  ".repeat(item.depth as usize),
          if item.checked { 'x' } else { ' ' },
          item.text
        )
      })
      .collect::<Vec<_>>()
      .join("\n")
  }

  /// Deliberately strict: never silently discard prose or malformed list items.
  pub fn from_markdown(markdown: &str) -> Result<Self> {
    ensure!(markdown.len() <= MAX_BYTES, "Markdown exceeds {MAX_BYTES} bytes");
    let mut list = Self::new("Checklist".into());
    let mut indents = Vec::new();
    let mut heading_seen = false;
    for (line_number, original) in markdown.trim_start_matches('\u{feff}').lines().enumerate() {
      let line = original.trim_end_matches('\r');
      if line.trim().is_empty() {
        continue;
      }
      let body = line.trim_start_matches(' ');
      if list.items.is_empty() && !heading_seen && body.starts_with('#') {
        let title = body.trim_start_matches('#');
        ensure!(title.starts_with(' '), "Line {}: expected a heading or checkbox", line_number + 1);
        valid_text(title.trim()).with_context(|| format!("Line {}", line_number + 1))?;
        list.title = title.trim().into();
        heading_seen = true;
        continue;
      }
      let parse_line = || -> Result<(usize, bool, String)> {
        ensure!(!body.starts_with('\t'), "use spaces, not tabs, for indentation");
        let bytes = body.as_bytes();
        ensure!(
          bytes.len() >= 6 && matches!(bytes[0], b'-' | b'*' | b'+') && bytes[1..3] == *b" [" && bytes[4..6] == *b"] ",
          "expected - [ ] text or - [x] text"
        );
        ensure!(matches!(bytes[3], b' ' | b'x' | b'X'), "invalid checkbox state");
        let text = body[6..].trim().to_string();
        valid_text(&text)?;
        Ok((line.len() - body.len(), bytes[3] != b' ', text))
      };
      let (indent, checked, text) = parse_line().with_context(|| format!("Line {}", line_number + 1))?;
      match indents.last().copied() {
        None => indents.push(indent),
        Some(previous) if indent > previous => indents.push(indent),
        Some(previous) if indent < previous => {
          let level = indents
            .iter()
            .position(|&known| known == indent)
            .with_context(|| format!("Line {}: inconsistent indentation", line_number + 1))?;
          indents.truncate(level + 1);
        }
        _ => {}
      }
      ensure!(
        indents.len() <= MAX_DEPTH as usize + 1,
        "Line {}: maximum nesting depth is {MAX_DEPTH}",
        line_number + 1
      );
      ensure!(list.items.len() < MAX_ITEMS, "Too many checklist items (maximum {MAX_ITEMS})");
      list.items.push(Item {
        id: Uuid::new_v4(),
        text,
        checked,
        depth: (indents.len() - 1) as u8,
      });
    }
    ensure!(
      !list.items.is_empty(),
      "No Markdown checklist items found. Paste lines such as - [ ] Item"
    );
    let document = Document {
      version: 1,
      lists: vec![list.clone()],
    };
    document.encode()?;
    Ok(list)
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
  pub version: u32,
  pub lists: Vec<Checklist>,
}

impl Default for Document {
  fn default() -> Self {
    Self {
      version: 1,
      lists: Vec::new(),
    }
  }
}

impl Document {
  pub fn decode(raw: Option<&str>) -> Result<Self> {
    match raw {
      None | Some("") => Ok(Self::default()),
      Some(raw) => {
        ensure!(raw.len() <= MAX_BYTES, "Checklist exceeds {MAX_BYTES} bytes; data was not changed");
        let document: Self = serde_json::from_str(raw).context("Invalid checklist JSON; data was not changed")?;
        document.validate()?;
        Ok(document)
      }
    }
  }

  pub fn encode(&self) -> Result<String> {
    self.validate()?;
    let raw = serde_json::to_string(self)?;
    ensure!(raw.len() <= MAX_BYTES, "Checklist exceeds {MAX_BYTES} bytes");
    Ok(raw)
  }

  pub fn validate(&self) -> Result<()> {
    ensure!(self.version == 1, "Unsupported checklist version {}; data was not changed", self.version);
    ensure!(self.lists.len() <= MAX_ITEMS, "Too many checklists");
    let mut ids = HashSet::new();
    let mut count = 0;
    for list in &self.lists {
      valid_text(&list.title)?;
      ensure!(ids.insert(list.id), "Duplicate checklist ID");
      let mut previous_depth = 0;
      for (i, item) in list.items.iter().enumerate() {
        count += 1;
        ensure!(count <= MAX_ITEMS, "Too many checklist items (maximum {MAX_ITEMS})");
        valid_text(&item.text)?;
        ensure!(ids.insert(item.id), "Duplicate item ID");
        ensure!(
          item.depth <= MAX_DEPTH && (i != 0 || item.depth == 0) && item.depth <= previous_depth + 1,
          "Invalid checklist nesting"
        );
        previous_depth = item.depth;
      }
    }
    Ok(())
  }

  pub fn progress(&self) -> (usize, usize) {
    self.lists.iter().map(Checklist::progress).fold((0, 0), |(a, b), (c, d)| (a + c, b + d))
  }
}

fn valid_text(text: &str) -> Result<()> {
  ensure!(!text.trim().is_empty(), "Text must not be empty");
  ensure!(
    !text.chars().any(char::is_control),
    "Text must be a single line without control characters"
  );
  Ok(())
}

pub fn raw_value(task: &Task) -> Result<Option<String>> {
  match task.uda().get(UDA) {
    None => Ok(None),
    Some(UDAValue::Str(raw)) => Ok(Some(raw.clone())),
    _ => bail!("The {UDA} UDA must be a string; data was not changed"),
  }
}

pub fn from_task(task: &Task) -> Result<Document> {
  Document::decode(raw_value(task)?.as_deref())
}

pub fn summary(task: &Task) -> String {
  if !task.uda().contains_key(UDA) {
    return String::new();
  }
  match from_task(task) {
    Ok(document) => {
      let (done, total) = document.progress();
      format!("{done}/{total}")
    }
    Err(_) => "invalid checklist".into(),
  }
}

async fn run_task(task_exe: &str, args: &[String]) -> Result<Vec<u8>> {
  let mut command = Command::new(task_exe);
  command
    .args([
      "rc.confirmation=off",
      "rc.recurrence.confirmation=off",
      "rc.context=",
      "rc.json.array=on",
      "rc.json.depends.array=on",
      "rc.uda.tuichecklist.type=string",
      "rc.uda.tuichecklist.label=Checklist",
    ])
    .args(args)
    .stdin(Stdio::null())
    .kill_on_drop(true);
  let output = tokio::time::timeout(Duration::from_secs(15), command.output())
    .await
    .context("Taskwarrior timed out; refresh before retrying")?
    .context("Unable to run Taskwarrior")?;
  ensure!(
    output.status.success(),
    "Taskwarrior failed: {} {}",
    String::from_utf8_lossy(&output.stderr).trim(),
    String::from_utf8_lossy(&output.stdout).trim()
  );
  Ok(output.stdout)
}

pub async fn load_task(task_exe: &str, uuid: Uuid) -> Result<Task> {
  let bytes = run_task(task_exe, &[uuid.to_string(), "export".into()]).await?;
  let mut tasks: Vec<Task> = import(bytes.as_slice()).context("Unable to read checklist task")?;
  ensure!(
    tasks.len() == 1 && *tasks[0].uuid() == uuid,
    "Task no longer exists; checklist was not saved"
  );
  Ok(tasks.remove(0))
}

/// A stale-check, not an atomic compare-and-swap. Do not promise cross-client merging.
pub async fn save(task_exe: &str, uuid: Uuid, expected: Option<&str>, document: &Document) -> Result<()> {
  let encoded = document.encode()?;
  let current = load_task(task_exe, uuid).await?;
  let raw = raw_value(&current)?;
  ensure!(
    raw.as_deref() == expected,
    "Checklist changed outside this editor. Cancel, refresh, and retry; your draft has been kept"
  );
  // Validate the original too: unknown/corrupt data must not be replaced by an empty list.
  Document::decode(raw.as_deref())?;
  let value = if document.lists.is_empty() { "" } else { &encoded };
  run_task(task_exe, &[uuid.to_string(), "modify".into(), format!("{UDA}:{value}")]).await?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  const EXAMPLE: &str = include_str!("../tests/fixtures/checklist-ru.md");

  #[test]
  fn russian_markdown_round_trip() {
    let list = Checklist::from_markdown(EXAMPLE).unwrap();
    assert_eq!(list.progress(), (5, 13));
    assert_eq!(list.items.iter().filter(|i| i.depth == 1).count(), 4);
    assert_eq!(list.markdown(), EXAMPLE.trim_end());
    let doc = Document {
      version: 1,
      lists: vec![list],
    };
    assert_eq!(Document::decode(Some(&doc.encode().unwrap())).unwrap(), doc);
  }

  #[test]
  fn markdown_formats_and_invalid_lines() {
    let list = Checklist::from_markdown("# Release\r\n\r\n* [X] Parent\r\n    + [ ] 子\r\n        - [x] deep\r\n- [ ] root\r\n").unwrap();
    assert_eq!(list.title, "Release");
    assert_eq!(list.items.iter().map(|i| i.depth).collect::<Vec<_>>(), vec![0, 1, 2, 0]);
    for text in [
      "",
      "plain text",
      "- [y] no",
      "- [ ] ",
      "- [ ] parent\n\t- [ ] child",
      "- [ ] p\n    - [ ] c\n  - [ ] invalid",
      "- [ ] p\nforgotten prose",
      "- [ ] control\u{1b}",
    ] {
      assert!(Checklist::from_markdown(text).is_err(), "{text:?}");
    }
    assert!(format!("{:#}", Checklist::from_markdown("- [ ] ok\nnot a checkbox").unwrap_err()).contains("Line 2"));
  }

  #[test]
  fn subtree_editing_preserves_siblings_and_ids() {
    let mut list = Checklist::from_markdown("- [ ] p\n  - [ ] a\n    - [ ] grandchild\n  - [ ] b\n- [ ] q").unwrap();
    let id = list.items[1].id;
    list.outdent(1).unwrap();
    assert_eq!(list.markdown(), "- [ ] p\n  - [ ] b\n- [ ] a\n  - [ ] grandchild\n- [ ] q");
    assert_eq!(list.items[2].id, id);
    list.move_item(2, true).unwrap();
    assert_eq!(list.items[3].id, id);
    list.move_item(3, false).unwrap();
    list.indent(2).unwrap();
    assert_eq!(list.markdown(), "- [ ] p\n  - [ ] b\n  - [ ] a\n    - [ ] grandchild\n- [ ] q");
    assert!(list.indent(0).is_err());
    assert!(list.outdent(0).is_err());
    list.items[0].checked = true;
    assert!(!list.items[1].checked);
    list.items.drain(list.subtree(2));
    assert_eq!(list.items.len(), 3);
  }

  #[test]
  fn unsafe_data_is_not_treated_as_empty() {
    let mut doc = Document {
      version: 1,
      lists: vec![Checklist::from_markdown("- [ ] a\n  - [ ] b").unwrap()],
    };
    let raw = doc.encode().unwrap();
    assert!(Document::decode(Some("not json")).is_err());
    assert!(Document::decode(Some(&raw.replace("\"version\":1", "\"version\":2"))).is_err());
    doc.lists[0].items[0].depth = 1;
    assert!(doc.encode().is_err());
    doc.lists[0].items[0].depth = 0;
    doc.lists[0].items[1].id = doc.lists[0].items[0].id;
    assert!(doc.encode().is_err());
    assert!(Checklist::from_markdown(&format!("- [ ] {}", "x".repeat(MAX_BYTES))).is_err());
  }
}
