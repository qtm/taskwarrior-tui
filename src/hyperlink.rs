//! Hyperlinks are found in source text, then carried through wrapping as column ranges.
//! Escape sequences are added only to rendered cells, never to text measured by widgets.

use std::{fmt, num::NonZeroU16, ops::Range, sync::Arc};

use linkify::{LinkFinder, LinkKind};
use ratatui::{
  buffer::{Buffer, CellDiffOption, CellWidth},
  layout::Rect,
  style::Style,
  text::{Line, Span},
  widgets::{Paragraph, Widget},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
struct Link {
  columns: Range<usize>,
  target: Arc<str>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct LinkedLine {
  pub text: Line<'static>,
  links: Vec<Link>,
}

impl LinkedLine {
  pub fn new(text: impl Into<Line<'static>>) -> Self {
    Self {
      text: text.into(),
      links: Vec::new(),
    }
  }

  pub fn width(&self) -> usize {
    self.text.width()
  }

  /// Join columns without losing the right column's link positions.
  pub fn append(&mut self, mut other: Self) {
    let offset = self.width();
    for link in &mut other.links {
      link.columns = link.columns.start + offset..link.columns.end + offset;
    }
    self.text.spans.extend(other.text.spans);
    self.links.extend(other.links);
  }

  fn link(&mut self, columns: Range<usize>, target: &Arc<str>) {
    if let Some(last) = self.links.last_mut()
      && last.columns.end == columns.start
      && last.target == *target
    {
      last.columns.end = columns.end;
    } else {
      self.links.push(Link {
        columns,
        target: target.clone(),
      });
    }
  }
}

impl fmt::Display for LinkedLine {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    self.text.fmt(f)
  }
}

/// A prewrapped paragraph. Slice the lines before rendering to scroll it.
pub(crate) struct LinkedParagraph(pub Vec<LinkedLine>);

impl Widget for LinkedParagraph {
  fn render(self, area: Rect, buf: &mut Buffer) {
    for (line, row) in self.0.into_iter().zip(area.rows()) {
      Paragraph::new(line.text).render(row, buf);
      for link in line.links {
        let mut x = usize::from(row.x) + link.columns.start;
        let end = (usize::from(row.x) + link.columns.end).min(usize::from(row.right()));
        while x < end {
          let Some(cell) = buf.cell_mut((x as u16, row.y)) else { break };
          let width = cell.cell_width();
          if let Some(width) = NonZeroU16::new(width)
            && x + usize::from(width.get()) <= end
            && !cell.symbol().chars().all(char::is_whitespace)
          {
            // Each cell is independently bracketed: Ratatui may redraw just one
            // changed glyph. A shared id groups fragments of the same URL.
            let symbol = format!("\x1b]8;id=taskwarrior-tui;{}\x1b\\{}\x1b]8;;\x1b\\", link.target, cell.symbol());
            cell.set_symbol(&symbol).set_diff_option(CellDiffOption::ForcedWidth(width));
          }
          // ForcedWidth (Ratatui >= 0.30.2) keeps OSC bytes out of buffer diff
          // width calculations, including wide glyphs and subsequent frames.
          x += usize::from(width.max(1));
        }
      }
    }
  }
}

/// Wrap at grapheme boundaries, preserving newlines and indentation. Prewrapping
/// makes scrolling and resize clamping use the exact number of displayed lines.
pub(crate) fn push_wrapped(lines: &mut Vec<LinkedLine>, text: &str, width: u16, indent: &str, style: Style) {
  push_wrapped_spans(lines, &[Span::styled(text, style)], width, indent, style);
}

pub(crate) fn push_wrapped_spans(lines: &mut Vec<LinkedLine>, spans: &[Span<'_>], width: u16, indent: &str, style: Style) {
  let indent = if indent.width() < usize::from(width) { indent } else { "" };
  push_wrapped_with_prefix(lines, spans, width, indent, vec![Span::styled(indent.to_string(), style)], style);
}

/// Like `push_wrapped_spans`, but the first row can have a styled checklist prefix.
/// The caller ensures that the prefix and continuation indent fit in the viewport.
pub(crate) fn push_wrapped_with_prefix(
  lines: &mut Vec<LinkedLine>,
  spans: &[Span<'_>],
  width: u16,
  indent: &str,
  prefix: Vec<Span<'static>>,
  style: Style,
) {
  let width = usize::from(width);
  if width == 0 {
    return;
  }
  // Detect before wrapping (and across style boundaries), not from screen rows.
  let source: String = spans.iter().map(|span| span.content.as_ref()).collect();
  let mut finder = LinkFinder::new();
  finder.kinds(&[LinkKind::Url]);
  let mut links = finder
    .links(&source)
    .filter(|link| {
      let url = link.as_str();
      // Only web URLs; never emit terminal controls from task data in OSC 8.
      (url.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("http://")) || url.get(..8).is_some_and(|s| s.eq_ignore_ascii_case("https://")))
        && !url.chars().any(char::is_control)
    })
    .map(|link| (link.start()..link.end(), Arc::<str>::from(link.as_str())))
    .peekable();
  let new_line = || LinkedLine::new(Line::styled(indent.to_string(), style));
  let mut line = LinkedLine::new(Line::from(prefix).style(style));
  let mut used = line.width();
  let mut offset = 0;
  for span in spans {
    for (index, grapheme) in span.content.grapheme_indices(true) {
      let index = offset + index;
      while links.peek().is_some_and(|(range, _)| range.end <= index) {
        links.next();
      }
      if grapheme == "\n" || grapheme == "\r\n" {
        lines.push(std::mem::replace(&mut line, new_line()));
        used = indent.width();
        continue;
      }
      // A terminal cannot display a wide grapheme in a one-column viewport.
      let grapheme = if grapheme.width() > width - indent.width() { "�" } else { grapheme };
      let size = grapheme.width();
      if used + size > width {
        lines.push(std::mem::replace(&mut line, new_line()));
        used = indent.width();
      }
      if let Some((range, target)) = links.peek()
        && range.contains(&index)
        && size > 0
      {
        line.link(used..used + size, target);
      }
      if let Some(last) = line.text.spans.last_mut()
        && last.style == span.style
      {
        last.content.to_mut().push_str(grapheme);
      } else {
        line.text.spans.push(Span::styled(grapheme.to_string(), span.style));
      }
      used += size;
    }
    offset += span.content.len();
  }
  lines.push(line);
}

#[cfg(test)]
pub(crate) fn cell_link(cell: &ratatui::buffer::Cell) -> Option<(&str, &str)> {
  let symbol = cell.symbol().strip_prefix("\x1b]8;id=taskwarrior-tui;")?;
  let (target, symbol) = symbol.split_once("\x1b\\")?;
  Some((target, symbol.strip_suffix("\x1b]8;;\x1b\\").expect("Each linked cell must close OSC 8")))
}

#[cfg(test)]
mod tests {
  use super::*;
  use ratatui::{
    backend::{Backend, CrosstermBackend},
    style::{Color, Modifier},
    widgets::Clear,
  };

  fn wrapped(text: &str, width: u16) -> Vec<LinkedLine> {
    let mut lines = Vec::new();
    push_wrapped(&mut lines, text, width, "  ", Style::default());
    lines
  }

  fn render(lines: Vec<LinkedLine>, area: Rect) -> Buffer {
    let mut buf = Buffer::empty(area);
    LinkedParagraph(lines).render(area, &mut buf);
    buf
  }

  #[test]
  fn full_targets_survive_wrapping_scrolling_and_unicode_at_every_width() {
    let url = "https://example.com/猫/e\u{301}/very/long/path?q=one&other=two#section";
    let source = format!("猫 e\u{301} {url} tail");
    for width in 1..=80 {
      let lines = wrapped(&source, width);
      assert!(lines.iter().all(|line| line.width() <= usize::from(width)));
      assert!(lines.iter().all(|line| !line.to_string().contains('\x1b')));
      // Nonzero origins exercise right-hand/bottom panes as well as fullscreen.
      let area = Rect::new(3, 5, width, lines.len() as u16);
      let plain = render(lines.iter().map(|line| LinkedLine::new(line.text.clone())).collect(), area);
      let linked = render(lines.clone(), area);
      let mut linked_count = 0;
      for (cell, plain_cell) in linked.content.iter().zip(&plain.content) {
        if let Some((target, glyph)) = cell_link(cell) {
          linked_count += 1;
          assert_eq!(target, url);
          assert_eq!(glyph, plain_cell.symbol());
          assert_eq!(cell.cell_width(), plain_cell.cell_width());
          assert_eq!(cell.style(), plain_cell.style());
          assert!(!glyph.chars().any(char::is_whitespace));
        } else {
          assert_eq!(cell, plain_cell);
        }
      }
      assert!(linked_count > 0);
      if lines.len() > 2 {
        let scrolled = render(lines.into_iter().skip(1).collect(), area);
        assert!(scrolled.content.iter().any(|cell| cell_link(cell).is_some()));
        for (target, _) in scrolled.content.iter().filter_map(cell_link) {
          assert_eq!(target, url);
        }
      }
    }
  }

  #[test]
  fn detection_handles_multiple_links_markdown_punctuation_and_style_boundaries() {
    let first = "https://example.com/wiki/Link_(title)";
    let second = "http://example.org/a?x=1&y=2#fragment";
    let source = format!("[docs]({first}), <{second}>.");
    let split = source.find("example.com").unwrap() + 3;
    let spans = [
      Span::styled(&source[..split], Style::default().fg(Color::Cyan)),
      Span::styled(&source[split..], Style::default().add_modifier(Modifier::BOLD)),
    ];
    let mut lines = Vec::new();
    push_wrapped_spans(&mut lines, &spans, 12, "", Style::default());
    let buf = render(lines, Rect::new(0, 0, 12, 20));
    for url in [first, second] {
      let text: String = buf
        .content
        .iter()
        .filter_map(cell_link)
        .filter(|(target, _)| *target == url)
        .map(|(_, glyph)| glyph)
        .collect();
      assert_eq!(text, url);
    }
    assert!(
      buf
        .content
        .iter()
        .filter_map(cell_link)
        .all(|(target, _)| target == first || target == second)
    );
    let cells: Vec<_> = buf.content.iter().filter(|cell| cell_link(cell).is_some()).collect();
    assert!(cells.iter().any(|cell| cell.fg == Color::Cyan));
    assert!(cells.iter().any(|cell| cell.modifier.contains(Modifier::BOLD)));
  }

  #[test]
  fn literal_newlines_are_not_joined_and_targets_cannot_inject_terminal_controls() {
    let source = "https://example.com/first\ncontinuation javascript:alert(1) file:///tmp/a";
    let buf = render(wrapped(source, 20), Rect::new(0, 0, 20, 10));
    assert!(
      buf
        .content
        .iter()
        .filter_map(cell_link)
        .all(|(target, _)| target == "https://example.com/first")
    );
    for control in ['\x07', '\x1b', '\u{009c}', '\r', '\t'] {
      let source = format!("https://example.com/before{control}after");
      let buf = render(wrapped(&source, 20), Rect::new(0, 0, 20, 10));
      for (target, _) in buf.content.iter().filter_map(cell_link) {
        assert!(!target.chars().any(char::is_control));
      }
    }
  }

  #[test]
  fn incremental_output_updates_hidden_target_changes_and_removes_old_links() {
    let area = Rect::new(0, 0, 18, 1);
    let first_url = "https://example.com/first";
    let second_url = "https://example.com/second";
    let first = render(wrapped(first_url, area.width), area);
    let second = render(wrapped(second_url, area.width), area);
    // The visible glyphs are identical; only the offscreen destination changed.
    assert_eq!(
      first.content.iter().filter_map(cell_link).map(|(_, glyph)| glyph).collect::<String>(),
      second.content.iter().filter_map(cell_link).map(|(_, glyph)| glyph).collect::<String>()
    );
    let diff = first.diff(&second);
    assert_eq!(diff.len(), 16);
    let mut output = Vec::new();
    let mut backend = CrosstermBackend::new(&mut output);
    backend.draw(diff.into_iter()).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.matches(&format!("\x1b]8;id=taskwarrior-tui;{second_url}\x1b\\")).count(), 16);
    assert_eq!(output.matches("\x1b]8;;\x1b\\").count(), 16);
    assert!(!output.contains(first_url));
    assert!(second.diff(&second).is_empty());

    // Same glyphs but no link must also repaint, so task changes/pane closing
    // cannot leave stale destinations in the terminal's screen cells.
    let plain_text = second
      .content
      .iter()
      .map(|cell| cell_link(cell).map_or(cell.symbol(), |(_, glyph)| glyph))
      .collect::<String>();
    let plain = render(vec![LinkedLine::new(plain_text)], area);
    assert_eq!(second.diff(&plain).len(), 16);
    assert!(plain.content.iter().all(|cell| cell_link(cell).is_none()));
  }

  #[test]
  fn wide_link_cells_do_not_skip_neighbors_and_popups_clear_link_metadata() {
    let area = Rect::new(0, 0, 40, 3);
    let linked = render(wrapped("https://example.com/猫z end", area.width), area);
    let diff = Buffer::empty(area).diff(&linked);
    assert!(diff.iter().any(|(_, _, cell)| cell_link(cell).is_some_and(|(_, glyph)| glyph == "猫")));
    assert!(diff.iter().any(|(_, _, cell)| cell_link(cell).is_some_and(|(_, glyph)| glyph == "z")));
    assert!(diff.iter().any(|(_, _, cell)| cell.symbol() == "e"));
    let mut popup = linked.clone();
    Clear.render(area, &mut popup);
    Paragraph::new("popup").render(area, &mut popup);
    assert!(
      popup
        .content
        .iter()
        .all(|cell| cell_link(cell).is_none() && cell.diff_option == CellDiffOption::None)
    );
    // Every old linked cell must be erased, even after a two-column character.
    let erased = linked.diff(&popup);
    for (x, y, _) in diff.iter().filter(|(_, _, cell)| cell_link(cell).is_some()) {
      assert!(erased.iter().any(|(ex, ey, _)| ex == x && ey == y));
    }
  }
}
