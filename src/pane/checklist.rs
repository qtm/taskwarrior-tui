use anyhow::{Context, Result, ensure};
use ratatui::{
  Frame,
  layout::{Constraint, Direction, Layout, Rect},
  style::{Modifier, Style},
  text::{Line, Span},
  widgets::{Block, Borders, Paragraph},
};
use rustyline::line_buffer::LineBuffer;
use task_hookrs::task::Task;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use super::annotations::{push_wrapped, push_wrapped_spans};
use crate::{
  action::Action,
  app::{Mode, TaskwarriorTui, handle_movement},
  checklist::{self, Checklist, Document, MAX_BYTES},
  clipboard,
  config::Config,
  event::KeyCode,
  keyconfig::KeyConfig,
  utils,
};

#[derive(Clone)]
pub enum EditKind {
  Import,
  AddItem { list: Option<Uuid>, after: Option<Uuid>, child: bool },
  EditItem { list: Uuid, item: Uuid },
  DeleteItem { list: Uuid, item: Uuid },
  AddList,
  RenameList(Uuid),
  DeleteList(Uuid),
}

pub struct Editor {
  kind: EditKind,
  uuid: Uuid,
  baseline: Option<String>,
  document: Document,
  buffer: LineBuffer,
  pub error: Option<String>,
  notice: Option<String>,
  preview_scroll: usize,
}

impl Editor {
  fn label(&self) -> &str {
    match self.kind {
      EditKind::Import => "Import Markdown: Enter attaches a NEW list; Esc cancels; Ctrl-u clears; Ctrl-n inserts newline",
      EditKind::AddItem { child: true, .. } => "Add child item",
      EditKind::AddItem { .. } => "Add item",
      EditKind::EditItem { .. } => "Edit item",
      EditKind::DeleteItem { .. } => "Delete item AND its children? Enter confirms; Esc cancels",
      EditKind::AddList => "New checklist name",
      EditKind::RenameList(_) => "Rename checklist",
      EditKind::DeleteList(_) => "Delete entire checklist? Enter confirms; Esc cancels",
    }
  }

  fn apply(&self) -> Result<(Document, Option<Uuid>, Option<Uuid>)> {
    let mut doc = self.document.clone();
    let text = self.buffer.as_str().trim().to_string();
    let mut list_id = None;
    let mut item_id = None;
    match self.kind {
      EditKind::Import => {
        let list = Checklist::from_markdown(self.buffer.as_str())?;
        list_id = Some(list.id);
        item_id = list.items.first().map(|item| item.id);
        doc.lists.push(list);
      }
      EditKind::AddList => {
        let list = Checklist::new(text);
        list_id = Some(list.id);
        doc.lists.push(list);
      }
      EditKind::AddItem { list, after, child } => {
        let index = match list {
          Some(id) => doc.lists.iter().position(|list| list.id == id).context("List no longer exists")?,
          None => {
            doc.lists.push(Checklist::new("Checklist".into()));
            doc.lists.len() - 1
          }
        };
        let list = &mut doc.lists[index];
        let after = after
          .map(|id| list.items.iter().position(|item| item.id == id).context("Item no longer exists"))
          .transpose()?;
        item_id = Some(list.insert(after, child, text)?);
        list_id = Some(list.id);
      }
      EditKind::EditItem { list, item } | EditKind::DeleteItem { list, item } => {
        let list = doc
          .lists
          .iter_mut()
          .find(|candidate| candidate.id == list)
          .context("List no longer exists")?;
        let index = list
          .items
          .iter()
          .position(|candidate| candidate.id == item)
          .context("Item no longer exists")?;
        list_id = Some(list.id);
        if matches!(self.kind, EditKind::EditItem { .. }) {
          list.items[index].text = text;
          item_id = Some(item);
        } else {
          list.items.drain(list.subtree(index));
          item_id = list.items.get(index.min(list.items.len().saturating_sub(1))).map(|item| item.id);
        }
      }
      EditKind::RenameList(id) => {
        doc.lists.iter_mut().find(|list| list.id == id).context("List no longer exists")?.title = text;
        list_id = Some(id);
      }
      EditKind::DeleteList(id) => doc.lists.retain(|list| list.id != id),
    }
    doc.encode()?;
    Ok((doc, list_id, item_id))
  }
}

