use std::{process::Command, time::SystemTime};

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDateTime;
use ratatui::{
  Frame,
  layout::{Constraint, Direction, Layout, Rect},
  style::{Modifier, Style},
  text::Line,
  widgets::Paragraph,
};
use task_hookrs::{import::import, task::Task};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use crate::{action::Action, app::Mode, config::Config, datetime, event::KeyCode, keyconfig::KeyConfig};

#[derive(Debug, PartialEq, Eq)]
struct AnnotationEntry {
  task_uuid: Uuid,
  task_id: Option<u64>,
  project: String,
  task: String,
  entry: NaiveDateTime,
  description: String,
}

impl AnnotationEntry {
  fn heading(&self) -> String {
    let project = if self.project.is_empty() { "(no project)" } else { &self.project };
    let id = self
      .task_id
      .map(|id| id.to_string())
      .unwrap_or_else(|| self.task_uuid.to_string()[..8].to_string());
    format!("{project} / [{id}] {}", self.task)
  }
}

#[derive(Default)]
pub struct AnnotationsState {
  pub last_refresh: Option<SystemTime>,
  entries: Vec<AnnotationEntry>,
  previous_mode: Option<Mode>,
  scroll: usize,
  max_scroll: usize,
  viewport_height: usize,
  error: Option<String>,
}

impl AnnotationsState {
  /// This view is a detour, not part of the normal tab cycle. Never intercept typing in a prompt.
  pub fn toggle(&mut self, mode: &mut Mode) -> bool {
    match mode {
      Mode::Annotations => {
        *mode = self.previous_mode.take().unwrap_or(Mode::Tasks(Action::Report));
      }
      Mode::Tasks(Action::Report) | Mode::Projects | Mode::Timesheet | Mode::Calendar => {
        self.previous_mode = Some(mode.clone());
        *mode = Mode::Annotations;
        self.scroll = 0;
      }
      _ => return false,
    }
    true
  }

  pub fn refresh(&mut self, task_exe: &str) {
    match Self::load_tasks(task_exe) {
      Ok(tasks) => {
        self.entries = Self::entries_from_tasks(&tasks);
        self.error = None;
      }
      Err(error) => self.error = Some(format!("Unable to load annotations: {error:#}")),
    }
    self.last_refresh = Some(SystemTime::now());
  }

  fn load_tasks(task_exe: &str) -> Result<Vec<Task>> {
    // Explicitly bypass both the active context and any customized all-report filter.
    // Exporting all retains annotations on completed, deleted, and waiting tasks too.
    let output = Command::new(task_exe)
      .args([
        "rc.json.array=on",
        "rc.json.depends.array=on",
        "rc.confirmation=off",
        "rc.color=off",
        "rc._forcecolor=off",
        "rc.context=",
        "rc.report.all.filter=",
        "export",
        "all",
      ])
      .output()
      .context("Unable to run Taskwarrior export")?;
    if !output.status.success() {
      return Err(anyhow!(
        "Taskwarrior export failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
      ));
    }
    import(output.stdout.as_slice()).context("Unable to parse Taskwarrior export")
  }

  fn entries_from_tasks(tasks: &[Task]) -> Vec<AnnotationEntry> {
    let mut entries = Vec::new();
    for task in tasks {
      if let Some(annotations) = task.annotations() {
        for annotation in annotations {
          entries.push(AnnotationEntry {
            task_uuid: *task.uuid(),
            task_id: task.id().filter(|id| *id != 0),
            project: task.project().cloned().unwrap_or_default(),
            task: task.description().clone(),
            entry: **annotation.entry(),
            description: annotation.description().clone(),
          });
        }
      }
    }
    // Sort UTC instants, not formatted local times (which can repeat across DST changes).
    // Deterministic tie breakers keep equal-timestamp entries stable across exports.
    entries.sort_by(|a, b| {
      b.entry
        .cmp(&a.entry)
        .then(a.task_uuid.cmp(&b.task_uuid))
        .then(a.description.cmp(&b.description))
    });
    entries
  }

