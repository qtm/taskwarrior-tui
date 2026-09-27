use std::{
  collections::{HashMap, HashSet},
  fmt::Display,
  iter::{self, Iterator},
};

use kasuari::{
  Expression, Solver, Strength, Variable,
  WeightedRelation::{EQ, GE, LE},
};
use ratatui::{
  buffer::Buffer,
  layout::{Constraint, Rect},
  style::Style,
  widgets::{Block, StatefulWidget, Widget},
};
use unicode_segmentation::{Graphemes, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

const MEDIUM: Strength = Strength::MEDIUM;
const REQUIRED: Strength = Strength::REQUIRED;
const WEAK: Strength = Strength::WEAK;

#[derive(Debug, Clone)]
pub enum TableMode {
  SingleSelection,
  MultipleSelection,
}

#[derive(Clone)]
pub struct TaskwarriorTuiTableState {
  offset: usize,
  current_selection: Option<usize>,
  marked: HashSet<usize>,
  mode: TableMode,
}

impl Default for TaskwarriorTuiTableState {
  fn default() -> TaskwarriorTuiTableState {
    TaskwarriorTuiTableState {
      offset: 0,
      current_selection: Some(0),
      marked: HashSet::new(),
      mode: TableMode::SingleSelection,
    }
  }
}

impl TaskwarriorTuiTableState {
  pub fn mode(&self) -> TableMode {
    self.mode.clone()
  }

  pub fn multiple_selection(&mut self) {
    self.mode = TableMode::MultipleSelection;
  }

  pub fn single_selection(&mut self) {
    self.mode = TableMode::SingleSelection;
  }

  pub fn current_selection(&self) -> Option<usize> {
    self.current_selection
  }

  pub fn select(&mut self, index: Option<usize>) {
    self.current_selection = index;
    if index.is_none() {
      self.offset = 0;
    }
  }

  pub fn mark(&mut self, index: Option<usize>) {
    if let Some(i) = index {
      self.marked.insert(i);
    }
  }

  pub fn unmark(&mut self, index: Option<usize>) {
    if let Some(i) = index {
      self.marked.remove(&i);
    }
  }

  pub fn toggle_mark(&mut self, index: Option<usize>) {
    if let Some(i) = index
      && !self.marked.insert(i)
    {
      self.marked.remove(&i);
    }
  }

  pub fn marked(&'_ self) -> std::collections::hash_set::Iter<'_, usize> {
    self.marked.iter()
  }

  pub fn clear(&mut self) {
    self.marked.drain().for_each(drop);
  }
}

/// Holds data to be displayed in a Table widget
#[derive(Debug, Clone)]
pub enum Row<D>
where
  D: Iterator,
  D::Item: Display,
{
  Data(D),
  StyledData(D, Style),
}

/// A widget to display data in formatted columns
///
/// # Examples
///
/// ```rust
/// # use ratatui::widgets::{Block, Borders, Table, Row};
/// # use ratatui::layout::Constraint;
/// # use ratatui::style::{Style, Color};
/// let row_style = Style::default().fg(Color::White);
/// Table::new(
///         ["Col1", "Col2", "Col3"].into_iter(),
///         vec![
///             Row::StyledData(["Row11", "Row12", "Row13"].into_iter(), row_style),
///             Row::StyledData(["Row21", "Row22", "Row23"].into_iter(), row_style),
///             Row::StyledData(["Row31", "Row32", "Row33"].into_iter(), row_style),
///             Row::Data(["Row41", "Row42", "Row43"].into_iter())
///         ].into_iter()
///     )
///     .block(Block::default().title("Table"))
///     .header_style(Style::default().fg(Color::Yellow))
///     .widths(&[Constraint::Length(5), Constraint::Length(5), Constraint::Length(10)])
///     .style(Style::default().fg(Color::White))
///     .column_spacing(1);
/// ```
#[derive(Debug, Clone)]
pub struct Table<'a, H, R> {
  /// A block to wrap the widget in
  block: Option<Block<'a>>,
  /// Base style for the widget
  style: Style,
  /// Header row for all columns
  header: H,
  /// Style for the header
  header_style: Style,
  /// Width constraints for each column
  widths: &'a [Constraint],
  /// Space between each column
  column_spacing: u16,
  /// Space between the header and the rows
  header_gap: u16,
  /// Style used to render the selected row
  highlight_style: Style,
  /// Symbol in front of the selected row
  highlight_symbol: Option<&'a str>,
  /// Symbol in front of the marked row
  mark_symbol: Option<&'a str>,
  /// Symbol in front of the unmarked row
  unmark_symbol: Option<&'a str>,
  /// Symbol in front of the marked and selected row
  mark_highlight_symbol: Option<&'a str>,
  /// Symbol in front of the unmarked and selected row
  unmark_highlight_symbol: Option<&'a str>,
  /// Text-only style overrides keyed by absolute (row, column) indices.
  cell_styles: HashMap<(usize, usize), Style>,
  /// Data to display in each row
  rows: R,
}

impl<'a, H, R> Default for Table<'a, H, R>
where
  H: Iterator + Default,
  R: Iterator + Default,
{
  fn default() -> Table<'a, H, R> {
    Table {
      block: None,
      style: Style::default(),
      header: H::default(),
      header_style: Style::default(),
      widths: &[],
      column_spacing: 1,
      header_gap: 1,
      highlight_style: Style::default(),
      highlight_symbol: None,
      mark_symbol: None,
      unmark_symbol: None,
      mark_highlight_symbol: None,
      unmark_highlight_symbol: None,
      cell_styles: HashMap::new(),
      rows: R::default(),
    }
  }
}
impl<'a, H, D, R> Table<'a, H, R>
where
  H: Iterator,
  D: Iterator,
  D::Item: Display,
  R: Iterator<Item = Row<D>>,
{
  pub fn new(header: H, rows: R) -> Table<'a, H, R> {
    Table {
      block: None,
      style: Style::default(),
      header,
      header_style: Style::default(),
      widths: &[],
      column_spacing: 1,
      header_gap: 1,
      highlight_style: Style::default(),
      highlight_symbol: None,
      mark_symbol: None,
      unmark_symbol: None,
      mark_highlight_symbol: None,
      unmark_highlight_symbol: None,
      cell_styles: HashMap::new(),
      rows,
    }
  }
  pub fn block(mut self, block: Block<'a>) -> Table<'a, H, R> {
    self.block = Some(block);
    self
  }

  pub fn header<II>(mut self, header: II) -> Table<'a, H, R>
  where
    II: IntoIterator<Item = H::Item, IntoIter = H>,
  {
    self.header = header.into_iter();
    self
  }

  pub fn header_style(mut self, style: Style) -> Table<'a, H, R> {
    self.header_style = style;
    self
  }

  pub fn widths(mut self, widths: &'a [Constraint]) -> Table<'a, H, R> {
    let between_0_and_100 = |&w| match w {
      Constraint::Percentage(p) => p <= 100,
      _ => true,
    };
    assert!(
      widths.iter().all(between_0_and_100),
      "Percentages should be between 0 and 100 inclusively."
    );
    self.widths = widths;
    self
  }

  pub fn rows<II>(mut self, rows: II) -> Table<'a, H, R>
  where
    II: IntoIterator<Item = Row<D>, IntoIter = R>,
  {
    self.rows = rows.into_iter();
    self
  }

  pub fn style(mut self, style: Style) -> Table<'a, H, R> {
    self.style = style;
    self
  }

  pub fn mark_symbol(mut self, mark_symbol: &'a str) -> Table<'a, H, R> {
    self.mark_symbol = Some(mark_symbol);
    self
  }

  pub fn unmark_symbol(mut self, unmark_symbol: &'a str) -> Table<'a, H, R> {
    self.unmark_symbol = Some(unmark_symbol);
    self
  }

  pub fn mark_highlight_symbol(mut self, mark_highlight_symbol: &'a str) -> Table<'a, H, R> {
    self.mark_highlight_symbol = Some(mark_highlight_symbol);
    self
  }

  pub fn unmark_highlight_symbol(mut self, unmark_highlight_symbol: &'a str) -> Table<'a, H, R> {
    self.unmark_highlight_symbol = Some(unmark_highlight_symbol);
    self
  }

  pub fn highlight_symbol(mut self, highlight_symbol: &'a str) -> Table<'a, H, R> {
    self.highlight_symbol = Some(highlight_symbol);
    self
  }

  pub fn highlight_style(mut self, highlight_style: Style) -> Table<'a, H, R> {
    self.highlight_style = highlight_style;
    self
  }

  pub fn column_spacing(mut self, spacing: u16) -> Table<'a, H, R> {
    self.column_spacing = spacing;
    self
  }

  pub fn header_gap(mut self, gap: u16) -> Table<'a, H, R> {
    self.header_gap = gap;
    self
  }

  /// Override cell text without coloring padding, selection markers, or neighboring cells.
  pub fn cell_styles(mut self, styles: HashMap<(usize, usize), Style>) -> Self {
    self.cell_styles = styles;
    self
  }
}

impl<H, D, R> StatefulWidget for Table<'_, H, R>
where
  H: Iterator,
  H::Item: Display,
  D: Iterator,
  D::Item: Display,
  R: Iterator<Item = Row<D>>,
{
  type State = TaskwarriorTuiTableState;

  fn render(mut self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
    buf.set_style(area, self.style);

    // Render block if necessary and get the drawing area
    let table_area = match self.block.take() {
      Some(b) => {
        let inner_area = b.inner(area);
        b.render(area, buf);
        inner_area
      }
      None => area,
    };

    let mut solver = Solver::new();
    let mut var_indices = HashMap::new();
    let mut ccs = Vec::new();
    let mut variables = Vec::new();
    for i in 0..self.widths.len() {
      let var = Variable::new();
      variables.push(var);
      var_indices.insert(var, i);
    }
    for (i, constraint) in self.widths.iter().enumerate() {
      ccs.push(variables[i] | GE(WEAK) | 0.);
      ccs.push(match *constraint {
        Constraint::Length(v) => variables[i] | EQ(MEDIUM) | f64::from(v),
        Constraint::Percentage(v) => variables[i] | EQ(WEAK) | (f64::from(v * area.width) / 100.0),
        Constraint::Ratio(n, d) => variables[i] | EQ(WEAK) | (f64::from(area.width) * f64::from(n) / f64::from(d)),
        Constraint::Min(v) => variables[i] | GE(WEAK) | f64::from(v),
        Constraint::Max(v) => variables[i] | LE(WEAK) | f64::from(v),
        Constraint::Fill(v) => variables[i] | EQ(WEAK) | f64::from(v),
      });
    }
    solver
      .add_constraint(
        variables.iter().fold(Expression::from_constant(0.), |acc, v| acc + *v)
          | LE(REQUIRED)
          | f64::from(area.width - 2 - (self.column_spacing * (variables.len() as u16 - 1))),
      )
      .unwrap();
    solver.add_constraints(ccs).unwrap();
    let mut solved_widths = vec![0; variables.len()];
    for &(var, value) in solver.fetch_changes() {
      let index = var_indices[&var];
      let value = if value.is_sign_negative() { 0 } else { value as u16 };
      solved_widths[index] = value;
    }

    let mut y = table_area.top();
    let mut x = table_area.left();

    // Draw header
    let mut header_index = usize::MAX;
    let mut index = 0;
    if y < table_area.bottom() {
      for (w, t) in solved_widths.iter().zip(self.header.by_ref()) {
        buf.set_stringn(
          x,
          y,
          format!("{symbol:>width$}", symbol = " ", width = *w as usize),
          *w as usize,
          self.header_style,
        );
        if t.to_string() == "ID" {
          buf.set_stringn(
            x,
            y,
            format!("{symbol:>width$}", symbol = t, width = *w as usize),
            *w as usize,
            self.header_style,
          );
          header_index = index;
        } else {
          buf.set_stringn(x, y, format!("{}", t), *w as usize, self.header_style);
        }
        x += *w + self.column_spacing;
        index += 1;
      }
    }
    y += 1 + self.header_gap;

    // Use highlight_style only if something is selected
    let (selected, highlight_style) = if state.current_selection().is_some() {
      (state.current_selection(), self.highlight_style)
    } else {
      (None, self.style)
    };

    let highlight_symbol = match state.mode {
      TableMode::MultipleSelection => {
        let s = self.highlight_symbol.unwrap_or("\u{2022}").trim_end();
        format!("{} ", s)
      }
      TableMode::SingleSelection => self.highlight_symbol.unwrap_or("").to_string(),
    };

    let mark_symbol = match state.mode {
      TableMode::MultipleSelection => {
        let s = self.mark_symbol.unwrap_or("\u{2714}").trim_end();
        format!("{} ", s)
      }
      TableMode::SingleSelection => self.highlight_symbol.unwrap_or("").to_string(),
    };

    let blank_symbol = match state.mode {
      TableMode::MultipleSelection => {
        let s = self.unmark_symbol.unwrap_or(" ").trim_end();
        format!("{} ", s)
      }
      TableMode::SingleSelection => " ".repeat(highlight_symbol.width()),
    };

    let mark_highlight_symbol = {
      let s = self.mark_highlight_symbol.unwrap_or("\u{29bf}").trim_end();
      format!("{} ", s)
    };

    let unmark_highlight_symbol = {
      let s = self.unmark_highlight_symbol.unwrap_or("\u{29be}").trim_end();
      format!("{} ", s)
    };

    // Draw rows
    let default_style = Style::default();
    if y < table_area.bottom() {
      let remaining = (table_area.bottom() - y) as usize;

      // Make sure the table shows the selected item
      state.offset = selected.map_or(0, |s| {
        if s >= remaining + state.offset - 1 {
          s + 1 - remaining
        } else if s < state.offset {
          s
        } else {
          state.offset
        }
      });
      for (i, row) in self.rows.skip(state.offset).take(remaining).enumerate() {
        let (data, style, symbol) = match row {
          Row::Data(d) | Row::StyledData(d, _) if Some(i) == state.current_selection().map(|s| s - state.offset) => match state.mode {
            TableMode::MultipleSelection => {
              if state.marked.contains(&(i + state.offset)) {
                (d, highlight_style, mark_highlight_symbol.to_string())
              } else {
                (d, highlight_style, unmark_highlight_symbol.to_string())
              }
            }
            TableMode::SingleSelection => (d, highlight_style, highlight_symbol.to_string()),
          },
          Row::Data(d) => {
            if state.marked.contains(&(i + state.offset)) {
              (d, default_style, mark_symbol.to_string())
            } else {
              (d, default_style, blank_symbol.to_string())
            }
          }
          Row::StyledData(d, s) => {
            if state.marked.contains(&(i + state.offset)) {
              (d, s, mark_symbol.to_string())
            } else {
              (d, s, blank_symbol.to_string())
            }
          }
        };
        x = table_area.left();
        for (c, (w, elt)) in solved_widths.iter().zip(data).enumerate() {
          let s = if c == 0 {
            buf.set_stringn(
              x,
              y + i as u16,
              format!("{symbol:^width$}", symbol = "", width = area.width as usize),
              *w as usize,
              style,
            );
            if c == header_index {
              let symbol = match state.mode {
                TableMode::SingleSelection | TableMode::MultipleSelection => &symbol,
              };
              format!(
                "{symbol}{elt:>width$}",
                symbol = symbol,
                elt = elt,
                width = (*w as usize).saturating_sub(symbol.to_string().width())
              )
            } else {
              format!(
                "{symbol}{elt:<width$}",
                symbol = symbol,
                elt = elt,
                width = (*w as usize).saturating_sub(symbol.to_string().width())
              )
            }
          } else {
            buf.set_stringn(
              x - 1,
              y + i as u16,
              format!("{symbol:^width$}", symbol = "", width = area.width as usize),
              *w as usize + 1,
              style,
            );
            if c == header_index {
              format!("{elt:>width$}", elt = elt, width = *w as usize)
            } else {
              format!("{elt:<width$}", elt = elt, width = *w as usize)
            }
          };
          buf.set_stringn(x, y + i as u16, s, *w as usize, style);
          if let Some(cell_style) = self.cell_styles.get(&(i + state.offset, c)) {
            let text = elt.to_string();
            let prefix_width = if c == 0 { symbol.width() } else { 0 };
            let content_width = (*w as usize).saturating_sub(prefix_width);
            let offset = if c == header_index {
              // Match format!'s character-based right alignment exactly, including for
              // wide project names under a custom "ID" label. Styling must not move text.
              prefix_width + content_width.saturating_sub(text.chars().count())
            } else {
              prefix_width
            };
            let available = (*w as usize).saturating_sub(offset);
            if available > 0 {
              buf.set_stringn(x + offset as u16, y + i as u16, text, available, style.patch(*cell_style));
            }
          }
          x += *w + self.column_spacing;
        }
      }
    }
  }
}

impl<H, D, R> Widget for Table<'_, H, R>
where
  H: Iterator,
  H::Item: Display,
  D: Iterator,
  D::Item: Display,
  R: Iterator<Item = Row<D>>,
{
  fn render(self, area: Rect, buf: &mut Buffer) {
    let mut state = TaskwarriorTuiTableState::default();
    StatefulWidget::render(self, area, buf, &mut state);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use ratatui::style::{Color, Modifier};

  #[test]
  fn cell_styles_only_color_text_and_preserve_selection_and_padding() {
    let base = Style::default().fg(Color::White).bg(Color::DarkGray);
    let highlight = base.bg(Color::Blue).add_modifier(Modifier::BOLD);
    let project = Style::default().fg(Color::Green).add_modifier(Modifier::ITALIC);
    let rows = vec![
      Row::StyledData(["home", "other"].into_iter(), base),
      Row::StyledData(["work", "task"].into_iter(), base),
    ];
    let widths = [Constraint::Length(8), Constraint::Length(8)];
    let table = Table::new(["Project", "Description"].into_iter(), rows.into_iter())
      .widths(&widths)
      .highlight_symbol(">")
      .highlight_style(highlight)
      .cell_styles(HashMap::from([((1, 0), project)]));
    let mut buffer = Buffer::empty(Rect::new(0, 0, 22, 5));
    let mut state = TaskwarriorTuiTableState::default();
    state.select(Some(1));
    StatefulWidget::render(table, buffer.area, &mut buffer, &mut state);
    assert_eq!(buffer[(0, 3)].symbol(), ">");
    assert_eq!(buffer[(0, 3)].fg, Color::White);
    for x in 1..5 {
      assert_eq!(buffer[(x, 3)].fg, Color::Green);
      assert_eq!(buffer[(x, 3)].bg, Color::Blue);
      assert!(buffer[(x, 3)].modifier.contains(Modifier::BOLD | Modifier::ITALIC));
    }
    for x in 5..17 {
      assert_eq!(
        buffer[(x, 3)].fg,
        Color::White,
        "padding and description must retain the row style at {x}"
      );
    }
    assert_eq!(buffer[(1, 2)].fg, Color::White);
  }

  #[test]
  fn cell_styles_do_not_change_text_layout_for_truncated_unicode_or_custom_id_labels() {
    for label in ["ID", "Project"] {
      for text in ["work", "work.long.project", "猫猫", "e\u{301}👩‍💻"] {
        for width in 2..=8 {
          let render = |colored| {
            let widths = [Constraint::Length(width), Constraint::Length(5)];
            let styles = if colored {
              HashMap::from([((0, 0), Style::default().fg(Color::Green))])
            } else {
              HashMap::new()
            };
            let table = Table::new([label, "Task"].into_iter(), vec![Row::Data([text, "task"].into_iter())].into_iter())
              .widths(&widths)
              .highlight_symbol(">")
              .cell_styles(styles);
            let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 4));
            Widget::render(table, buffer.area, &mut buffer);
            buffer.content.iter().map(|cell| cell.symbol().to_string()).collect::<Vec<_>>()
          };
          assert_eq!(render(false), render(true), "{label}, {text}, width {width}");
        }
      }
    }
  }

  #[test]
  fn cell_styles_follow_absolute_rows_when_scrolling_and_keep_markers_uncolored() {
    let base = Style::default().fg(Color::White);
    let rows = (0..5).map(|_| Row::StyledData(["1", "猫猫"].into_iter(), base));
    let widths = [Constraint::Length(4), Constraint::Length(7)];
    let table = Table::new(["ID", "Project"].into_iter(), rows)
      .widths(&widths)
      .header_gap(0)
      .mark_highlight_symbol("@")
      .highlight_style(base.bg(Color::Blue))
      .cell_styles(HashMap::from([((4, 1), Style::default().fg(Color::Red))]));
    let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 4));
    let mut state = TaskwarriorTuiTableState::default();
    state.multiple_selection();
    state.select(Some(4));
    state.mark(Some(4));
    StatefulWidget::render(table, buffer.area, &mut buffer, &mut state);
    assert_eq!(state.offset, 2);
    assert_eq!(buffer[(0, 3)].symbol(), "@");
    assert_eq!(buffer[(0, 3)].fg, Color::White);
    for x in [5, 7] {
      assert_eq!(buffer[(x, 3)].symbol(), "猫");
      assert_eq!(buffer[(x, 3)].fg, Color::Red);
      assert_eq!(buffer[(x, 3)].bg, Color::Blue);
    }
    assert_eq!(buffer[(9, 3)].fg, Color::White);
    assert_eq!(buffer[(5, 1)].fg, Color::White);
  }
}
