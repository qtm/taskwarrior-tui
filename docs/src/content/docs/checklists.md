---
title: Markdown Checklists
description: Attach nested checklists to tasks, edit them in a split pane, and import Markdown from the clipboard.
---

## Quick start

1. Highlight a task and press **`I`** to read a Markdown checklist from the clipboard.
2. Review the preview, then press **Enter** to attach it as a new checklist. **Esc** cancels without saving anything.
3. Press **`C`** to show/hide the checklist pane later. It shares the screen equally with the task report.
4. Press **Tab** to switch focus between tasks and checklists. With checklist focus, use **j/k** to select an item and **Space** to check/uncheck it.

For example:

```markdown
# Миграция репозитория
- [x] Применить terraform c правами
  - [x] Добавить tech юзера для pull
- [ ] Секреты в vault
- [ ] Вмержить PR в мигрировавший репо
  - [ ] Проверить сборку dockerfile за proxy
- [ ] Найти в registry пакеты
  - [ ] Выдать права на helm chart из iyb-infra для репозитория
```

The pane displays literal Markdown task-list markers, nested indentation, and progress. It wraps long text underneath the item's text. Cyrillic, emoji, and other Unicode are supported.

**Checkboxes are independent:** checking a parent does not check its children, and finishing all children does not check the parent. Progress counts every item, including parent items. Finishing a checklist does not complete its task.

## Task annotations

Below the displayed checklist, the pane shows **only the highlighted task's annotations**, newest first. They use the same project/task headings, local timestamps, colors, indentation, and Unicode wrapping as the all-task annotation view (`n`). Annotations also remain available when the task has no checklists.

Use **Ctrl-e/Ctrl-y** to scroll down/up or **Ctrl-d/Ctrl-u** for a page, with either task or checklist focus. Scrolling does not change the selected checkbox; annotations are read-only here and never count toward checklist progress. Switching tasks updates both sections; `r` refreshes them. Import previews show only the draft, without annotations.

## Clipboard support

- **macOS:** `pbpaste` (included with macOS).
- **Wayland:** `wl-paste`, provided by `wl-clipboard`.
- **X11:** `xclip` or `xsel`, using the clipboard selection rather than the primary selection.

On Debian/Ubuntu, install the appropriate helper, for example `sudo apt install wl-clipboard` or `sudo apt install xclip`.

The application only reads the clipboard; it never changes it. Reader commands have a timeout and an output-size limit. Clipboard contents are not added to command history or logged.

### Terminal paste / SSH fallback

If a clipboard reader is unavailable or fails, the import prompt stays open. Use your terminal's normal paste shortcut (often Ctrl-Shift-V or Cmd-V), then press Enter. Bracketed paste preserves the Markdown newlines and indentation.

In the import prompt:

- **Ctrl-u:** clear the input before pasting a replacement.
- **Ctrl-n:** insert a newline when entering Markdown manually.
- **PageUp/PageDown:** scroll the preview.
- **Enter:** save as a new list.
- **Esc:** discard the draft.

An empty or invalid clipboard never modifies the task. Errors include the offending line number where applicable. On terminals without bracketed paste, multiline paste may be interpreted as keystrokes; use the clipboard helper instead.

## Supported Markdown

- Unordered task-list bullets: `-`, `*`, or `+`.
- Checkbox states: `[ ]`, `[x]`, or `[X]`.
- Nested items with consistent space indentation, including two- and four-space indentation.
- Multiline descriptions beneath items, including blank lines between description paragraphs, and LF/CRLF line endings.
- An optional leading Markdown heading for the list name. Without one, the name is `Checklist`.

Rendering normalizes nesting to two spaces per level and checked markers to lowercase `x`. Non-checkbox lines after an item become its description until the next checkbox, whether indented or unindented. Checkbox indentation alone determines nesting; a description after a nested checkbox belongs to that nested item. Blank separator lines between items are ignored, while blank lines within a description are preserved.

Import rejects indentation tabs, inconsistent checkbox dedents, empty titles, malformed checkboxes, and prose before the first item rather than silently discarding content. This is a task-list importer, not a full Markdown interpreter; code fences and formatting inside descriptions have no special meaning.

Import **always adds a new list** to the highlighted task. It never overwrites an existing list or attaches to every marked task. Repeating an import intentionally creates another independent list.

## Multiline item descriptions

Both import and the checklist pane support descriptions like this:

```markdown
- [ ] task
  task description
  on multi line
  - [ ] subtask
    another description
- [ ] first level task without description
- [ ] first level task with description
  description
- [ ] first level task without description
```

Descriptions are part of their item, not separate checkboxes. They move, delete, and highlight together with the item; they do not increase the progress count. Existing single-line checklists need no migration.

When **adding or editing an item** (`a`, `o`, or `e`):

- The first line is the item title; subsequent lines are its description.
- **Enter** inserts a newline without saving.
- **Shift+Enter** creates/saves the item.
- **Ctrl-s** also saves, as a fallback for terminals that cannot distinguish Shift+Enter from Enter.
- **Esc** cancels without changing the task.
- **Up/Down** moves the cursor between lines; Home/End moves to the start/end of the current line. The editor grows as you add lines and scrolls to keep the cursor visible.

The TUI requests enhanced keyboard reporting from compatible terminals and restores the previous mode on exit or when launching an external editor. In older terminals or some terminal multiplexer configurations, Shift+Enter still arrives as ordinary Enter; use Ctrl-s in that case.

