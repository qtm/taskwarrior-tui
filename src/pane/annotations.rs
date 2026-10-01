use std::process::Command;

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDateTime;
use ratatui::{
  Frame,
  layout::{Constraint, Direction, Layout, Rect},
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, Paragraph},
};
use task_hookrs::{import::import, task::Task};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use crate::{
  action::Action,
  app::Mode,
  config::Config,
  datetime,
  event::KeyCode,
  hyperlink::{LinkedLine, LinkedParagraph, push_wrapped, push_wrapped_spans},
  keyconfig::KeyConfig,
};

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
  fn heading(&self, style: Style, project_style: Option<Style>) -> [Span<'static>; 2] {
    let project = if self.project.is_empty() { "(no project)" } else { &self.project };
    let id = self
      .task_id
      .map(|id| id.to_string())
      .unwrap_or_else(|| self.task_uuid.to_string()[..8].to_string());
    [
      Span::styled(project.to_string(), style.patch(project_style.unwrap_or_default())),
      Span::styled(format!(" / [{id}] {}", self.task), style),
    ]
  }
}

#[derive(Default)]
pub struct AnnotationsState {
  pub visible: bool,
  entries: Vec<AnnotationEntry>,
  previous_mode: Option<Mode>,
  scroll: usize,
  max_scroll: usize,
  viewport_height: usize,
  error: Option<String>,
}

impl AnnotationsState {
  /// Keep the task report active while the pane is open. Never intercept typing in a prompt.
  pub fn toggle(&mut self, mode: &mut Mode) -> bool {
    match mode {
      Mode::Tasks(Action::Report) if self.visible => {
        *mode = self.previous_mode.take().unwrap_or(Mode::Tasks(Action::Report));
        self.visible = false;
      }
      Mode::Tasks(Action::Report) | Mode::Projects | Mode::Timesheet | Mode::Calendar => {
        self.previous_mode = Some(mode.clone());
        *mode = Mode::Tasks(Action::Report);
        self.visible = true;
        self.scroll = 0;
      }
      _ => return false,
    }
    true
  }

  /// Switch back to task details without navigating away from the task report.
  pub fn hide(&mut self) {
    self.visible = false;
    self.previous_mode = None;
  }

