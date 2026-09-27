# Task-attached Markdown checklists — implementation plan

## Goal

Attach one or more named checklists to any task, render them using Markdown task-list syntax, preserve nested items, and import a complete checklist from the clipboard without retyping it. Keep the task report visible in the existing half-screen layout.

## Required behavior

- Render unchecked items as `- [ ] text` and checked items as `- [x] text`.
- Render children with two additional spaces per nesting level. Wrap long text under its text, not its checkbox. Preserve Cyrillic, other Unicode, and grapheme boundaries.
- Support multiline item descriptions: the first text line is the title, remaining lines are its description. Description lines are displayed indented below the title and are part of the same item for selection, progress, and subtree operations.
- Keep explicit checked state for every item. Checking a parent does **not** check its children; checking all children does **not** check the parent.
- Progress counts every item, including parents and children.
- Support multiple named lists per task, with add/rename/delete operations.
- Support adding siblings/children, editing, checking, moving subtrees, indenting/outdenting, and confirmed subtree deletion.
- Completing a checklist does not complete its Taskwarrior task.
- Checklist edits do not create annotations or extra tasks.

## Storage and compatibility

- Use the Taskwarrior string UDA `tuichecklist`, containing versioned JSON.
- The document contains ordered lists with stable UUIDs. Each list contains ordered items with stable UUIDs, text, checked state, and depth (a validated preorder representation of the tree).
- Keep item IDs scoped to the owning task; native duplication can safely copy the complete document, including checked states.
- No sidecar database and no direct edits to Taskwarrior storage files.
- Declare `serde_json` and explicit UUID features as dependencies.
- Read through existing exported `Task::uda()` values. Save only `tuichecklist` for one captured task UUID using a dedicated CLI adapter, never the multi-selected task modification command.
- Supply the UDA type/label as command-local configuration; do not silently edit the user's `.taskrc`. Document persistent UDA definitions for other Taskwarrior clients.
- Preserve hooks and native undo. Disable recurrence propagation for checklist edits so siblings/templates are not modified accidentally.
- Re-read the checklist field before writing and reject stale edits; preserve drafts on failure. This is best-effort conflict detection, not atomic compare-and-swap or cross-device merging.
- Missing data means an empty document. Malformed, oversized, unsupported-version, or invalid-tree data is read-only with a visible error, never silently replaced.
- Bound document size, number of items, and nesting depth; reject excess rather than truncate.
- Native `task info` may display the raw JSON field. The TUI checklist pane provides the formatted view; the report's `tuichecklist` column will show compact progress.

## Pane and keyboard interaction

- `C` (configurable): toggle checklists for the highlighted task.
- `I` (configurable): read a Markdown checklist from the clipboard and open an import preview for the highlighted task.
- Reuse the existing bottom/right placement, 50/50 split, and transpose behavior.
- Checklist visibility temporarily replaces the current secondary pane. Closing restores it. Explicit `n` / `z` switches to annotations / task details.
- Centralize secondary-pane selection using an enum; preserve existing task-details preference and annotation return-view behavior.
- The task list initially retains focus. `Tab` switches between task and checklist focus; show focus visibly.
- With checklist focus: `j/k` or arrows select items, Space toggles, `a` adds a sibling, `o` adds a child, `e` edits, `x` asks to delete the selected subtree, `Alt-j/k` reorders sibling subtrees, `>` / `<` indents/outdents, `[` / `]` selects a list, `A/E/X` adds/renames/deletes lists.
- Existing configurable task navigation/add/edit/delete bindings should also work for the corresponding checklist actions where practical.
- In focused checklists, unknown keys must not fall through to destructive parent-task actions. Global quit/help/refresh/undo/pane switching remain available when not editing.
- In item add/edit prompts, Enter inserts a newline and Shift+Enter or Ctrl-s saves. Import, list-name, and deletion prompts retain Enter confirmation. Esc cancels and toggle keys become ordinary text. A prompt captures its task UUID, list/item ID, and original document so refresh/reordering cannot redirect the write. Request enhanced keyboard reporting on supported terminals, restore it on suspend/exit, and provide Ctrl-s for legacy terminals.
- Native undo remains global Taskwarrior undo, not a separate per-checklist history.

## Clipboard import