  fn lines(&self, width: u16, heading_style: Style, date_style: Style) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut previous_task = None;
    for entry in &self.entries {
      if previous_task != Some(entry.task_uuid) {
        if previous_task.is_some() {
          lines.push(Line::default());
        }
        push_wrapped(&mut lines, &entry.heading(), width, "", heading_style);
      }
      push_wrapped(&mut lines, &datetime::format_local_date_time(&entry.entry), width, "  ", date_style);
      push_wrapped(&mut lines, &entry.description, width, "    ", Style::default());
      previous_task = Some(entry.task_uuid);
    }
    lines
  }

  pub fn draw(&mut self, f: &mut Frame, area: Rect, config: &Config, keys: &KeyConfig) {
    let chunks = Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Min(0), Constraint::Length(1)])
      .split(area);
    let body = chunks[0];
    let lines = if let Some(error) = &self.error {
      let mut lines = Vec::new();
      push_wrapped(&mut lines, error, body.width, "", config.uda_style_command_error);
      lines
    } else if self.entries.is_empty() {
      vec![Line::from("No annotations found.")]
    } else {
      self.lines(
        body.width,
        config.uda_style_title.add_modifier(Modifier::BOLD),
        config.color.get("color.label").copied().unwrap_or_default(),
      )
    };
    self.viewport_height = usize::from(body.height);
    self.max_scroll = lines.len().saturating_sub(self.viewport_height.max(1));
    self.scroll = self.scroll.min(self.max_scroll);
    // Slice before rendering instead of using Paragraph's u16 scroll offset, so long
    // annotation histories are not limited to 65,535 lines.
    let visible: Vec<_> = lines.into_iter().skip(self.scroll).take(self.viewport_height).collect();
    f.render_widget(Paragraph::new(visible), body);
    let toggle = key_label(keys.annotations);
    let refresh = key_label(keys.refresh);
    let footer = format!(
      "{toggle}/Esc: back | {refresh}: refresh | {}/{}: scroll | {} annotations (local time)",
      key_label(keys.down),
      key_label(keys.up),
      self.entries.len()
    );
    f.render_widget(Paragraph::new(footer).style(config.uda_style_command), chunks[1]);
  }

  pub fn handle_navigation(&mut self, input: KeyCode, keys: &KeyConfig) {
    let page = self.viewport_height.max(1);
    if input == keys.down || input == KeyCode::Down {
      self.scroll = self.scroll.saturating_add(1);
    } else if input == keys.up || input == KeyCode::Up {
      self.scroll = self.scroll.saturating_sub(1);
    } else if input == keys.page_down || input == KeyCode::PageDown {
      self.scroll = self.scroll.saturating_add(page);
    } else if input == keys.page_up || input == KeyCode::PageUp {
      self.scroll = self.scroll.saturating_sub(page);
    } else if input == keys.go_to_top || input == KeyCode::Home {
      self.scroll = 0;
    } else if input == keys.go_to_bottom || input == KeyCode::End {
      self.scroll = self.max_scroll;
    }
    self.scroll = self.scroll.min(self.max_scroll);
  }
}

fn key_label(key: KeyCode) -> String {
  match key {
    KeyCode::Char(c) => c.to_string(),
    _ => format!("{key:?}"),
  }
}