**Import previews, checklist names, and deletion confirmations still use ordinary Enter to confirm.** To create a nested checkbox rather than a description line, finish the current item and use `o`, or import a complete Markdown checklist with `I`.

## Editing keys

These keys apply when the **checklist pane has focus**:

| Key | Action |
| --- | --- |
| j/k or arrows | Select an item |
| J/K or PageDown/PageUp | Move selection by approximately one page |
| g/G | First/last item |
| Space | Check/uncheck the selected item only |
| a | Add a sibling after the selected subtree; create a default list if needed |
| o | Add a child as the last child of the selected item |
| e | Edit item text |
| x | Confirm deletion of the selected item and all its children |
| > / < | Indent / outdent the selected subtree |
| Alt-j / Alt-k | Move a subtree down / up among siblings |
| [ / ] | Previous / next named checklist |
| A / E / X | Add / rename / confirm deletion of an entire list |
| I | Import another checklist from the clipboard |
| Ctrl-e / Ctrl-y | Scroll items and task annotations down / up without changing selection |
| Ctrl-d / Ctrl-u | Scroll items and task annotations by a page without changing selection |
| Tab | Return focus to the task list |
| C / Esc | Close the pane and restore the previous pane/view |

The configured task add/edit/delete and navigation keys also apply to the corresponding checklist operations. Item editors use Enter for a new line and Shift+Enter (or Ctrl-s) to save; other prompts use Enter to confirm. Esc cancels. Text editing uses the normal cursor movement and deletion keys. Pane shortcuts are ordinary text while editing.

Outdenting moves a subtree **after its former parent's entire subtree**, preserving the remaining children's parentage. Indenting requires a preceding sibling. These operations never implicitly change checkboxes.

With task focus, normal task controls remain active and the checklist follows the highlighted task. With checklist focus, unrecognized task-action keys are ignored to prevent accidental parent-task changes. Quit, help, refresh, global undo, transpose, and explicit pane switching remain available outside editing prompts.

`u` uses **global Taskwarrior undo**, not checklist-only history. If another task operation happened most recently, undo affects that operation.

## Layout and colors

The pane uses the same bottom/right location and 50/50 split as task details and annotations. `\` transposes the split. `n` switches to annotations; `z` switches to task details. Toggling checklists off restores the underlying pane and task-details preference.

Project names reuse `uda.taskwarrior-tui.style.project-title.<project>` colors without coloring checklist text. Checked items are dimmed unless selected. When the checklist has focus (**Tab**), the current item is highlighted across the full pane width, including wrapped continuation lines. By default the highlight is bold reverse-video; an explicit `uda.taskwarrior-tui.style.report.selection` replaces the default style, and `uda.taskwarrior-tui.selection.*` modifiers also apply. Returning focus to the task list removes the active checklist highlight.

## Configuration and reports

Optional key settings in `~/.taskrc`:

```ini
uda.taskwarrior-tui.keyconfig.checklist=C
uda.taskwarrior-tui.keyconfig.import-checklist=I
```

Checklists are stored as versioned JSON in the task's **`tuichecklist` string UDA**. The TUI supplies the UDA definitions for its own checklist commands and does not silently edit `.taskrc`.

For other Taskwarrior clients, or reports that include a checklist column, add:

```ini
uda.tuichecklist.type=string
uda.tuichecklist.label=Checklist
```

Add `tuichecklist` to a report's columns to display total progress in the TUI:

```ini
report.next.columns=id,project,description,tuichecklist
report.next.labels=ID,Project,Description,Checklist
```

This example replaces the report columns; adapt it if you want to retain other columns. The TUI renders the checklist column as `5/13`. Tasks without a checklist have an empty cell. Native Taskwarrior reports and `task info` may show the raw JSON instead.

## Data safety and limitations

- Checklist changes modify only this task's checklist UDA, using its UUID—not its changing numeric ID or all marked tasks. Existing descriptions, projects, annotations, and unrelated fields are preserved unless your own Taskwarrior hooks modify them.
- Changes participate in native Taskwarrior undo and travel with normal task export/backup/synchronization. No separate local checklist database is used. Other clients need checklist-aware UI support to edit the structured data conveniently.
- Native duplication copies lists **and checked states**. IDs are scoped to the owning task, so copied lists are independent.
- Recurring instances inherit their template's checklist. Checklist edits disable recurrence propagation: modifying one instance does not change siblings or the template; modifying the template does not rewrite already-created instances.
- Before saving, the TUI re-reads the stored checklist and rejects a stale edit. If it reports an external change, your draft stays open. Cancel, refresh, and retry. On backend failure, retry the save shortcut (Shift+Enter/Ctrl-s for item editing; Enter for other prompts) after resolving the failure, or cancel safely.
- This check is **not atomic compare-and-swap**. Simultaneous edits or offline synchronization from different devices can still conflict because the complete document is stored as one field. There is no automatic item-level merge.
- Invalid JSON, unsupported versions, invalid trees, and non-string data are shown as errors and not silently replaced. Repair the UDA externally or restore a backup, then refresh.
- Limits: 64 KiB for the serialized document and import input, 2,000 items total, 2,000 lists, and nesting depth 16 (root depth zero). The byte limit may be reached before the item limit. Oversized data is rejected, not truncated.
- No automatic parent-task completion, nested-checkbox cascading, reusable templates, or incomplete-checklist completion warnings are added.