#[derive(Default)]
pub struct ChecklistsState {
  pub visible: bool,
  pub focused: bool,
  pub editor: Option<Editor>,
  task: Option<Task>,
  raw: Option<String>,
  document: Document,
  list_index: usize,
  selected: Option<Uuid>,
  scroll: usize,
  manual_scroll: bool,
  viewport: usize,
  load_error: Option<String>,
  pub error: Option<String>,
  previous_mode: Option<Mode>,
}

impl ChecklistsState {
  pub fn toggle(&mut self, mode: &mut Mode) -> bool {
    if !matches!(mode, Mode::Tasks(Action::Report) | Mode::Projects | Mode::Timesheet | Mode::Calendar) {
      return false;
    }
    if self.visible {
      self.close(mode);
    } else {
      self.previous_mode = Some(mode.clone());
      self.visible = true;
      self.focused = false;
      *mode = Mode::Tasks(Action::Report);
    }
    true
  }

  pub fn close(&mut self, mode: &mut Mode) {
    self.visible = false;
    self.focused = false;
    self.editor = None;
    *mode = self.previous_mode.take().unwrap_or(Mode::Tasks(Action::Report));
  }

  /// Explicit pane switches stay in Tasks, unlike closing a temporarily opened pane.
  pub fn hide(&mut self) {
    self.visible = false;
    self.focused = false;
    self.editor = None;
    self.previous_mode = None;
  }

  pub fn sync_task(&mut self, task: Option<Task>) {
    if self.editor.is_some() {
      return;
    }
    let same_task = self.task.as_ref().map(Task::uuid) == task.as_ref().map(Task::uuid);
    let raw = task.as_ref().map(checklist::raw_value).transpose().map(Option::flatten);
    self.task = task;
    if let Ok(raw) = &raw
      && same_task
      && *raw == self.raw
      && self.load_error.is_none()
    {
      return;
    }
    let old_list = if same_task { self.current_list().map(|list| list.id) } else { None };
    if !same_task {
      self.list_index = 0;
      self.selected = None;
      self.scroll = 0;
      self.manual_scroll = false;
    }
    self.error = None;
    match raw.and_then(|raw| {
      let doc = Document::decode(raw.as_deref())?;
      Ok((raw, doc))
    }) {
      Ok((raw, doc)) => {
        self.raw = raw;
        self.document = doc;
        self.load_error = None;
        self.list_index = old_list
          .and_then(|id| self.document.lists.iter().position(|list| list.id == id))
          .unwrap_or(self.list_index)
          .min(self.document.lists.len().saturating_sub(1));
        self.fix_selection();
      }
      Err(error) => {
        self.load_error = Some(format!("{error:#}"));
        self.document = Document::default();
        self.raw = None;
      }
    }
  }

  fn current_list(&self) -> Option<&Checklist> {
    self.document.lists.get(self.list_index)
  }
  fn selected_index(&self) -> Option<usize> {
    self.current_list()?.items.iter().position(|item| Some(item.id) == self.selected)
  }
  fn fix_selection(&mut self) {
    if self.selected_index().is_none() {
      self.selected = self.current_list().and_then(|list| list.items.first()).map(|item| item.id);
    }
  }

  fn begin(&mut self, kind: EditKind, text: &str) -> Result<()> {
    ensure!(self.load_error.is_none(), "{}", self.load_error.as_deref().unwrap_or_default());
    let uuid = *self.task.as_ref().context("Select a task first")?.uuid();
    let mut buffer = LineBuffer::with_capacity(MAX_BYTES);
    buffer.update(text, text.len(), &mut utils::Changeset::default());
    self.editor = Some(Editor {
      kind,
      uuid,
      baseline: self.raw.clone(),
      document: self.document.clone(),
      buffer,
      error: None,
      notice: None,
      preview_scroll: 0,
    });
    self.error = None;
    self.focused = true;
    Ok(())
  }

  pub fn paste(&mut self, text: &str, changes: &mut utils::Changeset) {
    if let Some(editor) = &mut self.editor {
      if editor.buffer.len() + text.len() > MAX_BYTES {
        editor.error = Some(format!("Input exceeds {MAX_BYTES} bytes; paste was not inserted"));
        return;
      }
      if matches!(editor.kind, EditKind::DeleteItem { .. } | EditKind::DeleteList(_)) {
        return;
      }
      for c in text.chars() {
        editor.buffer.insert(c, 1, changes);
      }
      editor.error = None;
      editor.notice = None;
      editor.preview_scroll = 0;
    }
  }

