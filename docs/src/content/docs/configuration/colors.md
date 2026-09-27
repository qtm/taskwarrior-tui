---
title: Color Configuration
description: Reuse Taskwarrior color settings and modifiers inside taskwarrior-tui.
---

`taskwarrior-tui` reads color values from your Taskwarrior `taskrc` file (default: `~/.taskrc`).

![](https://user-images.githubusercontent.com/1813121/96684390-bf173e80-1338-11eb-971c-ae64233d142e.png)

For example, `color.active` is used to style the active task. If you would like to try it, open your `taskrc` file and change `color.active=white on blue`.

So `color.active` will take precedence over `color.overdue`. You can see what `color.active` is by running `task show color.active` in your shell.

The following color attributes are supported:

```plaintext
color.deleted
color.completed
color.active
color.alternate
color.overdue
color.scheduled
color.due.today
color.due
color.blocked
color.blocking
color.recurring
color.tagged
```

`color.alternate` is applied to every other row in the task report by default. If you want to keep that Taskwarrior color defined but disable it in the TUI, set `uda.taskwarrior-tui.task-report.use-alternate-style=false`.

Similarly, `taskwarrior-tui`-specific styles can also be customized in `taskrc` with the following attributes:

```plaintext
uda.taskwarrior-tui.style.context.active
uda.taskwarrior-tui.style.calendar.title
uda.taskwarrior-tui.style.navbar
uda.taskwarrior-tui.style.command
uda.taskwarrior-tui.style.report.scrollbar
uda.taskwarrior-tui.style.report.scrollbar.area
```

## Project-name-only colors

To color **only project names**, add per-project TUI styles to `~/.taskrc`:

```plaintext
uda.taskwarrior-tui.style.project-title.work=bold blue
uda.taskwarrior-tui.style.project-title.personal=green
uda.taskwarrior-tui.style.project-title.work.client=color208
```

These styles apply to:

- Project values in the task report's `project` column, even with a custom column label.
- The project-name portion of each grouped heading in the annotations pane (`n`).
- The project name in the checklist pane (`C`).

Task descriptions, IDs, dates, annotation text, column headings, and selection markers are not recolored. Existing row/selection styles remain underneath the project-name style. All color formats and modifiers below are supported.

Subprojects inherit the nearest configured parent's style: `work.other` uses `work`, while `work.client.api` uses `work.client`. Matching respects dot-separated components, so `workshop` does not inherit `work`. Projects without a matching setting keep their normal styling; an empty per-project value disables inherited title styling for that project and its children.

This setting is independent of Taskwarrior's `color.project.<name>`, which **still colors entire task rows**. For title-only coloring, replace any corresponding `color.project.<name>` entries with the new `uda.taskwarrior-tui.style.project-title.<name>` entries. Restart the TUI to reload styles.

## Color Formats

All color formats supported by Taskwarrior are recognized:

### Named colors (16-color)

```plaintext
black  red  green  yellow  blue  magenta  cyan  white
```

Use `on <color>` to set the background:

```plaintext
red on blue        # red foreground, blue background
on yellow          # default foreground, yellow background
```

### High-intensity (bright) colors

Use the `bright` prefix on backgrounds to get the high-intensity variant (terminal colors 8-15):

```plaintext
black on bright green    # bright green background
white on bright black    # bright black (dark gray) background
```

### 256-color indexed

Use `color0` through `color255` to access the full 256-color palette:

```plaintext
color196           # bright red
color60 on color60 # foreground and background using index 60
```

### Grayscale ramp

Use `gray0` through `gray23` (or `grey0`-`grey23`) to access the 24-step grayscale ramp (terminal colors 232-255):

```plaintext
gray5              # dark gray (Color::Indexed(237))
white on gray10    # white text on mid-gray background
```

### RGB color cube

Use `rgbRGB` where `R`, `G`, and `B` are each a digit from `0` to `5` to address the `6x6x6` color cube (terminal colors 16-231):

```plaintext
rgb500             # bright red  (Color::Indexed(196))
rgb050             # bright green
rgb005             # bright blue
rgb444             # medium gray-ish
```

## Modifiers

The following text modifiers can be combined with any color:

| Modifier | Effect |
| --- | --- |
| `bold` | Bold text |
| `underline` | Underlined text |
| `inverse` | Swapped foreground/background |
| `italic` | Italic text |
| `strikethrough` | Strikethrough text |

Multiple modifiers can be combined:

```plaintext
bold underline red on blue
italic color111 on color60
```

## `bold` vs `bright`

- `bold` is a text attribute. It makes text bold and uses the regular color index.
- `bright` is a color variant. For backgrounds, it selects the high-intensity color (index + 8).

```plaintext
bold red           # regular red (#1) + bold attribute
on bright red      # high-intensity red background (#9)
bold on bright red # bold text + high-intensity red background
```

## Example: TokyoNight Moon Theme

```plaintext
color.active=color111 on color60
color.overdue=color203
color.due.today=color215
color.due=color111
color.blocked=color245
color.tagged=color141
color.recurring=color147
color.scheduled=color109
```

See [advanced configuration](./advanced.md) for TUI-specific style keys.
