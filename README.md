# bromine (Rust)

A terminal UI for the Hunter Schools Blackbaud LMS (mouse and keyboard), and the Rust client library it is built on.
Linux and macOS.

## Install

```sh
git clone https://github.com/TogiFerretFerret/blackbaud-tui
cd blackbaud-tui
cargo install --path bromine-tui   # needs a Rust toolchain: https://rustup.rs
bromine-tui
```

Signing in opens your browser (Google). That needs one small userscript, which hands the sign-in result back to
the app on `127.0.0.1`: install [`capture.user.js`](https://raw.githubusercontent.com/TogiFerretFerret/blackbaud-tui/main/capture.user.js)
in Tampermonkey or Violentmonkey (click the link and the manager offers to install it). The first run asks for
your school username (the part of your email before the `@`, or set `BB_USERNAME`). After that the saved
session refreshes itself and you only sign in again when it fully lapses. It is saved, mode 0600, in
`~/.bromine-mcp/session.json`.

Try it without an account: `bromine-tui --demo`.

| crate | what |
|---|---|
| `bromine` | the library: sign-in, session storage, and a typed API (classes, assignment center, details, status, personal tasks, feed, topics, announcements, files, grades) |
| `bromine-tui` | the TUI (the `bromine-tui` binary) |

```sh
cargo run -p bromine-tui            # your real account (signs in through the browser when needed)
cargo run -p bromine-tui -- --demo  # a fake in-memory LMS: no network, no sign-in
```

## The TUI

Four views:

- **Assignments**: the assignment center grouped by due date, with a detail pane and status buttons.
- **Classes**: pick a class, then switch between its **About**, **Work**, **Topics**, **News**, **Files**,
  **Grades** and **People**. Each is a list with a preview underneath, fetched the first time you open it.
- **Feed**: teacher posts.
- **Files**: the files the school has published to you.

| | |
|---|---|
| `↑ ↓ j k`, `PgUp PgDn`, `g G` | move (in Classes, in the focused column) |
| `1 2 3 4`, `Tab` | switch view |
| `→ l`, `← h Esc` | Classes: into the class's lists / back to the class list |
| `< >` (or `, .`) | Classes: previous / next section |
| `Enter` | Classes: open (Work: show in Assignments; Topics, Files: open the file). Feed: open the post's assignment. Files: open |
| `f` | Classes: show only this class in Assignments |
| `t` `i` `c` | set the selected item to to do / in progress / completed |
| `n` `e` `l` `x` | personal tasks: new; edit text and due date; link to an assignment (or unlink); delete (asks first) |
| `o` `d` `P` `a` | the file shown: open it / save it to Downloads / print it (PDF, double-sided, asks first) / next file |
| `/`, `p`, `h` | filter by name or class; show past work; hide completed |
| `[ ]`, `Ctrl-u/d` | scroll the detail pane |
| `r`, `?`, `q` | refresh; help; quit |

Files are saved to `$BROMINE_DOWNLOADS`, else `~/Downloads`, and never overwrite: a second copy becomes
`name (2).ext`. Open uses `xdg-open`; print uses `lpr -o sides=two-sided-long-edge`.

**Mouse:** click tabs, section tabs, rows, files and buttons; the wheel scrolls the list or the detail pane under
the cursor; dialogs have clickable buttons.

Status changes show instantly and are undone if the LMS refuses (for example "completed" on a
file-submission assignment you have not submitted, which the library never allows).

If the saved session has lapsed, the TUI refreshes it silently, and failing that opens the browser sign-in
and shows a dialog while it waits. Sign-in needs `capture.user.js` installed in your
userscript manager.

## The library

```rust
use bromine::{Client, Endpoints, SessionStore};

let session = SessionStore::default().refresh(&Endpoints::default())?; // headless; Err(LoginRequired) if lapsed
let client = Client::new(&session.tt)?;
for item in client.assignment_center()? {
    println!("{} {} {:?}", item.status.label(), item.name, item.due);
}
```

Behaviour worth knowing, all learned from the live LMS:

- An expired `t` shows up as a redirect, a 401, a **403** (asked of `userstatus.TokenValid` to tell it from a
  real refusal), or a 200 with an *anonymous* context. All of those are `Error::SessionExpired`; a genuine
  refusal (a class whose gradebook is not shared) is `Error::Refused`.
- The assignment center reports the wrong status for items due today, so `assignment_center()` replaces it
  with each assignment's own record.
- `set_status` refuses "completed" on a file-submission assignment with no submission, and only
  to do / in progress / completed can be set. Task edits and deletes refuse ids that are not your own tasks.
- `Client::download` only fetches `/ftpimages/…` and `/podium/UserFiles/Download.aspx` on the LMS host.
- Credentials and tokens never appear in `Debug` output; the session file is written 0600 in a 0700 directory.

`examples/smoke.rs` is a read-only check against the live LMS (`cargo run -p bromine --example smoke`).

## Layout of the TUI

State and input handling (`app.rs`) are pure and know nothing of the terminal or network. A background
worker thread (`worker.rs`) owns the session and every request, so the UI never blocks and re-authentication
is transparent; the two talk over channels (`Cmd` out, `Event` back). The renderer (`ui.rs`) records every
clickable region as it draws, so mouse handling always matches what is on screen. `demo.rs` is the fake LMS.

## Tests

```sh
cargo test --workspace     # unit + integration tests; the LMS is faked with mock servers
cargo clippy --workspace --all-targets
cargo build -p bromine-tui && uv run --with pyte python scripts/pty_demo.py   # the real binary in a pty
```

The library tests run the whole login flow (including the real local callback listener), expiry
detection, tasks and downloads against mock servers shaped from real captures. The TUI tests render to a
test backend and drive keys and clicks through the same handlers the app uses.

## Not done yet

Submitting work, grades beyond the per-assignment scores the LMS shows students, and opening the links inside
topics (they are shown as text; your terminal can open them).
