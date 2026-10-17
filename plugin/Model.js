function emptySnapshot() {
  return {
    schema_version: 1, timestamp: 0, tracked: true, servers: [], hidden: 0,
    memory: {total: 0, available: 0, servers: 0},
    limits: {memory_limit: 0, growth: 0, window_minutes: 10, idle_hours: 3, port_min: 1024, port_max: 32767}
  }
}

function decode(line) {
  var value = JSON.parse(line)
  if (!value || value.schema_version !== 1 || !Array.isArray(value.servers) || !value.memory || !value.limits)
    throw new Error("Unsupported oport stream")
  value.servers.forEach(function(server) {
    if (typeof server.id !== "string" || !Array.isArray(server.ports) || !server.ports.length
        || !Array.isArray(server.processes) || !server.history
        || [server.memory, server.cpu, server.uptime].some(function(n) { return typeof n !== "number" || !isFinite(n) || n < 0 }))
      throw new Error("Invalid server entry")
  })
  return value
}

function bytes(value) {
  var n = Math.max(0, Number(value) || 0)
  var units = ["B", "KiB", "MiB", "GiB", "TiB"]
  var unit = 0
  while (n >= 1024 && unit < units.length - 1) { n /= 1024; unit++ }
  return (unit === 0 ? Math.round(n) : n.toFixed(n < 10 ? 1 : 0)) + " " + units[unit]
}

function duration(seconds) {
  seconds = Math.max(0, Math.floor(Number(seconds) || 0))
  var days = Math.floor(seconds / 86400), hours = Math.floor(seconds % 86400 / 3600), minutes = Math.floor(seconds % 3600 / 60)
  if (days) return days + "d " + hours + "h"
  if (hours) return hours + "h " + minutes + "m"
  if (minutes) return minutes + "m"
  return seconds + "s"
}

function cpu(value) {
  var n = Math.max(0, Number(value) || 0)
  return (n >= 10 ? Math.round(n) : n.toFixed(1)) + "%"
}

function ports(server) {
  return server.ports.map(function(p) { return ":" + p }).join(" ")
}

// Each port keeps the same colour across sessions and views.
var palette = ["#7aa2f7", "#9ece6a", "#bb9af7", "#7dcfff", "#e0af68", "#f7768e", "#73daca", "#ff9e64", "#2ac3de", "#c0caf5"]
function portColor(port) {
  var n = Number(port) || 0
  var hash = ((n * 2654435761) >>> 0) % palette.length
  return palette[hash]
}

function home(path, homeDir) {
  path = String(path || "")
  return homeDir && path.indexOf(homeDir + "/") === 0 ? "~" + path.slice(homeDir.length) : path
}

// Secondary line of a server row: framework, agent and state, most urgent first.
function detail(server) {
  var parts = []
  if (server.leak) parts.push(server.leak)
  else if (server.cwd_missing) parts.push("folder deleted")
  else if (server.idle) parts.push("idle " + duration(server.idle_seconds))
  parts.push(server.framework || server.runtime)
  if (server.agent) parts.push(server.agent.label)
  parts.push("up " + duration(server.uptime))
  return parts.join("  ·  ")
}

function title(server) {
  return server.name + (server.branch ? "  " + server.branch : "")
}

// Fractions of physical RAM for the memory bar. Servers are a subset of used memory.
function memoryShares(memory) {
  var total = Number(memory.total) || 0
  if (total <= 0) return {servers: 0, other: 0, free: 1}
  var used = Math.max(0, total - (Number(memory.available) || 0))
  var servers = Math.min(used, Math.max(0, Number(memory.servers) || 0))
  return {servers: servers / total, other: (used - servers) / total, free: (total - used) / total}
}

function cleanupSelection(servers) {
  var selected = {}
  servers.forEach(function(server) { if (server.cleanup) selected[server.id] = true })
  return selected
}

function selectedCount(selected, servers) {
  return servers.filter(function(server) { return selected[server.id] && !server.protected }).length
}

function tooltip(snapshot, stale) {
  if (stale) return "oport · waiting for the scanner"
  var servers = snapshot.servers
  if (!servers.length) return "No dev servers listening"
  var lines = [servers.length + (servers.length === 1 ? " dev server · " : " dev servers · ") + bytes(snapshot.memory.servers)]
  servers.slice(0, 6).forEach(function(server) {
    lines.push(ports(server) + "  " + server.name + (server.leak ? "  ⚠ " + server.leak : ""))
  })
  if (servers.length > 6) lines.push("…and " + (servers.length - 6) + " more")
  return lines.join("\n")
}

if (typeof module !== "undefined") module.exports = {
  emptySnapshot: emptySnapshot, decode: decode, bytes: bytes, duration: duration, cpu: cpu, ports: ports,
  portColor: portColor, home: home, detail: detail, title: title, memoryShares: memoryShares,
  cleanupSelection: cleanupSelection, selectedCount: selectedCount, tooltip: tooltip
}
