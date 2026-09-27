---
title: Default Keybindings
description: Reference the default keyboard controls for reports, prompts, menus, and the calendar view.
---

:::note
If any bindings are changed in `.taskrc`, the built-in help menu will show them instead of the defaults. See [key configuration](./configuration/keys.md) for details on custom keybindings.
:::

Keybindings:

    Esc:                                 - Exit current action

    ]: Next view                         - Go to next view

    [: Previous view                     - Go to previous view

    n: annotations                       - Toggle half-screen annotations pane

    C: checklists                        - Toggle half-screen Markdown checklists pane

    I: import checklist                  - Preview Markdown from clipboard for selected task

The checklist pane also shows the highlighted task's annotations below its items, newest first. Use `Ctrl-e/y` or `Ctrl-d/u` to scroll them.

**Shift+E** opens the highlighted task's associated Markdown note in your terminal editor, creating it if needed. Set `uda.taskwarrior-tui.notes-directory` first; see [task notes](./configuration/advanced.md#task-notes). This acts on the highlighted task, not all marked tasks. When the checklist pane has focus, `E` still renames the checklist; press Tab to return focus to tasks first.

See [checklists](./checklists.md) for checklist focus, nested-item editing, and clipboard import controls. `Tab` switches between task and checklist focus; task controls remain active while the task list has focus. While adding or editing a checklist item, **Enter inserts a newline** and **Shift+Enter (or Ctrl-s) saves**. Other prompts still use Enter to confirm.

Keybindings for task report:

    /: task {string}                     - Filter task report

    a: task add {string}                 - Add new task

    d: task {selected} done              - Mark task as done

    e: task {selected} edit              - Open selected task in editor
    E: task note                         - Open/create highlighted task's Markdown note

    y: task {selected} duplicate         - Duplicate tasks

    j: {selected+=1}                     - Move down in task report

    k: {selected-=1}                     - Move up in task report

    J: {selected+=pageheight}            - Move page down in task report

    K: {selected-=pageheight}            - Move page up in task report

    g: {selected=first}                  - Go to top

    G: {selected=last}                   - Go to bottom

    l: task log {string}                 - Log new task

    m: task {selected} modify {string}   - Modify selected task

    q: exit                              - Quit

    s: task {selected} start/stop        - Toggle start and stop

    t: task {selected} +{tag}/-{tag}     - Toggle {uda.taskwarrior-tui.quick-tag.name} (default: `next`)

    u: task undo                         - Undo

    v: {toggle mark on selected}         - Toggle mark on selected

    V: {toggle marks on all tasks}       - Toggle marks on all tasks in current filter report

    x: task {selected} delete            - Delete

    z: toggle task info                  - Toggle task info view

    A: task {selected} annotate {string} - Annotate current task

    Ctrl-e: scroll down side pane        - Scroll task details / annotations down one line

    Ctrl-y: scroll up side pane          - Scroll task details / annotations up one line

    !: {string}                          - Custom shell command

    1-9: {string}                        - Run user defined shortcuts

    :: {task id}                         - Jump to task id

    c: context switcher menu             - Open context switcher menu

    R: report switcher menu              - Open report switcher menu

    ?: help                              - Help menu

Keybindings for filter / command prompt:

    Ctrl + f | Right: move forward       - Move forward one character

    Ctrl + b | Left: move backward       - Move backward one character

    Ctrl + h | Backspace: backspace      - Delete one character back

    Ctrl + d | Delete: delete            - Delete one character forward

    Ctrl + a | Home: home                - Go to the beginning of line

    Ctrl + e | End: end                  - Go to the end of line

    Ctrl + k: delete to end              - Delete to the end of line

    Ctrl + u: delete to beginning        - Delete to the beginning of line

    Ctrl + w: delete previous word       - Delete previous word

    Alt + d: delete next word            - Delete next word

    Alt + b: move to previous word       - Move to previous word

    Alt + f: move to next word           - Move to next word

    Alt + t: transpose words             - Transpose words

    Up: scroll history                   - Go backward in history matching from beginning of line to cursor

    Down: scroll history                 - Go forward in history matching from beginning of line to cursor

    TAB | Ctrl + n: tab complete         - Open tab completion and selection first element OR cycle to next element

    BACKTAB | Ctrl + p: tab complete     - Cycle to previous element

Keybindings for context switcher:

    j: {selected+=1}                     - Move forward a context

    k: {selected-=1}                     - Move back a context

    Enter: task context {selected}       - Select highlighted context

Keybindings for report switcher:

    j: {selected+=1}                     - Move forward a report

    k: {selected-=1}                     - Move back a report

    Enter: task report {selected}        - Select highlighted report

## Annotations pane

Press `n` to show annotations alongside the task list in a **50/50 split**, using the same bottom/right placement as the `z` task-info pane. The task list remains visible and interactive. Annotations temporarily replace task details rather than adding a third pane. Press `n` again (or `Esc` from the task report) to restore the previous layout, including whether task details were shown.

The pane follows `uda.taskwarrior-tui.task-report.info-location` (`auto`, `bottom`, or `right`); use `\` to transpose the split. Pressing `z` while annotations are visible switches the pane to the selected task's details.

From Projects, Timesheet, or Calendar, `n` opens the task list with annotations; toggling it off returns to the previous view. The toggle and annotation scrolling do not intercept text while editing a command, filter, or menu search.

Each entry contains one annotation and its creation date/time, with a project/task heading. Entries are sorted newest first across **all tasks**, including completed, deleted, and waiting tasks, regardless of the current report filter or active context. Only consecutive entries belonging to the same task share a heading; annotations are never reordered to force grouping. Tasks without a project show `(no project)`, and tasks without a numeric ID show their short UUID. Timestamps use your local timezone.

- `Ctrl-e` / `Ctrl-y`: scroll annotations down / up one line.
- `Ctrl-d` / `Ctrl-u`: scroll annotations down / up one page.
- `j` / `k`, arrows, `J` / `K`, and `g` / `G`: navigate the **task list**, as usual.
- Task actions and filtering remain available while the pane is open.
- `r`: refresh tasks and annotations (database changes are also picked up on normal refresh ticks).
- `q`: quit the application.

Customize the toggle with `uda.taskwarrior-tui.keyconfig.annotations` in your `.taskrc`. To color only the project names in annotation headings, use [`uda.taskwarrior-tui.style.project-title.<project>`](./configuration/colors.md#project-name-only-colors).

Keybindings for calendar:

    j: {selected+=1}                     - Move forward a year in calendar

    k: {selected-=1}                     - Move back a year in calendar

    J: {selected+=10}                    - Move forward a decade in calendar

    K: {selected-=10}                    - Move back a decade in calendar
