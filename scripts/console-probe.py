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
#:
#: M176: extended with the printable repertoire. These are the US-layout virtual-key codes
#: for unshifted characters, which is what ConPTY and conhost use to place a character into
#: the console's input buffer. The four control entries above are NOT part of that
#: repertoire and must stay: they are how this probe sends a key that has no character.
VK_BY_CHAR = {
    "\r": 0x0D, "\n": 0x0D, "\t": 0x09, "\x1b": 0x1B,
    " ": 0x20, "a": 0x41, "b": 0x42, "c": 0x43, "d": 0x44, "e": 0x45, "f": 0x46,
    "g": 0x47, "h": 0x48, "i": 0x49, "j": 0x4A, "k": 0x4B, "l": 0x4C, "m": 0x4D,
    "n": 0x4E, "o": 0x4F, "p": 0x50, "q": 0x51, "r": 0x52, "s": 0x53, "t": 0x54,
    "u": 0x55, "v": 0x56, "w": 0x57, "x": 0x58, "y": 0x59, "z": 0x5A,
    "0": 0x30, "1": 0x31, "2": 0x32, "3": 0x33, "4": 0x34,
    "5": 0x35, "6": 0x36, "7": 0x37, "8": 0x38, "9": 0x39,
    "/": 0xBF, "\\": 0xDC, "-": 0xBD, "_": 0xDF, "=": 0xBB,
    ".": 0xBE, ",": 0xBC, "?": 0xBF, ":": 0xBA, ";": 0xBA,
    "'": 0xDE, '"': 0xDE, "!": 0x31, "@": 0x32, "#": 0x33,
    "$": 0x34, "%": 0x35, "^": 0x36, "&": 0x37, "*": 0x38,
    "(": 0x39, ")": 0x30, "+": 0xBB, "[": 0xDB, "]": 0xDD,
    "{": 0xDB, "}": 0xDD, "<": 0xBC, ">": 0xBE, "|": 0xDC, "~": 0xC0, "`": 0xC0,
}

# M176: keys that have no character, so `type_text` cannot express them. Without these the
# probe could compose a command and press Enter, but could not drive a picker - the
# commands whose whole behaviour is "you move through rows and choose one" were the ones
# no real-console run could reach.
SPECIAL_KEYS = {
    "up": 0x26, "down": 0x28, "left": 0x25, "right": 0x27,
    "enter": 0x0D, "tab": 0x09, "esc": 0x1B, "backspace": 0x08,
    "home": 0x24, "end": 0x23, "pgup": 0x21, "pgdn": 0x22,
    "space": 0x20, "delete": 0x2E,
}


def parse_key_sequence(text):
    """Turn `<up><down><enter>/tree` into a list of what to type.

    `<name>` is a special key from `SPECIAL_KEYS`; anything else is literal text. The
    angle-bracket syntax is deliberately not something a slash command can contain, so
    `--keys` stays usable for plain text without quoting rules.
    """
    out, i, buf = [], 0, ""
    while i < len(text):
        if text[i] == "<":
            end = text.find(">", i)
            if end > i:
                name = text[i + 1:end].lower()
                if name in SPECIAL_KEYS:
                    if buf:
                        out.append(("text", buf))
                        buf = ""
                    out.append(("key", name))
                    i = end + 1
                    continue
        buf += text[i]
        i += 1
    if buf:
        out.append(("text", buf))
    return out