/// Wrap at grapheme boundaries, preserving newlines and indentation. Prewrapping
/// makes scrolling and resize clamping use the exact number of displayed lines.
fn push_wrapped(lines: &mut Vec<Line<'static>>, text: &str, width: u16, indent: &str, style: Style) {
  let width = usize::from(width);
  if width == 0 {
    return;
  }
  let indent = if indent.width() < width { indent } else { "" };
  for source in text.split('\n') {
    let mut line = indent.to_string();
    let mut used = indent.width();
    for grapheme in source.graphemes(true) {
      // A terminal cannot display a wide grapheme in a one-column viewport.
      let grapheme = if grapheme.width() > width - indent.width() { "�" } else { grapheme };
      let size = grapheme.width();
      if used + size > width {
        lines.push(Line::styled(line, style));
        line = indent.to_string();
        used = indent.width();
      }
      line.push_str(grapheme);
      used += size;
    }
    lines.push(Line::styled(line, style));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn tasks() -> Vec<Task> {
    import(
      br#"[
        {"uuid":"00000000-0000-0000-0000-000000000001","id":1,"entry":"20251201T120000Z","status":"pending","description":"First task","project":"work",
         "annotations":[{"entry":"20260101T120000Z","description":"oldest"},
                        {"entry":"20260104T120000Z","description":"newest"},
                        {"entry":"20260103T120000Z","description":"adjacent"}]},
        {"uuid":"00000000-0000-0000-0000-000000000002","id":0,"entry":"20251201T120000Z","status":"completed","description":"Other task",
         "annotations":[{"entry":"20260102T120000Z","description":"middle"}]},
        {"uuid":"00000000-0000-0000-0000-000000000003","entry":"20251201T120000Z","status":"deleted","description":"Deleted task",
         "annotations":[{"entry":"20251231T120000Z","description":"retained"}]},
        {"uuid":"00000000-0000-0000-0000-000000000004","entry":"20251201T120000Z","status":"waiting","description":"No annotations"}
      ]"#
        .as_slice(),
    )
    .unwrap()
  }

  #[test]
  fn annotation_entries_are_individual_and_newest_first_across_all_statuses() {
    let entries = AnnotationsState::entries_from_tasks(&tasks());
    assert_eq!(
      entries.iter().map(|e| e.description.as_str()).collect::<Vec<_>>(),
      ["newest", "adjacent", "middle", "oldest", "retained"]
    );
    assert!(entries.windows(2).all(|pair| pair[0].entry >= pair[1].entry));
  }

  #[test]
  fn annotation_grouping_only_merges_consecutive_entries_from_the_same_task() {
    let state = AnnotationsState {
      entries: AnnotationsState::entries_from_tasks(&tasks()),
      ..Default::default()
    };
    let lines: Vec<String> = state
      .lines(100, Style::default(), Style::default())
      .iter()
      .map(ToString::to_string)
      .collect();
    assert_eq!(lines.iter().filter(|line| line.as_str() == "work / [1] First task").count(), 2);
    for annotation in ["newest", "adjacent", "middle", "oldest", "retained"] {
      assert_eq!(lines.iter().filter(|line| line.trim() == annotation).count(), 1);
    }
    assert!(lines.iter().any(|line| line.contains("(no project) / [00000000] Other task")));
  }

  #[test]
  fn annotation_sort_ties_are_deterministic_and_different_tasks_are_not_grouped() {
    let mut tasks = tasks();
    let mut second = tasks[0].clone();
    *second.uuid_mut() = Uuid::from_u128(10);
    tasks.push(second);
    let first_order = AnnotationsState::entries_from_tasks(&tasks);
    tasks.reverse();
    assert_eq!(first_order, AnnotationsState::entries_from_tasks(&tasks));
    assert_ne!(first_order[0].task_uuid, first_order[1].task_uuid);
  }

  #[test]
  fn annotation_toggle_restores_each_view_and_does_not_interrupt_prompts() {
    for original in [Mode::Tasks(Action::Report), Mode::Projects, Mode::Timesheet, Mode::Calendar] {
      let mut state = AnnotationsState::default();
      let mut mode = original.clone();
      assert!(state.toggle(&mut mode));
      assert_eq!(mode, Mode::Annotations);
      assert!(state.toggle(&mut mode));
      assert_eq!(mode, original);
      assert!(state.previous_mode.is_none());
    }
    for action in [
      Action::Add,
      Action::Annotate,
      Action::Modify,
      Action::Filter,
      Action::ContextMenu,
      Action::ReportMenu,
      Action::Error,
    ] {
      let mut state = AnnotationsState::default();
      let mut mode = Mode::Tasks(action);
      assert!(!state.toggle(&mut mode));
      assert_eq!(mode, Mode::Tasks(action));
    }
  }

  #[test]
  fn annotation_navigation_clamps_and_supports_long_histories() {
    let keys = KeyConfig::default();
    let mut state = AnnotationsState {
      max_scroll: 100_000,
      viewport_height: 20,
      ..Default::default()
    };
    state.handle_navigation(keys.up, &keys);
    assert_eq!(state.scroll, 0);
    state.handle_navigation(keys.page_down, &keys);
    assert_eq!(state.scroll, 20);
    state.handle_navigation(keys.go_to_bottom, &keys);
    state.handle_navigation(keys.down, &keys);
    assert_eq!(state.scroll, 100_000);
    state.handle_navigation(keys.page_up, &keys);
    assert_eq!(state.scroll, 99_980);
    state.handle_navigation(keys.go_to_top, &keys);
    assert_eq!(state.scroll, 0);
  }

  fn render(state: &mut AnnotationsState, width: u16, height: u16) -> String {
    let config = Config::new(
      "data.location /unused\nrule.precedence.color project.\nuda.priority.values H,M,L,",
      "next",
    )
    .unwrap();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| state.draw(f, f.area(), &config, &KeyConfig::default())).unwrap();
    terminal
      .backend()
      .buffer()
      .content
      .chunks(usize::from(width).max(1))
      .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
      .collect::<Vec<_>>()
      .join("\n")
  }

  #[test]
  fn annotation_render_shows_metadata_individual_entries_and_group_headings() {
    let mut state = AnnotationsState {
      entries: AnnotationsState::entries_from_tasks(&tasks()),
      ..Default::default()
    };
    let output = render(&mut state, 100, 30);
    assert_eq!(output.matches("work / [1] First task").count(), 2);
    assert!(output.contains("(no project) / [00000000] Other task"));
    assert!(output.contains(&datetime::format_local_date_time(&state.entries[0].entry)));
    assert!(output.find("newest").unwrap() < output.find("adjacent").unwrap());
    assert!(output.find("adjacent").unwrap() < output.find("middle").unwrap());
    assert!(output.find("middle").unwrap() < output.find("oldest").unwrap());
    assert!(output.contains("n/Esc: back"));
    assert!(output.contains("5 annotations (local time)"));
  }

  #[test]
  fn annotation_render_handles_empty_errors_wrapping_and_resize() {
    let mut state = AnnotationsState::default();
    assert!(render(&mut state, 80, 5).contains("No annotations found."));
    state.error = Some("Unable to load annotations: backend unavailable".into());
    assert!(render(&mut state, 80, 5).contains("backend unavailable"));
    state.error = None;
    state.entries = AnnotationsState::entries_from_tasks(&tasks());
    state.entries.truncate(1);
    state.entries[0].description = format!("{}\nLAST", "Long annotation 猫 ".repeat(12));
    render(&mut state, 12, 6);
    assert!(state.max_scroll > 6);
    state.handle_navigation(KeyCode::End, &KeyConfig::default());
    assert!(render(&mut state, 12, 6).contains("LAST"));
    render(&mut state, 100, 50);
    assert_eq!(state.scroll, 0);
    for width in 0..=4 {
      for height in 0..=3 {
        render(&mut state, width, height);
      }
    }
  }

  #[cfg(unix)]
  struct FakeTask(std::path::PathBuf);

  #[cfg(unix)]
  impl FakeTask {
    fn new() -> Self {
      use std::os::unix::fs::PermissionsExt;
      let dir = std::env::temp_dir().join(format!("taskwarrior-tui-annotations-{}", Uuid::new_v4()));
      std::fs::create_dir(&dir).unwrap();
      let task = dir.join("task");
      std::fs::write(&task, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\ncat \"$0.json\"\n").unwrap();
      std::fs::set_permissions(&task, std::fs::Permissions::from_mode(0o700)).unwrap();
      Self(dir)
    }

    fn executable(&self) -> String {
      self.0.join("task").to_str().unwrap().to_string()
    }

    fn output(&self, text: &str) {
      std::fs::write(self.0.join("task.json"), text).unwrap();
    }
  }

  #[cfg(unix)]
  impl Drop for FakeTask {
    fn drop(&mut self) {
      let _ = std::fs::remove_dir_all(&self.0);
    }
  }

  #[cfg(unix)]
  #[test]
  fn annotation_export_bypasses_context_and_report_and_recovers_from_errors() {
    let task = FakeTask::new();
    let mut state = AnnotationsState::default();
    task.output(
      r#"[{"uuid":"00000000-0000-0000-0000-000000000001","entry":"20260101T000000Z","status":"completed","description":"Finished",
      "annotations":[{"entry":"20260102T000000Z","description":"Still visible"}]}]"#,
    );
    state.refresh(&task.executable());
    assert!(state.error.is_none(), "{:?}", state.error);
    assert_eq!(state.entries.len(), 1);
    let args = std::fs::read_to_string(task.0.join("task.args")).unwrap();
    assert!(args.lines().any(|arg| arg == "rc.context="));
    assert!(args.lines().any(|arg| arg == "rc.report.all.filter="));
    assert!(args.ends_with("export\nall\n"));

    task.output("invalid JSON");
    state.refresh(&task.executable());
    assert!(state.error.as_ref().unwrap().contains("Unable to parse"));
    task.output("[]");
    state.refresh(&task.executable());
    assert!(state.error.is_none());
    assert!(state.entries.is_empty());

    std::fs::write(task.0.join("task"), "#!/bin/sh\necho 'export denied' >&2\nexit 2\n").unwrap();
    state.refresh(&task.executable());
    assert!(state.error.as_ref().unwrap().contains("export denied"));
    std::fs::remove_file(task.0.join("task")).unwrap();
    state.refresh(&task.executable());
    assert!(state.error.as_ref().unwrap().contains("Unable to run Taskwarrior export"));
  }

  #[test]
  fn annotation_wrapping_preserves_unicode_and_newlines() {
    let mut lines = Vec::new();
    push_wrapped(&mut lines, "猫猫猫\ne\u{301}nd", 6, "  ", Style::default());
    assert_eq!(
      lines.iter().map(ToString::to_string).collect::<Vec<_>>(),
      ["  猫猫", "  猫", "  e\u{301}nd"]
    );
    assert!(lines.iter().all(|line| line.width() <= 6));
    push_wrapped(&mut lines, "hidden", 0, "", Style::default());
    assert_eq!(lines.len(), 3);
  }
}