- macOS: `pbpaste`.
- Linux Wayland: `wl-paste`; X11: `xclip` with the clipboard selection or `xsel --clipboard --output`.
- Use direct process arguments, bounded output, timeout, and useful failure messages. Do not log clipboard contents or invoke a shell.
- If no clipboard reader is available (including SSH), keep an import prompt open and accept terminal bracketed paste, preserving newlines and indentation. Provide a clear-buffer action.
- Preview before saving; Enter attaches the parsed checklist as a **new named list**, preserving all existing lists; Esc changes nothing.
- Accept `-`, `*`, and `+` task-list bullets, `[ ]`, `[x]`, and `[X]`, LF/CRLF, blank lines, and an optional leading Markdown heading for the list name.
- Recognize nesting from consistent checkbox indentation levels (including common two- and four-space Markdown indentation). Non-checkbox lines after an item become its description until the next checkbox; both indented and lazy continuations are accepted. Reject ambiguous indentation tabs, inconsistent checkbox dedents, malformed checkbox lines, empty titles, and prose before the first item with line numbers. Do not silently discard content.
- Normalize display/export formatting to two spaces per depth and lowercase `x` without changing item text or checked state.

## Implementation stages

- [x] 1. Model: versioned data, validation, Markdown parser/serializer, tree operations, unit tests.
- [x] 2. Persistence and clipboard adapters: exact UUID writes, conflict checks, native undo compatibility, safe clipboard reads, integration tests with disposable data.
- [x] 3. Pane: Markdown rendering, Unicode wrapping, scrolling, progress, list/item selection, explicit focus and layout restoration.
- [x] 4. Editing/import: prompts, paste routing, confirmation, refresh/draft preservation, safe keyboard dispatch, report progress.
- [x] 5. Documentation: settings, clipboard prerequisites, keys, semantics, failure recovery, limitations; update built-in help.
- [x] 6. Validation: formatting, full test suite, Clippy with only existing unrelated allowances, build, live PTY tests, regression tests for annotations/project colors. Review generated artifacts and preserve pre-existing user changes.

## Acceptance tests

1. Import the supplied Russian example: preserve all 13 items, four children, order, nesting, and checked states (five checked).
2. Render exact Markdown markers and indentation, with narrow-pane/Unicode wrapping.
3. Check a parent without changing child states; save/restart and native undo preserve the entire document.
4. Import a second list without overwriting the first; cancel import and invalid Markdown leave storage unchanged.
5. Exercise add/edit/remove/reorder/indent/outdent on nested subtrees, including non-last children.
6. Use clipboard helper success/failure/timeout/oversize paths and bracketed-paste fallback.
7. Keep editing targeted to one UUID even with multiple tasks marked, changing filters, sorting, refresh, or external edits.
8. Reject corrupt/unknown-version data and stale edits without losing data or the draft.
9. Verify task details, annotations, project-only colors, configured keys, bottom/right layout, and previous-view restoration.
10. Verify duplicate, delete/undo, and recurring-instance edits with a disposable Taskwarrior 3.x database. Never use personal task data for tests.

## Explicit limitations / future work

- Single-field JSON synchronization has whole-field conflict semantics; cross-device automatic merging is not implemented. Normal UDA export/import compatibility is tested; a real sync-server test requires an isolated server and credentials.
- No reusable templates, checklist-to-task conversion, auto-completion of parent tasks, or incomplete-checklist completion warnings in this release.
- Native duplication preserves checked states. Recurring instances inherit their template's checklist; instance edits do not propagate back to the template.
- No full Markdown document interpreter: import supports task lists, multiline descriptions, and an optional title. Formatting/code fences in descriptions have no special meaning; prose before the first item is rejected.

## Progress log