  pub fn refresh(&mut self, task_exe: &str) {
    match Self::load_tasks(task_exe) {
      Ok(tasks) => {
        self.entries = Self::entries_from_tasks(&tasks);
        self.error = None;
      }
      Err(error) => self.error = Some(format!("Unable to load annotations: {error:#}")),
    }
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

  /// Checklist-only layout: dates on the left and annotation text on the right.
  /// Sorting, local time formatting and heading colors match the all-task timeline.
  pub(super) fn task_lines(task: &Task, width: u16, config: &Config) -> Vec<LinkedLine> {
    let entries = Self::entries_from_tasks(std::slice::from_ref(task));
    // Two columns need at least one cell each plus a gap. Only degenerate
    // viewports fall back to stacked text, without dropping either value.
    if width < 3 {
      return Self::format_entries(&entries, width, config);
    }
    let mut lines = Vec::new();
    let Some(first) = entries.first() else {
      return lines;
    };
    let heading_style = config.uda_style_title.add_modifier(Modifier::BOLD);
    let heading = first.heading(heading_style, config.project_title_style(&first.project));
    push_wrapped_spans(&mut lines, &heading, width, "", heading_style);

    let dates: Vec<_> = entries.iter().map(|entry| datetime::format_local_date_time(&entry.entry)).collect();
    let gap = if width == 3 { 1 } else { 2 };
    // Reserve at least half the usable width for text. On narrow splits the
    // date wraps within its column; all annotation continuations stay aligned.
    let date_width = dates
      .iter()
      .map(|date| date.width())
      .max()
      .unwrap_or(0)
      .min(usize::from((width - gap) / 2)) as u16;
    let text_width = width - date_width - gap;
    let date_style = config.color.get("color.label").copied().unwrap_or_default();
    for (entry, date) in entries.iter().zip(dates) {
      let mut date_lines = Vec::new();
      let mut text_lines = Vec::new();
      push_wrapped(&mut date_lines, &date, date_width, "", date_style);
      push_wrapped(&mut text_lines, &entry.description, text_width, "", Style::default());
      let row_count = date_lines.len().max(text_lines.len());
      let mut date_lines = date_lines.into_iter();
      let mut text_lines = text_lines.into_iter();
      for _ in 0..row_count {
        let mut date_line = date_lines.next().unwrap_or_default();
        let padding = usize::from(date_width + gap).saturating_sub(date_line.width());
        date_line.text.spans.push(Span::raw(" ".repeat(padding)));
        date_line.append(text_lines.next().unwrap_or_default());
        // Date color belongs to its spans, not the entire combined row.
        date_line.text.style = Style::default();
        lines.push(date_line);
      }
    }
    lines
  }

  fn lines(&self, width: u16, config: &Config) -> Vec<LinkedLine> {
    Self::format_entries(&self.entries, width, config)
  }

  fn format_entries(entries: &[AnnotationEntry], width: u16, config: &Config) -> Vec<LinkedLine> {
    let heading_style = config.uda_style_title.add_modifier(Modifier::BOLD);
    let date_style = config.color.get("color.label").copied().unwrap_or_default();
    let mut lines = Vec::new();
    let mut previous_task = None;
    for entry in entries {
      if previous_task != Some(entry.task_uuid) {
        if previous_task.is_some() {
          lines.push(LinkedLine::default());
        }
        let heading = entry.heading(heading_style, config.project_title_style(&entry.project));
        push_wrapped_spans(&mut lines, &heading, width, "", heading_style);
      }
      push_wrapped(&mut lines, &datetime::format_local_date_time(&entry.entry), width, "  ", date_style);
      push_wrapped(&mut lines, &entry.description, width, "    ", Style::default());
      previous_task = Some(entry.task_uuid);
    }
    lines
  }

  pub fn draw(&mut self, f: &mut Frame, area: Rect, config: &Config, keys: &KeyConfig) {
    let title = format!("Annotations — all tasks | {} annotations (local time)", self.entries.len());
    let block = Block::default()
      .borders(Borders::TOP)
      .border_style(config.uda_style_title_border)
      .title(Line::styled(title, config.uda_style_title));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let chunks = Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Min(0), Constraint::Length(1)])
      .split(inner);
    let body = chunks[0];
    let lines = if let Some(error) = &self.error {
      let mut lines = Vec::new();
      push_wrapped(&mut lines, error, body.width, "", config.uda_style_command_error);
      lines
    } else if self.entries.is_empty() {
      vec![LinkedLine::new("No annotations found.")]
    } else {
      self.lines(body.width, config)
    };
    self.viewport_height = usize::from(body.height);
    self.max_scroll = lines.len().saturating_sub(self.viewport_height.max(1));
    self.scroll = self.scroll.min(self.max_scroll);
    // Slice before rendering instead of using Paragraph's u16 scroll offset, so long
    // annotation histories are not limited to 65,535 lines.
    let visible: Vec<_> = lines.into_iter().skip(self.scroll).take(self.viewport_height).collect();
    f.render_widget(LinkedParagraph(visible), body);
    let toggle = key_label(keys.annotations);
    let refresh = key_label(keys.refresh);
    let footer = format!("{toggle}/Esc: close | Ctrl-e/y: scroll | {refresh}: refresh");
    f.render_widget(Paragraph::new(footer).style(config.uda_style_command), chunks[1]);
  }

  /// Use secondary-pane controls so ordinary movement still selects tasks.
  pub fn handle_navigation(&mut self, input: KeyCode) -> bool {
    let page = self.viewport_height.max(1);
    match input {
      KeyCode::Ctrl('e') => self.scroll = self.scroll.saturating_add(1),
      KeyCode::Ctrl('y') => self.scroll = self.scroll.saturating_sub(1),
      KeyCode::Ctrl('d') => self.scroll = self.scroll.saturating_add(page),
      KeyCode::Ctrl('u') => self.scroll = self.scroll.saturating_sub(page),
      _ => return false,
    }
    self.scroll = self.scroll.min(self.max_scroll);
    true
  }
}

