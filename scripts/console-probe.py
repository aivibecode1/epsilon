"""Measure what the TUI does to a real Windows console.

The owner's report was: launch `epsilon`, start typing, and "the terminal shrinks
and freezes" — nothing typed or streamed ever appears.  A ConPTY test (which is
what `crates/epsilon-cli/tests/terminal.rs` uses) cannot see a *window* resize,
because under ConPTY the console lives inside the terminal application, not in
conhost.  This probe drives a genuine console window instead:

  1. detach from whatever console we inherited and create a brand-new one by
     running a shell in it — exactly what a user does when they open a terminal;
  2. attach to that console and record the host window class, the screen buffer
     size and the visible window rectangle.  The shell sleeps for a few seconds
     first, so this is a real "before": a console the TUI has not touched;
  3. let the shell launch the TUI *in the same console*, and record the same
     measurements.  A visible window that changed size here is the reported
     shrink;
  4. push real KEY_EVENT records into the console input buffer and read the screen
     back — that is what "nothing I type ever appears" is about.

The TUI must be launched *by the shell in that console*, not spawned as a child of
this probe: a child would inherit this process's stdout, which is the pipe the
harness reads, and the program would render into the pipe instead of the screen.
That mistake costs one confusing run — the first version of this file made it, and
the screen read back a bare prompt while the frame appeared in the JSON.

Output is one JSON object on stdout.  Nothing is written to the repository.
"""

import ctypes
import json
import os
import subprocess
import sys
import time
from ctypes import wintypes

K32 = ctypes.WinDLL("kernel32", use_last_error=True)
# GetClassNameW lives in user32, not kernel32. The window class is how this probe
# tells a legacy conhost window from a Windows Terminal one: "ConsoleWindowClass"
# is conhost, "CASCADIA_HOSTING_WINDOW_CLASS" is Windows Terminal.
USER32 = ctypes.WinDLL("user32", use_last_error=True)

GENERIC_READ = 0x80000000
GENERIC_WRITE = 0x40000000
FILE_SHARE_READ = 0x00000001
FILE_SHARE_WRITE = 0x00000002
OPEN_EXISTING = 3
CREATE_NEW_CONSOLE = 0x00000010
CREATE_NO_WINDOW = 0x08000000


class COORD(ctypes.Structure):
    _fields_ = [("X", ctypes.c_short), ("Y", ctypes.c_short)]


class SMALL_RECT(ctypes.Structure):
    _fields_ = [
        ("Left", ctypes.c_short),
        ("Top", ctypes.c_short),
        ("Right", ctypes.c_short),
        ("Bottom", ctypes.c_short),
    ]


class CONSOLE_SCREEN_BUFFER_INFO(ctypes.Structure):
    _fields_ = [
        ("dwSize", COORD),
        ("dwCursorPosition", COORD),
        ("wAttributes", wintypes.WORD),
        ("srWindow", SMALL_RECT),
        ("dwMaximumWindowSize", COORD),
    ]


class KEY_EVENT_RECORD(ctypes.Structure):
    _fields_ = [
        ("bKeyDown", wintypes.BOOL),
        ("wRepeatCount", wintypes.WORD),
        ("wVirtualKeyCode", wintypes.WORD),
        ("wVirtualScanCode", wintypes.WORD),
        ("UnicodeChar", ctypes.c_wchar),
        ("dwControlKeyState", wintypes.DWORD),
    ]


class _EVENT_UNION(ctypes.Union):
    _fields_ = [("KeyEvent", KEY_EVENT_RECORD), ("_pad", ctypes.c_byte * 16)]


class INPUT_RECORD(ctypes.Structure):
    _fields_ = [("EventType", wintypes.WORD), ("Event", _EVENT_UNION)]


KEY_EVENT = 0x0001
assert ctypes.sizeof(INPUT_RECORD) == 20, ctypes.sizeof(INPUT_RECORD)

K32.FreeConsole.restype = wintypes.BOOL
K32.AttachConsole.restype = wintypes.BOOL
K32.AttachConsole.argtypes = [wintypes.DWORD]
K32.GetConsoleWindow.restype = wintypes.HWND
K32.CreateFileW.restype = wintypes.HANDLE
K32.CreateFileW.argtypes = [
    wintypes.LPCWSTR,
    wintypes.DWORD,
    wintypes.DWORD,
    ctypes.c_void_p,
    wintypes.DWORD,
    wintypes.DWORD,
    ctypes.c_void_p,
]
USER32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
USER32.GetClassNameW.restype = ctypes.c_int
USER32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
USER32.ShowWindow.restype = wintypes.BOOL
SW_MAXIMIZE = 3
SW_RESTORE = 9


