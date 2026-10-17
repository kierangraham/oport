# Changelog

All notable changes to oport are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and oport uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-10-17

### Added

- An Omarchy bar widget: a dot grid with one dot per dev server and a count,
  amber when a server uses too much memory.
- A panel with list, details and clean-up views, and keyboard control.
- Details for each server: project, framework, branch and worktree, folder,
  command, listening addresses, connections, 10-minute memory and CPU charts,
  and the process tree.
- Coding-agent detection for Claude Code, Codex, OpenCode, Gemini CLI, Amp,
  Aider, Goose and Crush, with **Resume** for Claude Code and Codex sessions.
- Memory-leak warnings for servers above a limit, or growing fast within a
  time window.
- Stop, restart and clean up, with a second click to confirm. Databases and
  container daemons are protected.
- The `oport` CLI: `list`, `stop`, `restart`, `open`, `cleanup` and `watch`,
  with JSON output, and an interactive terminal view.
- Settings in the bar's widget settings, shared with the CLI.
- An Arch `PKGBUILD` for the CLI, and `make install`, `make uninstall` and
  `make check`.

[0.1.0]: https://github.com/kierangraham/oport/releases/tag/v0.1.0
