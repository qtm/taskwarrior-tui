#!/usr/bin/env python3
"""Regress external-editor/shortcut resume without cursor-position replies.

Build first, then run with --task /path/to/taskwarrior-3.x. Uses a disposable
Taskwarrior database, a fake editor, and the checklist suite's Unix PTY helper.
No personal tasks, configuration, or editor are used.
"""
import argparse
import json
import os
import re
import runpy
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
Terminal = runpy.run_path(str(ROOT / 'scripts/test-checklists.py'))['Terminal']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--task', default=os.environ.get('TASKWARRIOR_TUI_TASKWARRIOR_CLI', shutil.which('task')))
    parser.add_argument('--tui', default=str(ROOT / 'target/debug/taskwarrior-tui'))
    args = parser.parse_args()
    if not args.task:
        parser.error('Pass --task /path/to/taskwarrior-3.x')
    task, tui = str(Path(args.task).resolve()), str(Path(args.tui).resolve())
    with tempfile.TemporaryDirectory(prefix='editor-resume-e2e-') as directory:
        tmp = Path(directory)
        mode = tmp / 'mode'
        calls = tmp / 'editor-calls'
        editor = tmp / 'editor'
        editor.write_text(f'''#!{sys.executable}
import fcntl, json, re, struct, sys, termios
from pathlib import Path
mode = Path({str(mode)!r}).read_text()
with Path({str(calls)!r}).open('a') as log:
    log.write(json.dumps({{'mode': mode, 'args': sys.argv[1:], 'canonical': bool(termios.tcgetattr(0)[3] & termios.ICANON)}}) + '\\n')
if mode == 'fail':
    sys.exit(1)
if mode == 'edit':
    path = Path(sys.argv[1])
    text, count = re.subn(r'^  Description:.*$', '  Description:       EDITED_ONE', path.read_text(), flags=re.M)
    assert count == 1
    path.write_text(text)
if mode == 'note':
    path = Path(sys.argv[1])
    assert path.is_file(), 'Note must exist before launching editor'
    path.write_text(path.read_text() + '# Note content\\n')
if mode == 'resize':
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack('HHHH', 44, 100, 0, 0))
''')
        editor.chmod(0o700)
        env = dict(os.environ, TASKRC=str(tmp / 'taskrc'), TASKDATA=str(tmp / 'data'),
                   TASKWARRIOR_TUI_CONFIG=str(tmp / 'config'), TASKWARRIOR_TUI_DATA=str(tmp / 'tui-data'),
                   TASKWARRIOR_TUI_TASKWARRIOR_CLI=task, TERM='xterm-256color')
        (tmp / 'taskrc').write_text(f'''confirmation=off
editor={editor}
report.next.filter=status:pending
report.next.columns=id,description
report.next.labels=ID,Description
report.next.sort=id+
uda.taskwarrior-tui.task-report.show-info=false
uda.taskwarrior-tui.tick-rate=0
uda.taskwarrior-tui.notes-directory={tmp / 'My Notes'}
uda.taskwarrior-tui.shortcuts.1=/usr/bin/true
uda.taskwarrior-tui.shortcuts.2=/usr/bin/false
''')
        one = '00000000-0000-0000-0000-000000000001'
        two = '00000000-0000-0000-0000-000000000002'
        data = [dict(uuid=uuid, entry='20260101T000000Z', status='pending', description=name)
                for uuid, name in [(one, 'TASK_ONE'), (two, 'TASK_TWO')]]
        subprocess.run([task, 'import'], input=json.dumps(data), text=True, env=env, capture_output=True, check=True)

        def tasks():
            output = subprocess.run([task, 'rc.json.array=on', 'export'], text=True, env=env,
                                    capture_output=True, check=True).stdout
            return {item['uuid']: item for item in json.loads(output)}

        terminal = Terminal(tui, env, respond_to_cursor_queries=False)
        try:
            terminal.wait_for(lambda screen: 'TASK_ONE' in screen and 'Filter Tasks' in screen)

            def resume(key, label):
                clears = terminal.screen_clears
                pushes = terminal.keyboard_requests.count('>1')
                terminal.send(key, 0.1)
                # A fresh clear + frame is required: the old report may still be
                # visible while the editor runs or a cursor query is timing out.
                screen = terminal.wait_for(lambda screen: terminal.screen_clears > clears and label in screen)
                assert terminal.keyboard_requests.count('>1') == pushes + 1
                assert terminal.keyboard_requests.count('<1') == pushes
                assert terminal.cursor_queries == 0, 'Resume requested the terminal cursor position'
                return screen

            mode.write_text('noop')
            resume('e', 'Filter Tasks')
            assert 'No edits were detected.' in terminal.transcript
            assert tasks()[one]['description'] == 'TASK_ONE'
            print('PASS: no-op task editor returns to a fully redrawn, responsive TUI', flush=True)

            mode.write_text('edit')
            screen = resume('e', 'Filter Tasks')
            assert 'EDITED_ONE' in screen and 'TASK_ONE' not in screen, screen
            assert tasks()[one]['description'] == 'EDITED_ONE'
            assert tasks()[two]['description'] == 'TASK_TWO'
            print('PASS: saved external edits refresh the highlighted task', flush=True)

            mode.write_text('fail')
            # Taskwarrior 3.3 reports an editor failure on stdout but returns 0.
            resume('e', 'Filter Tasks')
            assert 'Editing failed with exit code' in terminal.transcript
            assert tasks()[one]['description'] == 'EDITED_ONE'
            print('PASS: failed editor returns without exiting or changing the task', flush=True)

            mode.write_text('resize')
            screen = resume('e', 'Filter Tasks')
            assert 'Filter Tasks' in screen.splitlines()[42], screen
            assert 'EDITED_ONE' in screen and 'TASK_TWO' in screen, screen
            resume('1', 'Filter Tasks')
            resume('2', 'Unable to run shortcut 2')
            terminal.send('\x1b')
            terminal.wait_for(lambda screen: 'Filter Tasks' in screen)
            mode.write_text('noop')
            resume('e', 'Filter Tasks')
            assert all(t['status'] == 'pending' for t in tasks().values())
            invocations = [json.loads(line) for line in calls.read_text().splitlines()]
            assert [call['mode'] for call in invocations] == ['noop', 'edit', 'fail', 'resize', 'noop']
            assert all(call['canonical'] for call in invocations), 'Editor inherited TUI raw mode'
            print('PASS: resize during editing, shortcuts, repeated resume, and terminal-mode restoration', flush=True)

            # Notes act on the highlighted task, even when other tasks are marked.
            terminal.send('V')
            mode.write_text('note')
            resume('E', 'Filter Tasks')
            note = Path(tasks()[one]['tuinote'])
            assert note.parent == tmp / 'My Notes'
            assert re.fullmatch(r'\d{6}-\d{4} EDITED_ONE\.md', note.name), note
            assert 'tuinote' not in tasks()[two]
            assert note.read_text() == '# Note content\n'
            resume('E', 'Filter Tasks')
            assert tasks()[one]['tuinote'] == str(note)
            assert note.read_text() == '# Note content\n' * 2
            assert len(list(note.parent.iterdir())) == 1
            print('PASS: Shift+E creates, associates, opens, and reopens only the highlighted task note', flush=True)

            mode.write_text('fail')
            resume('E', 'Note editor exited with')
            terminal.send('\x1b')
            terminal.wait_for(lambda screen: 'Filter Tasks' in screen)
            assert note.read_text() == '# Note content\n' * 2
            # A spawn failure must also restore input and redraw the TUI.
            moved = editor.with_name('editor-moved')
            editor.rename(moved)
            try:
                resume('E', 'Unable to start note editor')
            finally:
                moved.rename(editor)
            terminal.send('\x1b')
            terminal.wait_for(lambda screen: 'Filter Tasks' in screen)
            mode.write_text('noop')
            resume('E', 'Filter Tasks')
            assert tasks()[one]['tuinote'] == str(note)
            invocations = [json.loads(line) for line in calls.read_text().splitlines()]
            assert [call['mode'] for call in invocations[5:]] == ['note', 'note', 'fail', 'noop']
            assert all(call['args'] == [str(note)] for call in invocations[5:])
            assert all(call['canonical'] for call in invocations)
            print('PASS: note editor failure/spawn failure preserve the note and restore terminal input', flush=True)

            terminal.send('q', 0.1)
            deadline = time.monotonic() + 8
            pid, status = os.waitpid(terminal.pid, os.WNOHANG)
            while not pid and time.monotonic() < deadline:
                terminal.read(0.1)
                pid, status = os.waitpid(terminal.pid, os.WNOHANG)
            terminal.finished = bool(pid)
            assert pid and os.waitstatus_to_exitcode(status) == 0, (pid, status)
            assert terminal.keyboard_requests == ['>1', '<1'] * 13, terminal.keyboard_requests
            assert terminal.cursor_queries == 0
            print('PASS: clean exit; no cursor-position queries at any point', flush=True)
        except Exception:
            terminal.read(0.1)
            Path('/tmp/taskwarrior-editor-resume-failed-output.txt').write_text(terminal.transcript)
            raise
        finally:
            terminal.stop()


if __name__ == '__main__':
    main()