  pub fn draw_editor(&self, f: &mut Frame, area: Rect, config: &Config) {
    if let Some(editor) = &self.editor {
      let block = Block::default()
        .borders(Borders::TOP)
        .title(editor.label())
        .style(config.uda_style_command);
      let inner = block.inner(area);
      f.render_widget(block, area);
      if inner.width == 0 || inner.height == 0 {
        return;
      }
      if let Some(error) = &editor.error {
        f.render_widget(Paragraph::new(error.as_str()).style(config.uda_style_command_error), inner);
      } else if matches!(editor.kind, EditKind::Import) {
        f.render_widget(
          Paragraph::new("Paste Markdown; Enter: attach new list | Esc: cancel | PgUp/PgDn: preview"),
          inner,
        );
      } else {
        let position = utils::display_width(&editor.buffer.as_str()[..editor.buffer.pos()]);
        let scroll = position.saturating_sub(inner.width.saturating_sub(1) as usize).min(u16::MAX as usize);
        let text = utils::display_control_chars(editor.buffer.as_str());
        f.render_widget(Paragraph::new(text).scroll((0, scroll as u16)), inner);
        if !matches!(editor.kind, EditKind::DeleteItem { .. } | EditKind::DeleteList(_)) {
          f.set_cursor_position((
            inner.x + position.saturating_sub(scroll).min(inner.width.saturating_sub(1) as usize) as u16,
            inner.y,
          ));
        }
      }
    }
  }

