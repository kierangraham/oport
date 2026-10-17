import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import "Model.js" as Model

Item {
  id: root
  property var shell: null
  property var snapshot: Model.emptySnapshot()
  property double receivedAt: 0
  property double clock: Date.now()
  property string error: ""
  property string diagnostics: ""
  readonly property bool stale: receivedAt === 0 || clock - receivedAt > 8000
  readonly property bool ready: receivedAt > 0
  readonly property var servers: snapshot.servers
  readonly property int leakCount: servers.filter(function(s) { return !!s.leak }).length
  readonly property string homeDir: Quickshell.env("HOME") || ""
  readonly property string binary: Quickshell.env("OPORT_BIN") || homeDir + "/.local/bin/oport"

  function ingest(line) {
    try {
      snapshot = Model.decode(line)
      receivedAt = Date.now()
      clock = receivedAt
      error = ""
      diagnostics = ""
    } catch (e) {
      error = "Could not read servers: " + String(e)
    }
  }

  function run(args) { Util.execArgv([binary].concat(args)) }
  function stop(server) { run(["stop", server.id, "--force"]) }
  function stopMany(list) { if (list.length) run(["stop", "--force"].concat(list.map(function(s) { return s.id }))) }
  function restart(server) { run(["restart", server.id, "--force"]) }
  function open(server) { Util.execArgv(["xdg-open", server.url]) }
  function openFolder(server) { Util.execArgv(["omarchy-launch-editor", server.project_root || server.cwd]) }
  function openTerminal(server, command) {
    var argv = ["setsid", "uwsm-app", "--", "xdg-terminal-exec", "--dir=" + server.cwd]
    // The resume command is built by oport from a validated session id.
    if (command) argv = argv.concat(["bash", "-lc", command + "; exec bash"])
    Util.execArgv(argv)
  }

  function statusJson() {
    return JSON.stringify({ready: ready, stale: stale, error: error, servers: servers.length, leaking: leakCount})
  }

  Timer { interval: 1000; running: true; repeat: true; onTriggered: root.clock = Date.now() }
  Timer {
    id: retry
    interval: 5000
    onTriggered: scanner.running = true
  }
  Process {
    id: scanner
    command: [root.binary, "watch"]
    running: true
    stdout: SplitParser { onRead: function(data) { root.ingest(data) } }
    stderr: SplitParser { onRead: function(data) { root.diagnostics = String(data).slice(0, 240) } }
    onExited: function(code, status) {
      root.error = root.diagnostics || "Scanner stopped; retrying…"
      retry.restart()
    }
    onRunningChanged: if (!running && root.receivedAt === 0) {
      root.error = "Waiting for " + root.binary + "…"
      retry.restart()
    }
  }
  IpcHandler {
    target: "oport"
    function status(): string { return root.statusJson() }
    function list(): string { return JSON.stringify(root.snapshot) }
    function open(): void { if (root.shell) root.shell.summon("kierangraham.oport", "{}") }
    function close(): void { if (root.shell) root.shell.hide("kierangraham.oport") }
    function toggle(): void { if (root.shell) root.shell.toggle("kierangraham.oport", "{}") }
  }
}