- Planning: inspected existing panes, input routing, UDA imports, task modification, undo, and recurrence behavior. Verified JSON UDA round-trips and native undo/duplication in a disposable Taskwarrior 3.3.0 database. No personal task data changed.
- Iteration 1: implemented the versioned model, strict Markdown importer, nesting/subtree operations, and bounded clipboard reader. Added the exact Russian example as `tests/fixtures/checklist-ru.md`; verified 13 items, four children, and five checked items.
- Iteration 2: integrated the half-screen pane, focus-safe editing, named lists, clipboard preview/paste fallback, captured task/item identities, stale-write rejection, and compact report progress. Reused existing pane preferences through a central `SecondaryPane` arbitration enum rather than replacing the stored task-details preference.
- Iteration 3: added disposable-backend tests for persistence, native undo, duplicate, delete/undo, export/import into another database, corrupt data, command failure, and recurrence isolation. Added UI tests for layout, project-only colors, focus, draft retention, and configured keys.
- Iteration 4: added `scripts/test-checklists.py`, a repeatable live PTY test with a fake clipboard and isolated tasks. It exercises import, cancellation, malformed Markdown, nested edits/reordering/deletion, named lists, multi-selection safety, stale edits, terminal paste, pane restoration, transpose, and clean exit. Adjusted the test to await asynchronous redraw after saving rather than relying on a fixed sleep.
- Final validation: all **76 Rust tests passed** with isolated Taskwarrior 3.3.0 fixtures; formatting, debug build, diff whitespace checks, and sidebar JavaScript syntax checks passed. Clippy passed with only the three pre-existing lint categories allowed (`useless_borrows_in_formatting`, `let_and_return`, `needless_borrows_for_generic_args`). Both the new checklist PTY test and the existing annotations/project-title-color PTY regression test passed. Updated the help snapshot to reflect the longer help page.
- Highlighting follow-up: fixed invisible selection when the optional report-selection style is empty. Focused checklist items now have a reverse-video fallback plus configured selection modifiers, full-width highlighting on every wrapped line, and no implicit completed-item dimming. Added rendered-buffer tests covering selection movement, focus changes, custom colors/modifiers, and scrolled continuation lines. All **78 Rust tests**, Clippy (same existing allowances), the debug build, and the live checklist PTY test passed.
- Multiline follow-up: added title/description lines within the existing item text field, Markdown continuation import/rendering, and the exact example in `tests/fixtures/checklist-multiline.md`. Enter inserts a newline in item editors; Shift+Enter or Ctrl-s saves. The growing multiline editor supports line navigation, Unicode cursor positioning, and scroll-to-cursor. Added enhanced-keyboard request/restore handling and release-event filtering. All **84 Rust tests**, formatting, Clippy (same existing allowances), debug build, and both live PTY regression suites passed. Live tests cover actual Shift+Enter CSI-u input, Ctrl-s fallback, descriptions on nested items, and restoration across external-command suspend/resume and exit. The PTY fixture now answers cursor-position queries during resume.
- Task-annotations follow-up: the checklist pane now appends only its owning task's annotations below the displayed checklist, newest first, using the all-task timeline's shared formatter (project-only colors, local dates, grouping and Unicode wrapping). The existing pane scroll controls cover both sections without selecting annotations or changing progress. Annotation-only refreshes do not require a checklist UDA change; task switching clears the previous task's notes. Empty/malformed checklists still allow reading annotations, while import previews remain draft-only. Added rendered-buffer coverage for styles/order, refresh, task/draft identity, empty/error states, long histories, scrolling and resizing; expanded the live checklist test for actual exports, navigation, refresh and annotation preservation. All **88 Rust tests**, formatting, Clippy (same existing allowances), debug build, checklist/editor-resume PTY tests, and annotations/project-color PTY regression passed.
- Checklist presentation follow-up: checked items now use green and unchecked items red, including descriptions, nested items, wrapped lines and import previews. Status colors replace implicit checked-item dimming; the existing selection overlay and explicit selection colors/modifiers are retained. Checklist annotations now use aligned date/text columns, with text continuations in the right column and date wrapping in narrow panes; the standalone annotation timeline remains unchanged. Added tests for state colors/toggles, preview colors, wrapping, Unicode, narrow layouts and timeline isolation. The PTY helper now tracks ANSI foregrounds and verifies actual colors, independent child states, no color leakage into annotations, and column alignment in both split orientations. All **91 Rust tests**, formatting, Clippy (same existing allowances), debug build and all three live PTY suites passed.
- Marker-only color refinement: restricted green/red status coloring to the three characters in `[x]` / `[ ]`. Bullets, titles, descriptions and wrapped continuation text keep their normal foreground; existing selection highlighting and custom selection styles are unchanged. Applied the same rule to import previews and narrow layouts where the marker itself wraps. All **92 Rust tests**, formatting, Clippy (same existing allowances), debug build and live checklist PTY checks passed, including explicit assertions that title/description text is not status-colored.
- Platform coverage: runtime tests ran on Linux. macOS uses the standard `pbpaste` path but was not runtime-tested here. No real remote sync server was used; cross-database export/import was tested and whole-field synchronization limitations are documented.

## Delivered files

- `src/checklist.rs`: model, validation, Markdown parsing/serialization, subtree operations, Taskwarrior persistence.
- `src/clipboard.rs`: read-only clipboard adapters, bounded output and timeout.
- `src/pane/checklist.rs`: rendering, focus, prompts, import preview, editing controller.
- `src/checklist_tests.rs`, `tests/fixtures/checklist-ru.md`, `tests/fixtures/checklist-multiline.md`, `scripts/test-checklists.py`: backend, fixtures, and live integration coverage.
- `src/app.rs`, `src/action.rs`, `src/event.rs`, `src/pane/mod.rs`, `src/keyconfig.rs`, `src/task_report.rs`, `src/main.rs`, `src/help.*`: integration, keyboard handling, and built-in help. Shared wrapping helpers remain in `src/pane/annotations.rs`.
- `docs/src/content/docs/checklists.md`: user guide; configuration, keybindings, colors, and sidebar pages link to it.

The implementation is complete for the required scope above. Explicit future-work items remain intentionally out of scope.
