"""Drive the real TUI binary (in --demo mode) through a pseudo-terminal: real keystrokes and SGR mouse
escape sequences in, the screen a terminal emulator would show out. Catches what unit tests cannot
(raw mode, mouse reporting, resize, clean exit).

    cargo build -p bromine-tui
    uv run --with pyte python rust/scripts/pty_demo.py
"""
import os, pty, select, struct, fcntl, termios, sys, time, signal
import pyte

COLS, ROWS = 120, 34
BIN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "debug", "bromine-tui")
pid, fd = pty.fork()
if pid == 0:
    os.environ.update(TERM="xterm-256color", COLUMNS=str(COLS), LINES=str(ROWS))
    os.execv(BIN, [BIN, "--demo"])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
os.kill(pid, signal.SIGWINCH)

screen = pyte.Screen(COLS, ROWS); stream = pyte.ByteStream(screen)
raw = bytearray()

def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try: data = os.read(fd, 65536)
            except OSError: return False
            if not data: return False
            raw.extend(data); stream.feed(data)
    return True

def send(b, wait=0.5): os.write(fd, b if isinstance(b, bytes) else b.encode()); pump(wait)
def click(x, y, wait=0.5): send(f"\x1b[<0;{x+1};{y+1}M\x1b[<0;{x+1};{y+1}m", wait)   # 0-based -> 1-based press+release
def wheel(x, y, down, wait=0.4): send(f"\x1b[<{65 if down else 64};{x+1};{y+1}M", wait)
def show(title):
    print(f"\n===== {title} =====")
    for line in screen.display: print(line.rstrip())
def find(text):
    for y, line in enumerate(screen.display):
        x = line.find(text)
        if x >= 0: return x, y
    raise SystemExit(f"could not find {text!r} on screen")

pump(1.2)
show("1. start (demo data loaded by the worker)")
assert "bromine" in screen.display[0] and "Due today" in "\n".join(screen.display)

x, y = find("2 Classes"); click(x + 2, y); pump(0.6)
show("2. clicked the 'Classes' tab")
assert "Ms. Alvarez" in "\n".join(screen.display) and "Topics" in "\n".join(screen.display)

send("1")
x, y = find("Pre-Lab: Heat"); click(x + 3, y); 
show("3. clicked an assignment row")
x, y = find("In progress"); click(x + 2, y, 0.7)
show("4. clicked the 'In progress' button")
assert "Marked as in progress" in "\n".join(screen.display), "the success toast should show"

send("n", 0.4); send("buy glue")
show("5. new-task dialog (typed 'buy glue')")
assert "New task" in "\n".join(screen.display)
send("\r", 0.9)
show("6. after Enter: the task was created and the list refreshed")
assert "buy glue" in "\n".join(screen.display)

x, y = find("buy glue"); click(x + 1, y)
send("x", 0.4)
show("7. pressed x on the new task: confirm dialog")
assert "Delete task?" in "\n".join(screen.display)
x, y = find("Delete   "); click(x + 2, y, 0.9)
show("8. clicked the Delete button")
assert "buy glue" not in "\n".join(screen.display) and "Task deleted" in "\n".join(screen.display)

send("n", 0.4); send("paint"); send("\r", 0.9)
x, y = find("paint"); click(x + 1, y)
send("e", 0.4); send("ing"); send("\r", 0.9)
show("8b. edited the task's text with e")
assert "painting" in "\n".join(screen.display) and "Task updated" in "\n".join(screen.display)
send("l", 0.4); send("unit 3", 0.4)
show("8c. link picker (typed 'unit 3')")
assert "Link task to" in "\n".join(screen.display)
send("\x1b[B", 0.2); send("\r", 0.9)
show("8d. linked")
assert "Personal task \u00b7 SPANISH III" in "\n".join(screen.display)

send("3", 0.5)
show("9. the Feed tab")
assert "Socratic Seminar" in "\n".join(screen.display)
wheel(30, 10, True); wheel(90, 10, True)

send("2", 0.6)
x, y = find("Topics"); click(x + 1, y, 0.8)
show("9b. Classes > Topics (clicked the section tab)")
assert "Unit 2: Reactions" in "\n".join(screen.display) and "Reading guide" in "\n".join(screen.display)
x, y = find("Grades"); click(x + 1, y, 0.8)
show("9c. Classes > Grades")
assert "Quarter 1" in "\n".join(screen.display)
x, y = find("Lab 1"); click(x + 1, y, 0.5)
assert "9 / 10" in "\n".join(screen.display)
send("4", 0.8)
show("9d. Files tab")
assert "Fall schedule.pdf" in "\n".join(screen.display)
send("\r", 0.9)
assert "Opened Fall schedule.pdf" in "\n".join(screen.display)

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 12, 50, 0, 0)); screen.resize(12, 50); os.kill(pid, signal.SIGWINCH); pump(0.6)
show("10. resized to 50x12")

send("?", 0.5)
show("11. help (small terminal)")
send("z", 0.3)
send("q", 0.5)
pump(0.5)
_, status = os.waitpid(pid, 0)
print("\nexit status:", os.waitstatus_to_exitcode(status))
print("mouse reporting disabled on exit:", any(m in raw for m in (b"\x1b[?1000l", b"\x1b[?1002l", b"\x1b[?1003l", b"\x1b[?1006l")))
print("left the alternate screen on exit:", b"\x1b[?1049l" in raw)