  pub fn draw(&mut self, f: &mut Frame, area: Rect, config: &Config, keys: &KeyConfig) {
    let focus = if self.focused { "checklist focus" } else { "task focus — Tab to edit" };
    let title = format!("Checklists — {focus}");
    let block = Block::default()
      .borders(Borders::TOP)
      .border_style(config.uda_style_title_border)
      .title(Line::styled(title, config.uda_style_title));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let regions = Layout::default()
      .direction(Direction::Vertical)
      .constraints([Constraint::Min(0), Constraint::Length(1)])
      .split(inner);
    let body = regions[0];
    self.viewport = usize::from(body.height);
    let mut lines = Vec::new();
    if let Some(task) = &self.task {
      let project = task.project().map(String::as_str).filter(|s| !s.is_empty()).unwrap_or("(no project)");
      let style = config.uda_style_title.add_modifier(Modifier::BOLD);
      let spans = [
        Span::styled(project.to_string(), style.patch(config.project_title_style(project).unwrap_or_default())),
        Span::styled(
          format!(
            " / [{}] {}",
            task.id().map(|id| id.to_string()).unwrap_or_else(|| task.uuid().to_string()[..8].into()),
            utils::display_control_chars(task.description())
          ),
          style,
        ),
      ];
      push_wrapped_spans(&mut lines, &spans, body.width, "", style);
    }
    if let Some(error) = self.editor.as_ref().and_then(|editor| editor.error.as_ref()) {
      push_wrapped(&mut lines, error, body.width, "", config.uda_style_command_error);
    }
    let mut selected_range = None;
    let mut preview_scroll = None;
    if let Some(editor) = &self.editor
      && matches!(editor.kind, EditKind::Import)
    {
      preview_scroll = Some(editor.preview_scroll);
      if let Some(notice) = &editor.notice {
        push_wrapped(&mut lines, notice, body.width, "", config.uda_style_command_error);
      }
      match Checklist::from_markdown(editor.buffer.as_str()) {
        Ok(list) => {
          let (done, total) = list.progress();
          push_wrapped(
            &mut lines,
            &format!("Import preview: {} — {done}/{total} complete (new list)", list.title),
            body.width,
            "",
            config.uda_style_title,
          );
          for item in &list.items {
            wrap_item(&mut lines, item, body.width, Style::default());
          }
        }
        Err(error) => {
          push_wrapped(&mut lines, &format!("{error:#}"), body.width, "", config.uda_style_command_error);
          push_wrapped(
            &mut lines,
            &utils::display_control_chars(editor.buffer.as_str()).replace("^J", "\n"),
            body.width,
            "",
            Style::default(),
          );
        }
      }
    } else if let Some(error) = &self.load_error {
      push_wrapped(&mut lines, error, body.width, "", config.uda_style_command_error);
    } else if let Some(list) = self.current_list() {
      let (done, total) = list.progress();
      push_wrapped(
        &mut lines,
        &format!(
          "# {} — {done}/{total} complete (list {}/{})",
          list.title,
          self.list_index + 1,
          self.document.lists.len()
        ),
        body.width,
        "",
        config.uda_style_title,
      );
      if list.items.is_empty() {
        lines.push(Line::from(format!(
          "Empty checklist. Tab to focus, then {} to add an item.",
          key_label(keys.add)
        )));
      }
      for item in &list.items {
        let start = lines.len();
        let highlighted = Some(item.id) == self.selected && self.focused;
        let style = if highlighted {
          // Do not inherit completed-item dimming into the active selection.
          selection_style(config)
        } else if item.checked {
          Style::default().add_modifier(Modifier::DIM)
        } else {
          Style::default()
        };
        wrap_item(&mut lines, item, body.width, style);
        if highlighted {
          // Paragraph styles only existing glyphs. Pad every continuation line
          // so the highlight covers the pane width, including trailing space.
          for line in &mut lines[start..] {
            line
              .spans
              .push(Span::raw(" ".repeat(usize::from(body.width).saturating_sub(line.width()))));
          }
        }
        if Some(item.id) == self.selected {
          let end = if lines.len() - start > self.viewport { start + 1 } else { lines.len() };
          selected_range = Some(start..end);
        }
      }
    } else if self.task.is_none() {
      lines.push(Line::from("Select a task first."));
    } else {
      lines.push(Line::from(format!(
        "No checklists. Tab then {} to add an item, or {} to import Markdown.",
        key_label(keys.add),
        key_label(keys.import_checklist)
      )));
    }
    if let Some(error) = &self.error {
      push_wrapped(&mut lines, error, body.width, "", config.uda_style_command_error);
    }
    let max_scroll = lines.len().saturating_sub(self.viewport.max(1));
    if let Some(scroll) = preview_scroll {
      self.scroll = scroll.min(max_scroll);
      if let Some(editor) = &mut self.editor {
        editor.preview_scroll = self.scroll;
      }
    } else if self.focused
      && !self.manual_scroll
      && let Some(range) = selected_range
    {
      if range.start < self.scroll {
        self.scroll = range.start;
      }
      if range.end > self.scroll + self.viewport {
        self.scroll = range.end.saturating_sub(self.viewport);
      }
    }
    self.scroll = self.scroll.min(max_scroll);
    f.render_widget(
      Paragraph::new(lines.into_iter().skip(self.scroll).take(self.viewport).collect::<Vec<_>>()),
      body,
    );
    let footer = if let Some(error) = &self.error {
      error.clone()
    } else if self.focused {
      format!(
        "Space: check | {}/o/{}/{}: items | >/<: nest | Alt-j/k: move | [/]: lists | {}: import | Tab: tasks",
        key_label(keys.add),
        key_label(keys.edit),
        key_label(keys.delete),
        key_label(keys.import_checklist)
      )
    } else {
      format!(
        "Tab: focus checklist | task keys remain active | {}/Esc: close",
        key_label(keys.checklist)
      )
    };
    f.render_widget(
      Paragraph::new(footer).style(if self.error.is_some() {
        config.uda_style_command_error
      } else {
        config.uda_style_command
      }),
      regions[1],
    );
  }
}

fn selection_style(config: &Config) -> Style {
  // Unlike the task table, Markdown rows have no separate selection marker.
  // An empty report-selection style therefore needs a visible fallback.
  let mut style = if config.uda_style_report_selection == Style::default() {
    Style::default().add_modifier(Modifier::REVERSED)
  } else {
    config.uda_style_report_selection
  };
  for (enabled, modifier) in [
    (config.uda_selection_bold, Modifier::BOLD),
    (config.uda_selection_italic, Modifier::ITALIC),
    (config.uda_selection_dim, Modifier::DIM),
    (config.uda_selection_blink, Modifier::SLOW_BLINK),
    (config.uda_selection_reverse, Modifier::REVERSED),
  ] {
    if enabled {
      style = style.add_modifier(modifier);
    }
  }
  style
}

fn key_label(key: KeyCode) -> String {
  match key {
    KeyCode::Char(c) => c.to_string(),
    _ => format!("{key:?}"),
  }
}

