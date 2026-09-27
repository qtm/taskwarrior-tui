#!/usr/bin/env python3
"""Live checklist smoke test; creates disposable tasks and a fake clipboard.

Build first, then run with --task /path/to/taskwarrior-3.x. No personal data or
clipboard is read. Requires a Unix PTY and only Python's standard library.
"""
import argparse
import codecs
import errno
import fcntl
import json
import os
import pty
import re
import select
import shlex
import shutil
import signal
import struct
import subprocess
import tempfile
import termios
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class Terminal:
    def __init__(self, executable, env, respond_to_cursor_queries=True):
        self.rows, self.cols = 48, 120
        self.screen = [[' '] * self.cols for _ in range(self.rows)]
        self.cursor = [0, 0]
        self.keyboard_requests = []
        self.cursor_queries = 0
        self.screen_clears = 0
        self.respond_to_cursor_queries = respond_to_cursor_queries
        self.transcript = ''
        self.decoder = codecs.getincrementaldecoder('utf-8')(errors='replace')
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            time.sleep(0.1)
            os.execve(executable, [executable], env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack('HHHH', self.rows, self.cols, 0, 0))
        self.finished = False

    def read(self, seconds=0.3):
        data = bytearray()
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([self.fd], [], [], 0.03)[0]:
                try:
                    chunk = os.read(self.fd, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                if not chunk:
                    break
                data.extend(chunk)
        decoded = self.decoder.decode(data)
        self.transcript = (self.transcript + decoded)[-100000:]
        for token in re.findall(r'\x1b\[[0-?]*[ -/]*[@-~]|\x1b.|[^\x1b]', decoded):
            if token.startswith('\x1b['):
                op, params = token[-1], token[2:-1]
                if params.startswith(('>', '<')):
                    if op == 'u':
                        self.keyboard_requests.append(params)
                    continue
                if params.startswith('?'):
                    continue
                values = [int(v or 0) for v in params.split(';')] if params else [0]
                n = values[0] or 1
                if op == 'n' and values[0] == 6:
                    self.cursor_queries += 1
                    if self.respond_to_cursor_queries:
                        row = min(max(self.cursor[0] + 1, 1), self.rows)
                        col = min(max(self.cursor[1] + 1, 1), self.cols)
                        os.write(self.fd, f'\x1b[{row};{col}R'.encode())
                elif op in ('H', 'f'):
                    self.cursor = [n - 1, ((values[1] if len(values) > 1 else 1) or 1) - 1]
                elif op == 'G': self.cursor[1] = n - 1
                elif op == 'A': self.cursor[0] = max(0, self.cursor[0] - n)
                elif op == 'B': self.cursor[0] += n
                elif op == 'C': self.cursor[1] += n
                elif op == 'D': self.cursor[1] = max(0, self.cursor[1] - n)
                elif op == 'J' and values[0] == 2:
                    self.screen_clears += 1
                    self.screen = [[' '] * self.cols for _ in range(self.rows)]
                elif op == 'K' and self.cursor[0] < self.rows:
                    start = 0 if values[0] in (1, 2) else self.cursor[1]
                    end = self.cursor[1] + 1 if values[0] == 1 else self.cols
                    self.screen[self.cursor[0]][start:end] = [' '] * (end - start)
            elif token == '\r': self.cursor[1] = 0
            elif token == '\n': self.cursor[0] += 1
            elif not token.startswith('\x1b') and ord(token) >= 32:
                row, col = self.cursor
                if 0 <= row < self.rows and 0 <= col < self.cols:
                    self.screen[row][col] = token
                self.cursor[1] += 1
        return '\n'.join(''.join(row).rstrip() for row in self.screen)

    def send(self, keys, seconds=0.3):
        os.write(self.fd, keys.encode())
        return self.read(seconds)

    def paste(self, text):
        return self.send('\x1b[200~' + text + '\x1b[201~')

    def wait_for(self, predicate, timeout=8):
        deadline = time.monotonic() + timeout
        screen = self.read(0.1)
        while not predicate(screen) and time.monotonic() < deadline:
            screen = self.read(0.1)
        assert predicate(screen), screen
        return screen

    def finish_edit(self, keys='\r'):
        self.send(keys, 0.1)
        # Persistence precedes the asynchronous report refresh/redraw. Wait for
        # Report mode rather than assuming a fixed sleep covers every machine.
        return self.wait_for(lambda screen: 'Filter Tasks' in '\n'.join(screen.splitlines()[-2:]))

    def stop(self):
        if not self.finished:
            try: os.kill(self.pid, signal.SIGTERM)
            except ProcessLookupError: pass
            os.waitpid(self.pid, 0)
        os.close(self.fd)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--task', default=os.environ.get('TASKWARRIOR_TUI_TASKWARRIOR_CLI', shutil.which('task')))
    parser.add_argument('--tui', default=str(ROOT / 'target/debug/taskwarrior-tui'))
    args = parser.parse_args()
    if not args.task:
        parser.error('Pass --task /path/to/taskwarrior-3.x')
    task, tui = str(Path(args.task).resolve()), str(Path(args.tui).resolve())
    with tempfile.TemporaryDirectory(prefix='checklists-e2e-') as directory:
        tmp = Path(directory)
        binaries = tmp / 'bin'
        binaries.mkdir()
        clipboard = tmp / 'clipboard.md'
        fail = tmp / 'clipboard-fail'
        for name in ('wl-paste', 'pbpaste'):
            helper = binaries / name
            helper.write_text(f'#!/bin/sh\n[ ! -e {shlex.quote(str(fail))} ] || exit 1\nexec /bin/cat {shlex.quote(str(clipboard))}\n')
            helper.chmod(0o700)
        for name in ('xclip', 'xsel'):
            helper = binaries / name
            helper.write_text('#!/bin/sh\nexit 1\n')
            helper.chmod(0o700)
        env = dict(os.environ, TASKRC=str(tmp / 'taskrc'), TASKDATA=str(tmp / 'data'),
                   TASKWARRIOR_TUI_CONFIG=str(tmp / 'config'), TASKWARRIOR_TUI_DATA=str(tmp / 'tui-data'),
                   TASKWARRIOR_TUI_TASKWARRIOR_CLI=task, TERM='xterm-256color',
                   PATH=str(binaries) + os.pathsep + os.environ.get('PATH', ''))
        (tmp / 'taskrc').write_text('''confirmation=off
report.next.filter=status:pending
report.next.columns=id,project,description,tuichecklist
report.next.labels=ID,Project,Description,Checklist
report.next.sort=id+
uda.tuichecklist.type=string
uda.tuichecklist.label=Checklist
uda.taskwarrior-tui.task-report.show-info=false
uda.taskwarrior-tui.task-report.prompt-on-undo=false
uda.taskwarrior-tui.task-report.info-location=bottom
uda.taskwarrior-tui.tick-rate=0
uda.taskwarrior-tui.shortcuts.1=/usr/bin/true
''')
        one = '00000000-0000-0000-0000-000000000001'
        two = '00000000-0000-0000-0000-000000000002'
        data = [dict(uuid=uuid, entry='20260101T000000Z', status='pending', description=name, project='work')
                for uuid, name in [(one, 'TASK_ONE'), (two, 'TASK_TWO')]]
        data[0]['annotations'] = [
            dict(entry='20200101T120000Z', description='OLD_TASK_ONE_NOTE'),
            dict(entry='20200102T120000Z', description='NEW_TASK_ONE_NOTE'),
        ]
        data[1]['annotations'] = [dict(entry='20200103T120000Z', description='OTHER_TASK_NOTE')]
        subprocess.run([task, 'import'], input=json.dumps(data), text=True, env=env, capture_output=True, check=True)

        def command(*parts):
            return subprocess.run([task, *parts], text=True, env=env, capture_output=True, check=True).stdout

        def tasks():
            return {item['uuid']: item for item in json.loads(command('rc.json.array=on', 'export'))}

        def document(uuid=one):
            return json.loads(tasks()[uuid].get('tuichecklist', '{"version":1,"lists":[]}'))

        clipboard.write_text((ROOT / 'tests/fixtures/checklist-ru.md').read_text())
        terminal = Terminal(tui, env)
        try:
            screen = terminal.read(1.5)
            assert 'TASK_ONE' in screen, screen
            assert '>1' in terminal.keyboard_requests, 'Modified-key reporting was not requested'
            screen = terminal.send('I', 0.8)
            assert 'Import preview' in screen and '5/13 complete' in screen, screen
            assert 'Annotations — current task' not in screen and 'NEW_TASK_ONE_NOTE' not in screen
            assert not document()['lists']
            screen = terminal.finish_edit()
            first = document()['lists'][0]
            assert len(first['items']) == 13 and sum(i['depth'] == 1 for i in first['items']) == 4
            assert '  - [x] Добавить tech' in screen and '5/13 complete' in screen, screen
            assert '5/13' in screen.split('Checklists')[0], screen
            print('PASS: clipboard preview, Cyrillic nesting, attachment, and report progress', flush=True)

            screen = terminal.send('\x04')  # Scroll annotations with checklist focus.
            assert 'Annotations — current task' in screen and '2 annotations (local time)' in screen, screen
            assert screen.index('NEW_TASK_ONE_NOTE') < screen.index('OLD_TASK_ONE_NOTE'), screen
            assert 'OTHER_TASK_NOTE' not in screen, screen
            terminal.send('\t')
            screen = terminal.send('j')  # Follow the highlighted task, even with no checklists.
            assert 'No checklists.' in screen and 'OTHER_TASK_NOTE' in screen, screen
            assert 'TASK_ONE_NOTE' not in screen, screen
            terminal.send('k')
            before_annotation = document()
            command(one, 'annotate', 'REFRESHED_TASK_ONE_NOTE')
            terminal.send('r', 0.8)
            screen = terminal.send('\x04')  # Task focus has the same annotation scroll controls.
            assert screen.index('REFRESHED_TASK_ONE_NOTE') < screen.index('NEW_TASK_ONE_NOTE'), screen
            assert '3 annotations (local time)' in screen and 'OTHER_TASK_NOTE' not in screen, screen
            assert document() == before_annotation
            terminal.send('\x15')
            terminal.send('\t')
            print('PASS: task-only annotations below checklists, newest first, scrolling, task switching, and refresh', flush=True)

            terminal.send(' ')
            changed = document()['lists'][0]['items']
            assert not changed[0]['checked'] and changed[1]['checked']
            terminal.send('u', 0.8)
            assert document()['lists'][0] == first
            terminal.send('I')
            terminal.send('\x1b')
            assert len(document()['lists']) == 1
            clipboard.write_text('- [ ] valid\n- [y] malformed checkbox')
            terminal.send('I')
            screen = terminal.send('\r')
            assert 'Line 2' in screen and len(document()['lists']) == 1, screen
            terminal.send('\x1b')
            print('PASS: independent parent checkbox, native undo, cancel, and invalid import', flush=True)

            clipboard.write_text('# Second\n- [ ] Alpha\n  - [ ] Beta')
            terminal.send('I')
            terminal.finish_edit()
            assert len(document()['lists']) == 2 and document()['lists'][0] == first
            terminal.send('e\x15')
            terminal.paste('Новая строка')
            before_edit = document()
            terminal.send('\r')
            screen = terminal.paste('description\non multi line')
            assert 'description' in screen and 'on multi line' in screen, screen
            assert document() == before_edit, 'Enter must not save the item'
            terminal.finish_edit('\x1b[13;2u')  # Shift+Enter via the enhanced keyboard protocol.
            assert document()['lists'][1]['items'][0]['text'] == 'Новая строка\ndescription\non multi line'
            terminal.send('o')
            terminal.paste('дочерний')
            terminal.send('\r')
            terminal.paste('another description')
            terminal.finish_edit('\x13')  # Ctrl+s fallback on legacy terminals.
            terminal.send('>')
            assert document()['lists'][1]['items'][-1]['depth'] == 2
            terminal.send('<')
            assert document()['lists'][1]['items'][-1]['depth'] == 1
            terminal.send('\x1bk')
            assert document()['lists'][1]['items'][1]['text'] == 'дочерний\nanother description'
            terminal.send('g')
            terminal.send('x')
            terminal.send('\x1b')
            assert len(document()['lists'][1]['items']) == 3
            terminal.finish_edit('x\r')
            assert not document()['lists'][1]['items'] and document()['lists'][0] == first
            terminal.send('u', 0.8)
            assert len(document()['lists'][1]['items']) == 3
            assert tasks()[one]['status'] == 'pending'
            terminal.send('d')  # Focused checklist must never mark the task done.
            assert tasks()[one]['status'] == 'pending'
            print('PASS: editing, nested insert/reorder/indent/outdent, subtree deletion, task safety', flush=True)

            terminal.send('A')
            terminal.paste('Third')
            terminal.finish_edit()
            terminal.send('E\x15')
            terminal.paste('Renamed')
            terminal.finish_edit()
            assert document()['lists'][2]['title'] == 'Renamed'
            terminal.finish_edit('X\r')
            assert len(document()['lists']) == 2
            terminal.send('\t')
            terminal.send('jvkv')  # Mark both, but import attaches only to the highlighted task.
            clipboard.write_text('# Bulk-safe\n- [ ] One task only')
            terminal.send('I')
            terminal.finish_edit()
            assert len(document()['lists']) == 3 and not document(two)['lists']
            print('PASS: named list management and single-task writes with multiple tasks marked', flush=True)

            terminal.send('a')
            terminal.paste('draft retained')
            external = document()
            external['lists'][0]['title'] = 'Externally changed'
            command(one, 'modify', 'tuichecklist:' + json.dumps(external, ensure_ascii=False, separators=(',', ':')))
            screen = terminal.send('\x1b[13;2u', 0.8)
            assert 'changed outside this editor' in screen, screen
            assert document() == external
            terminal.send('\x1b')
            terminal.send('r', 0.8)
            fail.touch()
            screen = terminal.send('I', 0.8)
            assert 'Paste Markdown here' in screen, screen
            terminal.paste('# Pasted\n- [ ] Из буфера\n  - [x] Вложенный пункт')
            screen = terminal.finish_edit()
            assert document()['lists'][-1]['title'] == 'Pasted'
            assert document()['lists'][-1]['items'][1]['depth'] == 1
            assert document()['lists'][-1]['items'][1]['checked']
            print('PASS: stale-write rejection and multiline terminal-paste fallback', flush=True)

            fail.unlink()
            clipboard.write_text((ROOT / 'tests/fixtures/checklist-multiline.md').read_text())
            terminal.send('I')
            screen = terminal.finish_edit()
            items = document()['lists'][-1]['items']
            assert len(items) == 5
            assert items[0]['text'] == 'task\ntask description\non multi line'
            assert items[1]['text'] == 'subtask\nanother description' and items[1]['depth'] == 1
            assert items[3]['text'] == 'first level task with description\ndescription'
            assert '  task description' in screen and '    another description' in screen, screen
            print('PASS: multiline item editing, Shift+Enter, Ctrl+s, and nested Markdown descriptions', flush=True)

            terminal.send('C')
            screen = terminal.send('n', 0.8)
            assert 'Annotations — all tasks' in screen, screen
            terminal.send('C')
            screen = terminal.send('C')
            assert 'Annotations — all tasks' in screen, screen
            terminal.send('C')
            screen = terminal.send('\\')
            assert 'Checklists' in screen, screen
            screen = terminal.send('z', 0.8)
            assert 'Checklists —' not in screen and 'Annotations — all tasks' not in screen, screen
            assert all(t['status'] == 'pending' for t in tasks().values())
            assert {note['description'] for note in tasks()[one]['annotations']} == {
                'OLD_TASK_ONE_NOTE', 'NEW_TASK_ONE_NOTE', 'REFRESHED_TASK_ONE_NOTE',
            }, 'Checklist edits must preserve task annotations'
            terminal.send('1', 0.8)  # Suspend/resume around a harmless external shortcut.
            terminal.wait_for(lambda screen: terminal.keyboard_requests.count('>1') == 2 and 'TASK_ONE' in screen)
            assert terminal.keyboard_requests == ['>1', '<1', '>1'], terminal.keyboard_requests
            terminal.send('q', 0.1)
            deadline = time.monotonic() + 8
            pid, status = os.waitpid(terminal.pid, os.WNOHANG)
            while not pid and time.monotonic() < deadline:
                terminal.read(0.1)
                pid, status = os.waitpid(terminal.pid, os.WNOHANG)
            terminal.finished = bool(pid)
            assert pid and os.waitstatus_to_exitcode(status) == 0, (pid, status)
            assert terminal.keyboard_requests == ['>1', '<1', '>1', '<1'], 'Keyboard reporting was not restored'
            assert terminal.cursor_queries == 0, 'Full-screen redraw must not query the cursor'
            print('PASS: pane restoration, transpose, details switch, and clean exit', flush=True)
        except Exception:
            Path('/tmp/taskwarrior-checklists-failed-screen.txt').write_text(terminal.read(0.1))
            Path('/tmp/taskwarrior-checklists-failed-output.txt').write_text(terminal.transcript)
            log = tmp / 'tui-data/taskwarrior-tui.log'
            if log.exists():
                shutil.copyfile(log, '/tmp/taskwarrior-checklists-failed.log')
            raise
        finally:
            terminal.stop()


if __name__ == '__main__':
    main()