def open_console(name):
    h = K32.CreateFileW(
        name,
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        None,
        OPEN_EXISTING,
        0,
        None,
    )
    if h == wintypes.HANDLE(-1).value:
        raise OSError(f"cannot open {name}: {ctypes.get_last_error()}")
    return h


def snapshot(out_handle):
    info = CONSOLE_SCREEN_BUFFER_INFO()
    if not K32.GetConsoleScreenBufferInfo(out_handle, ctypes.byref(info)):
        return {"error": f"GetConsoleScreenBufferInfo failed: {ctypes.get_last_error()}"}
    win = info.srWindow
    return {
        "buffer": [info.dwSize.X, info.dwSize.Y],
        "window_origin": [win.Left, win.Top],
        "window_cols": win.Right - win.Left + 1,
        "window_rows": win.Bottom - win.Top + 1,
        "cursor": [info.dwCursorPosition.X, info.dwCursorPosition.Y],
    }


def screen_text(out_handle, limit_rows=16):
    """Read the visible window out of the console's active screen buffer.

    ReadConsoleOutputCharacterW wants the *starting cell*, which is the top-left of
    the visible window, not (0, 0) — on a console with scrollback, (0, 0) is
    off-screen history and reads back blank.
    """
    info = CONSOLE_SCREEN_BUFFER_INFO()
    if not K32.GetConsoleScreenBufferInfo(out_handle, ctypes.byref(info)):
        return []
    win = info.srWindow
    cols = win.Right - win.Left + 1
    rows = min(win.Bottom - win.Top + 1, limit_rows)
    lines = []
    for r in range(rows):
        buf = ctypes.create_unicode_buffer(cols + 1)
        read = wintypes.DWORD(0)
        start = COORD(win.Left, win.Top + r)
        ok = K32.ReadConsoleOutputCharacterW(
            out_handle, buf, cols, start, ctypes.byref(read)
        )
        lines.append(
            buf.value.rstrip() if ok else f"<read failed {ctypes.get_last_error()}>"
        )
    return lines


def host_class():
    hwnd = K32.GetConsoleWindow()
    if not hwnd:
        return None
    buf = ctypes.create_unicode_buffer(256)
    USER32.GetClassNameW(hwnd, buf, 256)
    return buf.value


def set_size(out_handle, cols, rows, buffer_rows=None):
    """Force the console to `cols` x `rows` visible cells, with `buffer_rows` of
    scrollback behind them.

    The buffer has to grow before the window can, and the window has to fit inside
    the buffer before the buffer may shrink — so the order is: big buffer, window,
    then the buffer we actually want. Getting this order wrong returns
    ERROR_INVALID_PARAMETER from whichever call is out of turn.

    `buffer_rows` matters: a real `cmd` window is typically 120x30 *visible* over a
    9001-row buffer. If entering the alternate screen resets the console to a
    default size, the visible window is what the user sees move, and the scrollback
    is what the console has to be holding for the comparison to mean anything.
    """
    final_rows = buffer_rows or rows
    steps = []
    steps.append(
        ("buffer-big", K32.SetConsoleScreenBufferSize(out_handle, COORD(cols, max(9001, final_rows))))
    )
    steps.append(
        (
            "window",
            K32.SetConsoleWindowInfo(
                out_handle, True, ctypes.byref(SMALL_RECT(0, 0, cols - 1, rows - 1))
            ),
        )
    )
    steps.append(
        ("buffer-exact", K32.SetConsoleScreenBufferSize(out_handle, COORD(cols, final_rows)))
    )
    return {name: bool(ok) for name, ok in steps}


#: Text characters that need a real virtual key code to mean anything. A console
#: application reads the *key*, not the character: `\r` with a zero VK code is a
#: keystroke no application recognises as Enter, which is why a probe that only sent
#: characters could type `/help` into the composer and never submit it.
VK_BY_CHAR = {"\r": 0x0D, "\n": 0x0D, "\t": 0x09, "\x1b": 0x1B}


def type_text(in_handle, text):
    records = (INPUT_RECORD * (len(text) * 2))()
    i = 0
    for ch in text:
        for down in (True, False):
            rec = records[i]
            rec.EventType = KEY_EVENT
            rec.Event.KeyEvent.bKeyDown = 1 if down else 0
            rec.Event.KeyEvent.wRepeatCount = 1
            rec.Event.KeyEvent.wVirtualKeyCode = VK_BY_CHAR.get(ch, 0)
            rec.Event.KeyEvent.wVirtualScanCode = 0
            rec.Event.KeyEvent.UnicodeChar = ch
            rec.Event.KeyEvent.dwControlKeyState = 0
            i += 1
    written = wintypes.DWORD(0)
    ok = K32.WriteConsoleInputW(in_handle, records, i, ctypes.byref(written))
    return {"ok": bool(ok), "written": written.value, "err": ctypes.get_last_error()}


