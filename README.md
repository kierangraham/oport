# oport

Every dev server on this machine, in the Omarchy bar. A port of
[WhatThePort](https://whattheport.dev/) for Omarchy: the port, project,
branch, uptime, memory and CPU of each local server, which coding-agent
session started it, memory-leak warnings, and one-click stop, restart and
clean up. Rust scans `/proc`; Quickshell and QML render the bar widget and
panel.

![oport panel listing four dev servers, one with a memory warning](docs/list.webp)

## Requirements

- Omarchy with Quickshell 0.3 and the shell plugin API
- A stable Rust toolchain, 1.95 or newer, with Cargo, to build the `oport` CLI
- Node.js, for development checks only

## Install

The plugin runs the `oport` CLI, so install that first. `--root ~/.local`
puts it in `~/.local/bin`, where the plugin looks for it:

```bash
cargo install --git https://github.com/kierangraham/oport --locked --root ~/.local
```

Then add the plugin. Omarchy clones the repository and enables the widget:

```bash
omarchy plugin add https://github.com/kierangraham/oport --enable
```

To update:

```bash
cargo install --git https://github.com/kierangraham/oport --locked --root ~/.local
omarchy plugin update kierangraham.oport
```

For a keyboard shortcut like WhatThePort's ⌥⌘P, add a Hyprland binding:

```
bind = SUPER ALT, P, exec, omarchy-shell oport toggle
```

## Usage

The bar shows a 3×3 dot grid with one lit dot per server (up to nine) and the
count. It turns amber when a server is using too much memory. Hover for a
summary. Click to open the panel; right-click to go straight to Clean up.

- **List.** One row per server, coloured consistently by port. Hover a row to
  open it in the browser or stop it. A memory bar splits RAM into dev servers,
  other apps and free memory.
- **Details.** Click a row: the agent session and its host app, branch and
  worktree, folder, command, listening addresses, connections, 10-minute
  memory and CPU charts, and every process in the tree. Actions: Open,
  Restart, Stop, Editor (`omarchy-launch-editor`), Terminal, and **Resume**,
  which reopens the Claude Code or Codex conversation in a terminal in the
  server's folder.
- **Clean up.** Checkboxes, with servers whose folder was deleted (a removed
  worktree) or that have been idle for hours preselected. Stop them together.

![Server details with a memory-growth warning and charts](docs/leak.webp)

Stop, Restart and Clean up ask for a second click. Keys in the panel: arrows or
`j`/`k` select, Enter opens details, `o` open, `s` stop, `r` restart, `e`
editor, `t` terminal, `c` clean up, Escape goes back or closes.

oport never sends desktop notifications. A memory warning only turns the bar
icon amber and marks the server in the panel.

```bash
omarchy-shell oport open | close | toggle
omarchy-shell oport status     # {"ready":true,"servers":3,"leaking":0,...}
```

## Terminal

`oport` in a terminal opens an interactive view with the same list, details
and clean up. It supports mouse clicks and scrolling; press `?` for keys.

```bash
oport list                 # table, then exit
oport list --json          # for scripts and agents
oport stop :3000 :5173     # a port, pid:N, or an id from --json
oport restart 3000         # stop, then rerun its command, detached
oport open 3000
oport cleanup --dry-run    # servers whose folder was deleted
oport watch                # one JSON snapshot per line (the bar's data source)
```

`--all` includes every listening process rather than only dev servers.
Restarted servers run in their own session and log to
`~/.local/state/oport/logs/`.

## How it works

- **Discovery.** Listening TCP sockets come from `/proc/net/tcp` and `tcp6`;
  socket inodes are matched to processes through `/proc/<pid>/fd`. Only your
  own processes can be inspected, so root-owned listeners such as
  `docker-proxy` are not shown. The scan runs every two seconds.
- **One server per process tree.** From the listening process, oport climbs
  through launchers that only wrap it (`npm run dev`, `pnpm`, `sh -c`,
  `cargo watch`, reloaders like Django's) but never into an interactive shell,
  terminal, tmux or coding agent. A shared task runner such as `turbo` stays
  out when it hosts several servers, so siblings in a monorepo stay separate.
  A process serving several ports is one entry with all of them.
- **Dev servers.** A listener counts when its folder is inside a project
  under your home directory (package.json, Cargo.toml, go.mod, pyproject.toml,
  Gemfile, mix.exs, a git repository and others) or it runs a dev runtime
  such as node, bun, deno, python or ruby. Others are counted as hidden.
- **Project, framework, branch.** The nearest manifest names the project
  (monorepo apps keep their own name, with the repository beside it). The
  framework comes from the command line, then package.json dependencies.
  The branch is read from `.git/HEAD`, including linked worktrees.
- **Agents.** Claude Code passes `CLAUDE_CODE_SESSION_ID` and Codex passes
  `CODEX_THREAD_ID` to the commands they run, so the server's environment
  gives the exact session to resume. OpenCode, Gemini CLI, Amp, Aider, Goose
  and Crush are recognised as ancestor processes. T3 Code, Zed and VS Code
  are reported as the host app.
- **Memory and CPU.** Memory is proportional set size (PSS) summed over the
  process tree, so shared libraries are not counted once per process. CPU
  is summed over the tree, with one logical core as 100%.
- **Leaks.** A server is flagged above 2 GiB, or when it grows by 500 MiB
  within 10 minutes. Two minutes of history are required first, so startup
  allocation is not a leak. Leaking servers are never preselected for clean up.
- **Idle.** A server is idle after three hours with under 2% CPU and no
  established connections. An open browser tab with a hot-reload socket
  keeps a server active.
- **Stop.** SIGTERM to the server's tree, children first, then SIGKILL for
  anything still running after five seconds. Every pid is checked against its
  start time, so a recycled pid is never signalled. Databases and container
  daemons are protected: they are never preselected and the CLI needs
  `--force`.

## Settings

The bar's widget settings are stored on the widget's entry in
`~/.config/omarchy/shell.json`. The CLI and TUI read the same entry, and
flags such as `--memory-limit`, `--ports` and `--idle-hours` override it.

| Key | Default | |
| --- | --- | --- |
| `memoryLimitMb` | 2048 | Warn above this; 0 turns it off |
| `growthMb`, `windowMinutes` | 500, 10 | Warn on this much growth within the window |
| `idleHours` | 3 | 0 turns idle detection off |
| `graceSeconds` | 5 | Delay before force-quitting |
| `portMin`, `portMax` | 1024, 32767 | Ports scanned; the upper default skips ephemeral ports |
| `protected` | postgres, redis-server, … | Comma-separated process names |
| `ignored` | | Process or project names to leave out |
| `showAll` | false | Every listening process, not only dev servers |
| `showCount`, `hideWhenEmpty` | true, false | Bar appearance |

## Development

```bash
./tests/run                 # fmt, unit tests, clippy, release build, manifest, model tests
./tests/qml-smoke           # loads the plugin on Wayland with every window hidden
./tests/qml-smoke --render  # offscreen renders of each view into tests/output/
cargo run -- list
```

Both QML runs start a throwaway `npm run dev` project on port 5199 and drive
the list, details and clean-up views against the live scanner. Layer-shell
windows cannot load offscreen, so `--render` substitutes `tests/shim/` for
Omarchy's KeyboardPanel; every other component is the installed one.

The plugin's QML lives in `plugin/`, with `manifest.json` at the repository
root so `omarchy plugin add` can install from git. To try local changes,
`./scripts/install` builds `oport` into `~/.local/bin`, copies the plugin into
`~/.config/omarchy/plugins/kierangraham.oport`, backs up `shell.json` and
enables the widget. It refuses to replace a plugin directory it did not
create, so remove a git-installed copy first.

## Not yet

WhatThePort features not ported: automatic clean up (only Ask mode exists),
pull-request and Vercel preview links, idle detection in one-shot CLI commands
(`cleanup` uses deleted folders only), and translations.

## Remove

```bash
omarchy plugin disable kierangraham.oport
omarchy plugin remove kierangraham.oport --yes
cargo uninstall oport --root ~/.local
```

## Licence

MIT. See [LICENSE](LICENSE). An independent project, unaffiliated with
WhatThePort; only its public website and README were consulted.

---

[kierangraham.com](https://kierangraham.com)