fn wrap_item(lines: &mut Vec<Line<'static>>, item: &checklist::Item, width: u16, style: Style) {
  let width = usize::from(width);
  if width == 0 {
    return;
  }
  let prefix = format!("{}- [{}] ", "  ".repeat(item.depth as usize), if item.checked { 'x' } else { ' ' });
  if prefix.width() >= width {
    push_wrapped(lines, &format!("{prefix}{}", item.text), width as u16, "", style);
    return;
  }
  let indent = " ".repeat(prefix.width());
  let mut used = prefix.width();
  let mut text = prefix;
  for grapheme in item.text.graphemes(true) {
    let grapheme = if grapheme.width() > width - indent.len() { "�" } else { grapheme };
    if used + grapheme.width() > width {
      lines.push(Line::styled(std::mem::replace(&mut text, indent.clone()), style));
      used = indent.len();
    }
    text.push_str(grapheme);
    used += grapheme.width();
  }
  lines.push(Line::styled(text, style));
}

impl TaskwarriorTui {
  /// Only call in Report mode. Prompt input is handled separately and never leaks to task actions.
  pub async fn handle_checklist_input(&mut self, input: KeyCode) -> Result<bool> {
    if self.mode != Mode::Tasks(Action::Report) {
      return Ok(false);
    }
    if input == self.keyconfig.checklist {
      self.checklists.toggle(&mut self.mode);
      self.checklists.sync_task(self.task_current());
      return Ok(true);
    }
    if input == self.keyconfig.import_checklist {
      if !self.checklists.visible {
        self.checklists.toggle(&mut self.mode);
      }
      self.checklists.sync_task(self.task_current());
      match self.checklists.begin(EditKind::Import, "") {
        Ok(()) => {
          self.mode = Mode::Tasks(Action::Checklist);
          match clipboard::read().await {
            Ok(text) => self.checklists.paste(&text, &mut self.changes),
            Err(error) => self.checklists.editor.as_mut().unwrap().notice = Some(format!("{error:#}")),
          }
        }
        Err(error) => self.checklists.error = Some(format!("{error:#}")),
      }
      return Ok(true);
    }
    if !self.checklists.visible {
      return Ok(false);
    }
    if input == KeyCode::Esc {
      self.checklists.close(&mut self.mode);
      return Ok(true);
    }
    if input == KeyCode::Tab || input == KeyCode::BackTab {
      self.checklists.focused = !self.checklists.focused;
      return Ok(true);
    }
    let scrolled = match input {
      KeyCode::Ctrl('e') => Some(self.checklists.scroll.saturating_add(1)),
      KeyCode::Ctrl('y') => Some(self.checklists.scroll.saturating_sub(1)),
      KeyCode::Ctrl('d') => Some(self.checklists.scroll.saturating_add(self.checklists.viewport.max(1))),
      KeyCode::Ctrl('u') => Some(self.checklists.scroll.saturating_sub(self.checklists.viewport.max(1))),
      _ => None,
    };
    if let Some(scroll) = scrolled {
      self.checklists.scroll = scroll;
      self.checklists.manual_scroll = true;
      return Ok(true);
    }
    if !self.checklists.focused {
      return Ok(false);
    }
    self.checklists.manual_scroll = false;
    // These explicitly safe global commands may use the normal report dispatcher.
    if [
      self.keyconfig.quit,
      self.keyconfig.help,
      self.keyconfig.refresh,
      self.keyconfig.undo,
      self.keyconfig.annotations,
      self.keyconfig.zoom,
      self.keyconfig.transpose,
      KeyCode::Ctrl('c'),
    ]
    .contains(&input)
    {
      return Ok(false);
    }
    self.checklists.error = None;
    let list_id = self.checklists.current_list().map(|list| list.id);
    let selected = self.checklists.selected;
    let index = self.checklists.selected_index();
    let mut edit = None;
    let mut text = String::new();
    if input == self.keyconfig.down
      || input == KeyCode::Down
      || input == self.keyconfig.up
      || input == KeyCode::Up
      || input == self.keyconfig.page_down
      || input == KeyCode::PageDown
      || input == self.keyconfig.page_up
      || input == KeyCode::PageUp
      || input == self.keyconfig.go_to_top
      || input == self.keyconfig.go_to_bottom
    {
      if let Some(list) = self.checklists.current_list() {
        let step = if [self.keyconfig.page_up, self.keyconfig.page_down, KeyCode::PageUp, KeyCode::PageDown].contains(&input) {
          self.checklists.viewport.max(1)
        } else {
          1
        };
        let next = if input == self.keyconfig.go_to_top {
          0
        } else if input == self.keyconfig.go_to_bottom {
          list.items.len().saturating_sub(1)
        } else if [self.keyconfig.up, self.keyconfig.page_up, KeyCode::Up, KeyCode::PageUp].contains(&input) {
          index.unwrap_or(0).saturating_sub(step)
        } else {
          index.unwrap_or(0).saturating_add(step).min(list.items.len().saturating_sub(1))
        };
        self.checklists.selected = list.items.get(next).map(|item| item.id);
      }
    } else if input == KeyCode::Char('[') || input == KeyCode::Char(']') {
      let count = self.checklists.document.lists.len();
      if count > 0 {
        self.checklists.list_index = if input == KeyCode::Char(']') {
          (self.checklists.list_index + 1) % count
        } else {
          (self.checklists.list_index + count - 1) % count
        };
        self.checklists.selected = None;
        self.checklists.fix_selection();
        self.checklists.scroll = 0;
      }
    } else if input == self.keyconfig.add || input == KeyCode::Char('o') {
      edit = Some(EditKind::AddItem {
        list: list_id,
        after: selected,
        child: input == KeyCode::Char('o'),
      });
    } else if input == KeyCode::Char('A') {
      edit = Some(EditKind::AddList);
    } else if input == KeyCode::Char('E') {
      if let Some(list) = self.checklists.current_list() {
        text = list.title.clone();
        edit = Some(EditKind::RenameList(list.id));
      }
    } else if input == KeyCode::Char('X') {
      if let Some(list) = self.checklists.current_list() {
        text = list.title.clone();
        edit = Some(EditKind::DeleteList(list.id));
      }
    } else if let (Some(list), Some(item), Some(index)) = (list_id, selected, index) {
      if input == self.keyconfig.edit {
        text = self.checklists.current_list().unwrap().items[index].text.clone();
        edit = Some(EditKind::EditItem { list, item });
      } else if input == self.keyconfig.delete {
        let current = self.checklists.current_list().unwrap();
        text = format!("{} ({} items)", current.items[index].text, current.subtree(index).len());
        edit = Some(EditKind::DeleteItem { list, item });
      } else {
        let mut document = self.checklists.document.clone();
        let target = &mut document.lists[self.checklists.list_index];
        let operation = match input {
          KeyCode::Char(' ') => {
            target.items[index].checked = !target.items[index].checked;
            Some(Ok(()))
          }
          KeyCode::Char('>') => Some(target.indent(index)),
          KeyCode::Char('<') => Some(target.outdent(index)),
          KeyCode::Alt('j') => Some(target.move_item(index, true)),
          KeyCode::Alt('k') => Some(target.move_item(index, false)),
          _ => None,
        };
        if let Some(result) = operation {
          match result {
            Ok(()) => {
              let uuid = *self.checklists.task.as_ref().unwrap().uuid();
              match checklist::save(&self.task_exe, uuid, self.checklists.raw.as_deref(), &document).await {
                Ok(()) => self.checklist_saved(uuid, Some(list), Some(item)).await,
                Err(error) => self.checklists.error = Some(format!("{error:#}")),
              }
            }
            Err(error) => self.checklists.error = Some(format!("{error:#}")),
          }
        }
      }
    }
    if let Some(edit) = edit {
      match self.checklists.begin(edit, &text) {
        Ok(()) => self.mode = Mode::Tasks(Action::Checklist),
        Err(error) => self.checklists.error = Some(format!("{error:#}")),
      }
    }
    // Never let an unrecognized focused-pane key modify the parent task.
    Ok(true)
  }