def kill(pid):
    subprocess.run(
        ["taskkill", "/F", "/T", "/PID", str(pid)],
        capture_output=True,
        creationflags=CREATE_NO_WINDOW,
    )


def main():
    argv = sys.argv[1:]
    # An unknown flag used to fall through to `positionals` and be treated as the exe path, so
    # `--help` opened a console and tried to run a file named "--help". A diagnostic that fails
    # by doing something confusing is worse than one that refuses: the obvious reading of the
    # output was "the probe is broken", not "you passed me nonsense".
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        print(__file__)
        return 0
    for a in argv:
        if a.startswith("--") and a not in ("--early", "--late", "--maximize", "--at", "--size"):
            sys.stderr.write(f"console-probe: unknown option {a!r}\n")
            sys.stderr.write("options: --early --late --maximize --at SECONDS --size COLSxROWS\n")
            return 2
    mode = "late"
    size = None
    maximize = False
    wait_s = 7.0
    text = "hello from the probe"
    positionals = []
    i = 0
    while i < len(argv):
        if argv[i] == "--early":
            mode = "early"
        elif argv[i] == "--late":
            mode = "late"
        elif argv[i] == "--maximize":
            maximize = True
        elif argv[i] == "--at":
            # Seconds to wait after attaching before typing. The default suits a settled
            # TUI; measuring whether early keystrokes survive needs it smaller.
            i += 1
            wait_s = float(argv[i])
        elif argv[i] == "--size":
            i += 1
            parts = argv[i].lower().split("x")
            size = tuple(int(p) for p in parts)
        else:
            positionals.append(argv[i])
        i += 1
    exe = os.path.abspath(positionals[0])
    if len(positionals) > 1:
        text = positionals[1]

    result = {"exe": exe, "typed_text": text, "mode": mode, "size": size, "wait_s": wait_s}

    # 1. A console of our own, created the way a user creates one. `ping` is the
    #    sleep: it does not read stdin, so nothing competes with the TUI for keys.
    K32.FreeConsole()
    comspec = os.environ.get("COMSPEC", r"C:\Windows\System32\cmd.exe")
    line = f'{comspec} /c "ping -n 6 127.0.0.1 >nul & "{exe}""'
    shell = subprocess.Popen(line, creationflags=CREATE_NEW_CONSOLE)
    time.sleep(1.5)

    if not K32.AttachConsole(shell.pid):
        result["error"] = f"AttachConsole failed: {ctypes.get_last_error()}"
        kill(shell.pid)
        print(json.dumps(result, indent=2))
        return 1
    result["host_class"] = host_class()

    out = open_console("CONOUT$")
    inp = open_console("CONIN$")

    if size:
        result["resize"] = set_size(out, *size)
    if maximize:
        # A maximized console is the common Windows setup, and it is the case where
        # a buffer switch is most visible: the window is the whole screen, so any
        # buffer size the console "helpfully" adopts shows up as the window jumping.
        hwnd = K32.GetConsoleWindow()
        USER32.ShowWindow(hwnd, SW_MAXIMIZE)
        time.sleep(1.0)
        result["maximized"] = True

    # 2. "Before": the shell is still sleeping, the TUI has not started.
    time.sleep(1.0)
    result["before"] = snapshot(out)

    if mode == "early":
        # The owner's words: "the moment I launch it and start typing". Type into the
        # console *while* the program is initialising — before it is listening — and
        # then again once it should be up, to see whether the early keys arrive,
        # vanish, or wedge the frame.
        time.sleep(2.4)  # ping is about to end; the shell is launching the TUI now
        result["typed_early"] = type_text(inp, text)
        time.sleep(1.2)
        result["typed_early_again"] = type_text(inp, " second")
        time.sleep(4.0)
    else:
        time.sleep(wait_s)

    result["after"] = snapshot(out)
    result["after_screen"] = screen_text(out, limit_rows=64)

    if mode == "late":
        result["typed"] = type_text(inp, text)
        time.sleep(2.5)
        result["after_typing"] = snapshot(out)
        result["typed_screen"] = screen_text(out, limit_rows=64)

    kill(shell.pid)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