fn key_label(key: KeyCode) -> String {
  match key {
    KeyCode::Char(c) => c.to_string(),
    _ => format!("{key:?}"),
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
    let lines: Vec<String> = state.lines(100, &test_config()).iter().map(ToString::to_string).collect();
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
  fn checklist_annotation_columns_are_newest_first_without_changing_the_global_timeline() {
    let tasks = tasks();
    let task = &tasks[0];
    let config = test_config();
    let columns = AnnotationsState::task_lines(task, 100, &config);
    assert_eq!(columns.len(), 4); // One task heading and one row per annotation.
    assert_eq!(columns[0].to_string(), "work / [1] First task");
    for (row, index) in [1, 2, 0].into_iter().enumerate() {
      let annotation = &task.annotations().unwrap()[index];
      let date = datetime::format_local_date_time(annotation.entry());
      assert_eq!(columns[row + 1].to_string(), format!("{date}  {}", annotation.description()));
    }
    assert!(AnnotationsState::task_lines(&tasks[3], 100, &config).is_empty());
    let timeline = AnnotationsState {
      entries: AnnotationsState::entries_from_tasks(&tasks),
      ..Default::default()
    };
    let lines = timeline.lines(100, &config);
    let newest = datetime::format_local_date_time(&timeline.entries[0].entry);
    assert_eq!(lines[1].to_string(), format!("  {newest}"));
    assert_eq!(lines[2].to_string(), "    newest");
  }

  #[test]
  fn checklist_annotation_columns_keep_multiline_unicode_text_aligned_and_fit_narrow_panes() {
    let config = test_config();
    let mut task = tasks().remove(0);
    task.annotations_mut().unwrap().truncate(1);
    *task.annotations_mut().unwrap()[0].description_mut() = "long annotation text wraps here\nПривет 猫 e\u{301}👩‍💻\n\nlast".into();
    let date = datetime::format_local_date_time(task.annotations().unwrap()[0].entry());
    let lines = AnnotationsState::task_lines(&task, 40, &config);
    let text: Vec<_> = lines.iter().map(ToString::to_string).collect();
    let indent = " ".repeat(21);
    assert_eq!(text[1], format!("{date}  long annotation tex"));
    assert_eq!(text[2], format!("{indent}t wraps here"));
    assert_eq!(text[3], format!("{indent}Привет 猫 e\u{301}👩‍💻"));
    assert_eq!(text[4], indent);
    assert_eq!(text[5], format!("{indent}last"));
    assert!(lines.iter().all(|line| line.width() <= 40));

    *task.annotations_mut().unwrap()[0].description_mut() = "short".into();
    let lines = AnnotationsState::task_lines(&task, 20, &config);
    let text: Vec<_> = lines.iter().map(ToString::to_string).collect();
    let first = text.iter().position(|line| line.starts_with(&date[..9])).unwrap();
    assert_eq!(text[first], format!("{}  short", &date[..9]));
    assert_eq!(text[first + 1], format!("{}  ", &date[9..18]));
    assert_eq!(text[first + 2], format!("{:<9}  ", &date[18..]));
    assert!(lines.iter().all(|line| line.width() <= 20));
    for width in 0..=30 {
      let lines = AnnotationsState::task_lines(&task, width, &config);
      assert!(lines.iter().all(|line| line.width() <= usize::from(width)), "width={width}");
    }
  }

  #[test]
  fn annotation_toggle_restores_each_view_and_does_not_interrupt_prompts() {
    for original in [Mode::Tasks(Action::Report), Mode::Projects, Mode::Timesheet, Mode::Calendar] {
      let mut state = AnnotationsState::default();
      let mut mode = original.clone();
      assert!(state.toggle(&mut mode));
      assert_eq!(mode, Mode::Tasks(Action::Report));
      assert!(state.visible);
      assert!(state.toggle(&mut mode));
      assert_eq!(mode, original);
      assert!(!state.visible);
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
      for visible in [false, true] {
        let mut state = AnnotationsState {
          visible,
          ..Default::default()
        };
        let mut mode = Mode::Tasks(action);
        assert!(!state.toggle(&mut mode));
        assert_eq!(mode, Mode::Tasks(action));
        assert_eq!(state.visible, visible);
      }
    }
  }

  #[test]
  fn annotation_navigation_clamps_and_supports_long_histories() {
    let mut state = AnnotationsState {
      max_scroll: 100_000,
      viewport_height: 20,
      ..Default::default()
    };
    assert!(state.handle_navigation(KeyCode::Ctrl('y')));
    assert_eq!(state.scroll, 0);
    assert!(state.handle_navigation(KeyCode::Ctrl('e')));
    assert_eq!(state.scroll, 1);
    assert!(state.handle_navigation(KeyCode::Ctrl('d')));
    assert_eq!(state.scroll, 21);
    state.scroll = usize::MAX;
    state.handle_navigation(KeyCode::Ctrl('e'));
    assert_eq!(state.scroll, 100_000);
    state.handle_navigation(KeyCode::Ctrl('u'));
    assert_eq!(state.scroll, 99_980);
    for key in [
      KeyCode::Char('j'),
      KeyCode::Char('k'),
      KeyCode::Down,
      KeyCode::Up,
      KeyCode::PageDown,
      KeyCode::Home,
    ] {
      assert!(!state.handle_navigation(key));
      assert_eq!(state.scroll, 99_980);
    }
  }

  fn test_config() -> Config {
    Config::new(
      "data.location /unused\nrule.precedence.color project.\nuda.priority.values H,M,L,",
      "next",
    )
    .unwrap()
  }

  fn render(state: &mut AnnotationsState, width: u16, height: u16) -> String {
    let config = test_config();
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
    assert!(output.contains("Annotations — all tasks"));
    assert!(output.contains("n/Esc: close"));
    assert!(output.contains("Ctrl-e/y: scroll"));
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
    state.scroll = state.max_scroll;
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
  fn project_title_color_only_affects_project_names_even_when_wrapped() {
    use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};
    let mut config = test_config();
    config.uda_style_title = Style::default().fg(Color::Cyan);
    config.project_title_styles.insert("work".into(), Style::default().fg(Color::Green));
    let mut state = AnnotationsState {
      entries: AnnotationsState::entries_from_tasks(&tasks()),
      ..Default::default()
    };
    state.entries.truncate(1);
    state.entries[0].project = "work.猫猫".into();
    state.entries[0].task = "TASK".into();
    state.entries[0].description = "NOTE".into();
    let lines = state.lines(100, &config);
    let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 6));
    LinkedParagraph(lines).render(buffer.area, &mut buffer);
    for x in [0, 1, 2, 3, 4, 5, 7] {
      assert_eq!(buffer[(x, 0)].fg, Color::Green);
    }
    for x in 9..20 {
      assert_eq!(
        buffer[(x, 0)].fg,
        Color::Cyan,
        "separator, ID and task description must keep heading style"
      );
    }
    assert_eq!(buffer[(4, 2)].fg, Color::Reset);
    assert_eq!(buffer[(2, 1)].fg, Color::Reset);

    // The project wraps before the heading's metadata; no color may leak past it.
    let mut wrapped = Buffer::empty(Rect::new(0, 0, 5, 20));
    LinkedParagraph(state.lines(5, &config)).render(wrapped.area, &mut wrapped);
    assert_eq!(wrapped[(0, 0)].fg, Color::Green);
    assert_eq!(wrapped[(0, 1)].symbol(), "猫");
    assert_eq!(wrapped[(0, 1)].fg, Color::Green);
    assert_eq!(wrapped[(4, 1)].symbol(), " ");
    assert_eq!(wrapped[(4, 1)].fg, Color::Cyan);
    assert_eq!(wrapped[(0, 2)].symbol(), "/");
    assert_eq!(wrapped[(0, 2)].fg, Color::Cyan);

    state.entries[0].project.clear();
    let heading = state.lines(100, &config).remove(0);
    assert!(heading.text.spans.iter().all(|span| span.style.fg != Some(Color::Green)));
  }

  #[test]
  fn annotation_hyperlinks_survive_scrolling_resize_and_date_columns() {
    use crate::hyperlink::cell_link;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, widgets::Widget};
    let config = test_config();
    let url = "https://example.com/long/annotation/path?one=1&two=2#fragment";
    let mut task = tasks().remove(0);
    task.annotations_mut().unwrap().truncate(1);
    *task.annotations_mut().unwrap()[0].description_mut() = format!("Read {url}.");
    let mut state = AnnotationsState {
      entries: AnnotationsState::entries_from_tasks(&[task.clone()]),
      ..Default::default()
    };
    let mut terminal = Terminal::new(TestBackend::new(25, 6)).unwrap();
    terminal.draw(|f| state.draw(f, f.area(), &config, &KeyConfig::default())).unwrap();
    state.scroll = state.max_scroll;
    terminal.draw(|f| state.draw(f, f.area(), &config, &KeyConfig::default())).unwrap();
    let links: Vec<_> = terminal.backend().buffer().content.iter().filter_map(cell_link).collect();
    assert!(!links.is_empty());
    assert!(links.iter().all(|(target, _)| *target == url));
    terminal.backend_mut().resize(90, 12);
    terminal.draw(|f| state.draw(f, f.area(), &config, &KeyConfig::default())).unwrap();
    assert_eq!(state.scroll, 0);
    let linked_text: String = terminal
      .backend()
      .buffer()
      .content
      .iter()
      .filter_map(cell_link)
      .map(|(_, glyph)| glyph)
      .collect();
    assert_eq!(linked_text, url);

    let mut columns = Buffer::empty(Rect::new(4, 2, 40, 15));
    LinkedParagraph(AnnotationsState::task_lines(&task, 40, &config)).render(columns.area, &mut columns);
    let mut linked_text = String::new();
    for y in columns.area.top()..columns.area.bottom() {
      for x in columns.area.left()..columns.area.right() {
        if let Some((target, glyph)) = cell_link(&columns[(x, y)]) {
          assert!(x >= columns.area.left() + 21, "Dates and column padding must not be linked");
          assert_eq!(target, url);
          linked_text.push_str(glyph);
        }
      }
    }
    assert_eq!(linked_text, url);
    state.entries.clear();
    terminal.draw(|f| state.draw(f, f.area(), &config, &KeyConfig::default())).unwrap();
    assert!(terminal.backend().buffer().content.iter().all(|cell| cell_link(cell).is_none()));
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