  pub async fn handle_checklist_editor(&mut self, input: KeyCode) -> Result<()> {
    if input == KeyCode::Esc {
      self.checklists.editor = None;
      self.mode = Mode::Tasks(Action::Report);
      self.checklists.sync_task(self.task_current());
      return Ok(());
    }
    if input == KeyCode::Char('\n') {
      if let Some(editor) = &self.checklists.editor {
        let uuid = editor.uuid;
        match editor.apply() {
          Ok((doc, list, item)) => match checklist::save(&self.task_exe, uuid, editor.baseline.as_deref(), &doc).await {
            Ok(()) => {
              self.checklists.editor = None;
              self.mode = Mode::Tasks(Action::Report);
              self.checklist_saved(uuid, list, item).await;
            }
            Err(error) => self.checklists.editor.as_mut().unwrap().error = Some(format!("{error:#}")),
          },
          Err(error) => self.checklists.editor.as_mut().unwrap().error = Some(format!("{error:#}")),
        }
      }
    } else if let Some(editor) = &mut self.checklists.editor {
      if matches!(editor.kind, EditKind::DeleteItem { .. } | EditKind::DeleteList(_)) {
        return Ok(());
      }
      let import = matches!(editor.kind, EditKind::Import);
      match input {
        KeyCode::Ctrl('u') if import => {
          editor.buffer.update("", 0, &mut self.changes);
          editor.notice = None;
          editor.preview_scroll = 0;
        }
        KeyCode::Ctrl('n') if import && editor.buffer.len() < MAX_BYTES => {
          editor.buffer.insert('\n', 1, &mut self.changes);
        }
        KeyCode::PageDown if import => editor.preview_scroll = editor.preview_scroll.saturating_add(self.checklists.viewport.max(1)),
        KeyCode::PageUp if import => editor.preview_scroll = editor.preview_scroll.saturating_sub(self.checklists.viewport.max(1)),
        KeyCode::Char(c) if editor.buffer.len() + c.len_utf8() > MAX_BYTES => {
          editor.error = Some("Input is too large".into());
          return Ok(());
        }
        _ => handle_movement(&mut editor.buffer, input, &mut self.changes),
      }
      editor.error = None;
    }
    Ok(())
  }