def send_sequence(in_handle, text):
    """Type a key sequence: literal text plus `<up>`-style special keys.

    Each item produces a key-down and a key-up record, which is what a real keyboard does
    and what `type_text` already did for characters.
    """
    items = parse_key_sequence(text)
    total = sum(2 if kind == "key" else 2 * len(value) for kind, value in items)
    records = (INPUT_RECORD * max(1, total))()
    i = 0
    for kind, value in items:
        if kind == "key":
            for down in (True, False):
                rec = records[i]
                rec.EventType = KEY_EVENT
                rec.Event.KeyEvent.bKeyDown = 1 if down else 0
                rec.Event.KeyEvent.wRepeatCount = 1
                rec.Event.KeyEvent.wVirtualKeyCode = SPECIAL_KEYS[value]
                rec.Event.KeyEvent.wVirtualScanCode = 0
                rec.Event.KeyEvent.UnicodeChar = "\0"
                rec.Event.KeyEvent.dwControlKeyState = 0
                i += 1
        else:
            for ch in value:
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
    return {"ok": bool(ok), "written": written.value, "err": ctypes.get_last_error(),
            "items": [list(x) for x in items]}


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
        if a.startswith("--") and a not in (
            "--early",
            "--late",
            "--maximize",
            "--at",
            "--size",
            "--enter",
            "--settle",
            "--keys",
            "--submit",
        ):
            sys.stderr.write(f"console-probe: unknown option {a!r}\n")
            sys.stderr.write(
                "options: --early --late --maximize --at SECONDS --size COLSxROWS"
                " --enter --settle SECONDS --keys SEQ --submit\n"
            "        SEQ is literal text plus <up> <down> <left> <right> <enter>\n"
            "        <tab> <esc> <home> <end> <pgup> <pgdn> <space> <backspace>\n"
            "        <delete>, e.g. \"<down><down><enter>\" (M176)\n"
            )
            return 2
    mode = "late"
    size = None
    maximize = False
    wait_s = 7.0
    press_enter = False
    submit_keys = False
    settle_s = 9.0
    keys = ""
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
        elif argv[i] == "--enter":
            # Submit what was typed. Without this the probe can only ever leave the
            # composer holding text, so the *one* state a user spends their time in —
            # after a turn has run, with real tokens in the header — was unreachable.
            # `type_text` already maps `\r` to VK 0x0D (see VK_BY_CHAR), which is the
            # keystroke a TUI's key handler actually recognises as Enter; a probe that
            # only wrote characters could type `/help` forever and never run it.
            press_enter = True
        elif argv[i] == "--submit":
            submit_keys = True
        elif argv[i] == "--keys":
            # Keys to send after submitting, to unblock whatever the turn is waiting on.
            i += 1
            keys = argv[i]
        elif argv[i] == "--settle":
            # Seconds to wait after submitting, before reading the screen. A faux
            # provider answers in well under a second; a real one does not, and this is
            # the knob that keeps the measurement about the header rather than about
            # how long a model took to answer.
            i += 1
            settle_s = float(argv[i])
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
        if press_enter:
            # Submit, then read the screen *again*. Everything the owner asked to see —
            # context used, cache hit rate, estimated cost — only has a value once a
            # reply has come back, so a probe that cannot press Enter can only ever
            # photograph the one state where those numbers are legitimately unknown.
            result["enter"] = type_text(inp, "\r")
            time.sleep(settle_s)
            result["after_turn"] = snapshot(out)
            result["turn_screen"] = screen_text(out, limit_rows=64)
            if keys:
                # Answer whatever the turn is blocked on. A `bash` call opens a permission
                # dialog and waits, so without this the probe photographs a turn that is
                # *parked*, not one that is *done* — and a header read while a dialog is up
                # says nothing about what the header says when the work has landed.
                result["keys"] = send_sequence(inp, keys)
                time.sleep(settle_s)
                result["after_keys"] = snapshot(out)
                result["settled_screen"] = screen_text(out, limit_rows=64)

    if mode == "late" and submit_keys and keys and not press_enter:
        # `--submit` types the keys into the composer and *then* presses Enter.
        #
        # This exists because `--keys` alone cannot: it only runs after `--enter` has
        # submitted a turn, and only to answer whatever that turn is blocked on. Asking it
        # to type a slash command therefore submits an empty line first and types the
        # command afterwards, leaving the popup open - which is a perfectly good photograph
        # of the wrong moment, and reads as "the command did nothing" if taken at face
        # value. A command probe has to type and then submit.
        result["submit_typed"] = send_sequence(inp, keys)
        time.sleep(2.0)
        result["submit_screen"] = screen_text(out, limit_rows=64)
        result["submit_enter"] = type_text(inp, "\r")
        time.sleep(settle_s)
        result["after_submit"] = snapshot(out)
        result["settled_screen"] = screen_text(out, limit_rows=64)

    kill(shell.pid)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