  async fn checklist_saved(&mut self, uuid: Uuid, list: Option<Uuid>, item: Option<Uuid>) {
    self.current_selection_uuid = Some(uuid);
    if let Err(error) = self.update(true).await {
      self.checklists.error = Some(format!("Saved, but refresh failed: {error:#}"));
      return;
    }
    self.checklists.sync_task(self.task_current());
    if self.checklists.task.as_ref().map(Task::uuid) == Some(&uuid) {
      if let Some(index) = list.and_then(|id| self.checklists.document.lists.iter().position(|list| list.id == id)) {
        self.checklists.list_index = index;
      }
      self.checklists.selected = item;
      self.checklists.fix_selection();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Color};

  fn selection_fixture() -> (ChecklistsState, Config) {
    let config = Config::new(
      "data.location /unused\nrule.precedence.color project.\nuda.priority.values H,M,L,",
      "next",
    )
    .unwrap();
    let markdown = format!(
      "- [x] {}\n  - [ ] Child\n- [x] Other done\n- [ ] Tail",
      "Selected Привет e\u{301} ".repeat(10)
    );
    let list = Checklist::from_markdown(&markdown).unwrap();
    let state = ChecklistsState {
      visible: true,
      focused: true,
      selected: Some(list.items[0].id),
      document: Document {
        version: 1,
        lists: vec![list],
      },
      ..Default::default()
    };
    (state, config)
  }

  fn render_selection(terminal: &mut Terminal<TestBackend>, state: &mut ChecklistsState, config: &Config) -> Buffer {
    terminal
      .draw(|frame| state.draw(frame, frame.area(), config, &KeyConfig::default()))
      .unwrap();
    terminal.backend().buffer().clone()
  }

  fn row_with(buffer: &Buffer, text: &str) -> u16 {
    (0..buffer.area.height)
      .find(|&y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>().contains(text))
      .unwrap_or_else(|| panic!("Missing {text:?} in {buffer:?}"))
  }

  #[test]
  fn selection_is_visible_by_default_across_wrapped_rows_and_tracks_focus() {
    let (mut state, config) = selection_fixture();
    assert_eq!(config.uda_style_report_selection, Style::default());
    let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
    let buffer = render_selection(&mut terminal, &mut state, &config);
    let first = row_with(&buffer, "- [x] Selected");
    let child = row_with(&buffer, "- [ ] Child");
    assert!(child > first + 1, "The selected item must wrap");
    for y in first..child {
      for x in 0..70 {
        assert!(buffer[(x, y)].modifier.contains(Modifier::BOLD | Modifier::REVERSED), "({x}, {y})");
        assert!(!buffer[(x, y)].modifier.contains(Modifier::DIM), "Checked selection must remain readable");
      }
    }
    assert!(!buffer[(2, child)].modifier.contains(Modifier::REVERSED));
    let other = row_with(&buffer, "Other done");
    assert!(buffer[(0, other)].modifier.contains(Modifier::DIM));

    // Moving selection removes all old continuation-line highlights.
    state.selected = Some(state.document.lists[0].items[1].id);
    let buffer = render_selection(&mut terminal, &mut state, &config);
    for y in first..child {
      assert!(!buffer[(0, y)].modifier.contains(Modifier::REVERSED));
      assert!(!buffer[(69, y)].modifier.contains(Modifier::REVERSED));
    }
    assert!(buffer[(2, child)].modifier.contains(Modifier::BOLD | Modifier::REVERSED));
    assert!(buffer[(69, child)].modifier.contains(Modifier::REVERSED));

    // Tab back to tasks removes the active checklist highlight.
    state.focused = false;
    let buffer = render_selection(&mut terminal, &mut state, &config);
    assert!(!buffer[(2, child)].modifier.intersects(Modifier::BOLD | Modifier::REVERSED));
    assert!(!buffer[(69, child)].modifier.contains(Modifier::REVERSED));
    assert!(buffer[(6, first)].modifier.contains(Modifier::DIM));
  }

  #[test]
  fn selection_respects_custom_style_and_highlights_scrolled_continuations() {
    let (mut state, mut config) = selection_fixture();
    config.uda_style_report_selection = Style::default().fg(Color::Yellow).bg(Color::Blue);
    config.uda_selection_bold = false;
    config.uda_selection_italic = true;
    state.manual_scroll = true;
    state.scroll = 2;
    let mut terminal = Terminal::new(TestBackend::new(70, 5)).unwrap();
    let buffer = render_selection(&mut terminal, &mut state, &config);
    // The top visible body line is a continuation of the checked selected item.
    assert_eq!(buffer[(0, 1)].symbol(), " ");
    for x in 0..70 {
      let cell = &buffer[(x, 1)];
      assert_eq!(cell.fg, Color::Yellow);
      assert_eq!(cell.bg, Color::Blue);
      assert!(cell.modifier.contains(Modifier::ITALIC));
      assert!(!cell.modifier.intersects(Modifier::BOLD | Modifier::REVERSED | Modifier::DIM));
    }
    config.uda_selection_reverse = true;
    config.uda_selection_dim = true;
    config.uda_selection_blink = true;
    let buffer = render_selection(&mut terminal, &mut state, &config);
    assert!(
      buffer[(69, 1)]
        .modifier
        .contains(Modifier::REVERSED | Modifier::DIM | Modifier::SLOW_BLINK)
    );
  }

  #[test]
  fn markdown_wrap_uses_hanging_indent_and_graphemes() {
    let mut lines = Vec::new();
    let item = checklist::Item {
      id: Uuid::new_v4(),
      text: "Привет 猫 e\u{301}👩‍💻".into(),
      checked: true,
      depth: 1,
    };
    wrap_item(&mut lines, &item, 17, Style::default());
    let rendered: Vec<String> = lines
      .iter()
      .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect())
      .collect();
    assert!(rendered[0].starts_with("  - [x] Привет"));
    assert!(rendered.iter().skip(1).all(|line| line.starts_with("        ")));
    assert!(rendered.iter().all(|line| line.width() <= 17));
    assert!(rendered.join("").contains("e\u{301}👩‍💻"));
    for width in 0..=8 {
      wrap_item(&mut Vec::new(), &item, width, Style::default());
    }
  }

  #[test]
  fn import_appends_without_changing_existing_items() {
    let existing = Checklist::from_markdown("- [x] existing").unwrap();
    let mut editor = Editor {
      kind: EditKind::Import,
      uuid: Uuid::new_v4(),
      baseline: None,
      document: Document {
        version: 1,
        lists: vec![existing.clone()],
      },
      buffer: LineBuffer::with_capacity(MAX_BYTES),
      error: None,
      notice: None,
      preview_scroll: 0,
    };
    editor.buffer.update("- [ ] new\n  - [x] child", 0, &mut utils::Changeset::default());
    let (doc, _, _) = editor.apply().unwrap();
    assert_eq!(doc.lists.len(), 2);
    assert_eq!(doc.lists[0], existing);
    assert_eq!(doc.lists[1].items[1].depth, 1);
    assert_eq!(editor.document.lists.len(), 1);
  }
}
