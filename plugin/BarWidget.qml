import QtQuick
import QtQuick.Controls as QQC
import Quickshell
import qs.Ui
import qs.Commons
import "Model.js" as Model

Panel {
  id: root
  moduleName: "kierangraham.oport"
  manageIpc: false
  readonly property var service: bar && bar.shell ? bar.shell.serviceFor(moduleName) : null
  readonly property var snapshot: service ? service.snapshot : Model.emptySnapshot()
  readonly property var servers: snapshot.servers
  readonly property bool ready: !!service && service.ready
  readonly property bool stale: !service || service.stale
  readonly property int leakCount: service ? service.leakCount : 0
  readonly property string homeDir: service ? service.homeDir : ""
  readonly property color ink: Color.popups.text
  readonly property color subdued: Qt.rgba(ink.r, ink.g, ink.b, 0.55)
  readonly property color faint: Qt.rgba(ink.r, ink.g, ink.b, 0.08)
  readonly property color warning: "#e0af68"
  readonly property color serverColor: Color.accent
  readonly property color otherColor: Qt.rgba(ink.r, ink.g, ink.b, 0.35)
  readonly property color statusInk: bar ? bar.barForeground : Color.foreground
  readonly property bool verticalBar: bar && bar.vertical
  readonly property bool showCount: setting("showCount", true)
  readonly property bool hideWhenEmpty: setting("hideWhenEmpty", false)
  readonly property bool concealed: hideWhenEmpty && ready && !stale && servers.length === 0 && !opened

  // "list", "details" or "cleanup".
  property string view: "list"
  property int cursor: 0
  property string detailId: ""
  property var selected: ({})
  property string armed: ""
  // Exposed for the smoke test's renders.
  readonly property Item panelContents: contents
  readonly property var detail: {
    for (var i = 0; i < servers.length; i++) if (servers[i].id === detailId) return servers[i]
    return null
  }

  visible: !concealed
  implicitWidth: concealed ? 0 : button.implicitWidth
  implicitHeight: bar ? bar.barSize : Style.bar.sizeHorizontal

  onOpenedChanged: if (!opened) { view = "list"; armed = "" }
  onServersChanged: cursor = Math.max(0, Math.min(cursor, servers.length - 1))

  function showList() { view = "list"; armed = ""; scroll.contentItem.contentY = 0 }
  function showDetails(id) { detailId = id; view = "details"; armed = ""; scroll.contentItem.contentY = 0 }
  function startCleanup() { selected = Model.cleanupSelection(servers); view = "cleanup"; armed = ""; cursor = 0 }
  function toggleSelected(server) {
    if (!server || server.protected) return
    var next = Object.assign({}, selected)
    if (next[server.id]) delete next[server.id]; else next[server.id] = true
    selected = next
  }
  function stopSelected() {
    var list = servers.filter(function(s) { return selected[s.id] && !s.protected })
    service.stopMany(list)
    showList()
  }
  // Destructive actions need a second click within a few seconds.
  function confirm(key, action) {
    if (armed === key) { armed = ""; action() }
    else { armed = key; disarm.restart() }
  }
  function moveCursor(dy) {
    if (!servers.length) return
    cursor = Math.max(0, Math.min(servers.length - 1, cursor + dy))
    var row = rows.itemAt(cursor) || cleanupRows.itemAt(cursor)
    if (row) {
      var top = row.mapToItem(contents, 0, 0).y
      var flick = scroll.contentItem
      if (top < flick.contentY) flick.contentY = top
      else if (top + row.height > flick.contentY + scroll.height) flick.contentY = top + row.height - scroll.height
    }
  }

  Timer { id: disarm; interval: 3500; onTriggered: root.armed = "" }

  WidgetButton {
    id: button
    anchors.centerIn: parent
    bar: root.bar
    labelVisible: false
    hasVisualContent: true
    dimmed: root.stale
    fixedWidth: root.verticalBar ? (root.bar ? root.bar.barSize : Style.bar.sizeVertical) : content.implicitWidth + Style.space(12)
    tooltipText: Model.tooltip(root.snapshot, root.stale)
    onPressed: function(mouseButton) {
      if (mouseButton === Qt.RightButton && root.servers.length) { root.startCleanup(); root.open(); return }
      root.toggle()
    }
    Row {
      id: content
      anchors.centerIn: parent
      spacing: Style.space(4)
      PortGlyph {
        width: Style.space(12); height: width
        anchors.verticalCenter: parent.verticalCenter
        count: root.servers.length
        foreground: root.statusInk
        alert: root.warning
        alerting: root.leakCount > 0
      }
      Text {
        visible: root.showCount && !root.verticalBar && root.servers.length > 0
        anchors.verticalCenter: parent.verticalCenter
        text: String(root.servers.length)
        color: root.leakCount > 0 ? root.warning : root.statusInk
        font.family: root.bar ? root.bar.fontFamily : Style.font.family
        font.pixelSize: Style.space(11)
        font.weight: Font.DemiBold
        renderType: Text.NativeRendering
      }
    }
  }

  KeyboardPanel {
    id: popup
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keys
    contentWidth: popup.fittedContentWidth(Style.space(470))
    contentHeight: popup.fittedContentHeight(contents.implicitHeight, Style.space(680))

    PanelKeyCatcher {
      id: keys
      anchors.fill: parent
      onCloseRequested: root.view === "list" ? root.close() : root.showList()
      onMoveRequested: function(dx, dy) {
        if (root.view === "details") {
          if (dx < 0) root.showList()
          else scroll.contentItem.contentY = Math.max(0, Math.min(contents.height - scroll.height, scroll.contentItem.contentY + dy * Style.space(60)))
        } else if (dy) root.moveCursor(dy)
        else if (dx > 0 && root.view === "list" && root.servers[root.cursor]) root.showDetails(root.servers[root.cursor].id)
      }
      onActivateRequested: {
        var server = root.servers[root.cursor]
        if (root.view === "cleanup") root.toggleSelected(server)
        else if (root.view === "list" && server) root.showDetails(server.id)
      }
      onTextKey: function(text) {
        var server = root.view === "details" ? root.detail : root.servers[root.cursor]
        if (text === "c" && root.view !== "cleanup" && root.servers.length) root.startCleanup()
        else if (text === "s" && root.view === "cleanup") root.confirm("cleanup", root.stopSelected)
        else if (!server || root.view === "cleanup") return
        else if (text === "o") root.service.open(server)
        else if (text === "s") root.confirm("stop:" + server.id, function() { root.service.stop(server) })
        else if (text === "r") root.confirm("restart:" + server.id, function() { root.service.restart(server) })
        else if (text === "e") root.service.openFolder(server)
        else if (text === "t") root.service.openTerminal(server, "")
      }

      QQC.ScrollView {
        id: scroll
        anchors.fill: parent
        clip: true
        QQC.ScrollBar.horizontal.policy: QQC.ScrollBar.AlwaysOff
        QQC.ScrollBar.vertical.policy: QQC.ScrollBar.AsNeeded
        contentWidth: availableWidth

        Column {
          id: contents
          width: scroll.availableWidth
          spacing: Style.space(14)

          // Header
          Row {
            width: parent.width
            Column {
              width: parent.width - liveLabel.implicitWidth
              spacing: Style.space(3)
              Label { text: "oport"; font.pixelSize: Style.font.title; font.bold: true }
              Label {
                text: root.servers.length + (root.servers.length === 1 ? " dev server" : " dev servers")
                  + "  ·  " + Model.bytes(root.snapshot.memory.servers)
                  + (root.snapshot.hidden ? "  ·  " + root.snapshot.hidden + " other listener" + (root.snapshot.hidden === 1 ? "" : "s") + " hidden" : "")
                color: root.subdued; font.pixelSize: Style.font.caption
              }
            }
            Label {
              id: liveLabel
              text: root.stale ? "● OFFLINE" : root.leakCount ? "● " + root.leakCount + " WARNING" + (root.leakCount === 1 ? "" : "S") : "● LIVE"
              color: root.stale ? Color.urgent : root.leakCount ? root.warning : "#73daca"
              font.pixelSize: Style.font.caption; font.bold: true
              anchors.verticalCenter: parent.verticalCenter
            }
          }

          Label {
            visible: root.stale
            width: parent.width; wrapMode: Text.WordWrap
            text: root.service && root.service.error ? root.service.error : "Waiting for the oport scanner…"
            color: Color.urgent
          }

          // Whole-machine memory: dev servers, everything else, free.
          Column {
            width: parent.width
            spacing: Style.space(5)
            visible: root.view !== "details" && root.snapshot.memory.total > 0
            readonly property var shares: Model.memoryShares(root.snapshot.memory)
            Rectangle {
              width: parent.width; height: Style.space(6); radius: height / 2
              color: root.faint; clip: true
              Row {
                anchors.fill: parent
                // Keep a sliver visible: servers are often a tiny share of RAM.
                Rectangle {
                  readonly property real share: parent.parent.parent.shares.servers
                  width: share > 0 ? Math.max(Style.space(3), parent.width * share) : 0
                  height: parent.height; color: root.serverColor
                }
                Rectangle { width: parent.width * parent.parent.parent.shares.other; height: parent.height; color: root.otherColor }
              }
            }
            Row {
              spacing: Style.space(14)
              Legend { tint: root.serverColor; text: "Servers " + Model.bytes(root.snapshot.memory.servers) }
              Legend { tint: root.otherColor; text: "Other apps " + Model.bytes(Math.max(0, root.snapshot.memory.total - root.snapshot.memory.available - root.snapshot.memory.servers)) }
              Legend { tint: root.faint; text: "Free " + Model.bytes(root.snapshot.memory.available) }
            }
          }

          // ── List ──────────────────────────────────────────────
          Column {
            width: parent.width
            spacing: Style.space(2)
            visible: root.view === "list"
            Label {
              visible: root.ready && root.servers.length === 0
              text: "No dev servers listening on ports " + root.snapshot.limits.port_min + "–" + root.snapshot.limits.port_max
              color: root.subdued
            }
            Repeater {
              id: rows
              model: root.view === "list" ? root.servers : []
              ServerRow {
                required property var modelData
                required property int index
                server: modelData
                current: root.cursor === index
                onClicked: root.showDetails(modelData.id)
                onHoveredChanged: if (hovered) root.cursor = index
              }
            }
          }

          Row {
            width: parent.width
            spacing: Style.space(6)
            visible: root.view === "list" && root.servers.length > 0
            ActionButton { text: "Clean up…"; onClicked: root.startCleanup() }
            Label {
              width: parent.width - x
              anchors.verticalCenter: parent.verticalCenter
              horizontalAlignment: Text.AlignRight
              elide: Text.ElideLeft
              text: "Enter details · o open · s stop · c clean up"
              color: root.subdued; font.pixelSize: Style.font.caption
            }
          }

          // ── Clean up ──────────────────────────────────────────
          Column {
            width: parent.width
            spacing: Style.space(8)
            visible: root.view === "cleanup"
            SectionLabel { text: "CLEAN UP" }
            Label {
              width: parent.width; wrapMode: Text.WordWrap
              color: root.subdued; font.pixelSize: Style.font.bodySmall
              text: "Servers whose folder was deleted, or idle for " + root.snapshot.limits.idle_hours
                + " h without CPU or connections, are preselected. Protected processes cannot be selected."
            }
            Repeater {
              id: cleanupRows
              model: root.view === "cleanup" ? root.servers : []
              Rectangle {
                id: cleanRow
                required property var modelData
                required property int index
                width: contents.width
                height: cleanContent.implicitHeight + Style.space(10)
                radius: Style.space(4)
                color: root.cursor === index ? root.faint : "transparent"
                opacity: modelData.protected ? 0.5 : 1
                Row {
                  id: cleanContent
                  anchors.verticalCenter: parent.verticalCenter
                  x: Style.space(6)
                  width: parent.width - Style.space(12)
                  spacing: Style.space(8)
                  Rectangle {
                    width: Style.space(13); height: width; radius: Style.space(3)
                    anchors.verticalCenter: parent.verticalCenter
                    color: root.selected[cleanRow.modelData.id] ? root.serverColor : "transparent"
                    border.width: 1; border.color: root.subdued
                    Label { anchors.centerIn: parent; text: "✓"; visible: !!root.selected[cleanRow.modelData.id]; color: Color.background; font.pixelSize: Style.font.caption; font.bold: true }
                  }
                  Column {
                    width: parent.width - Style.space(21)
                    Label { text: Model.ports(cleanRow.modelData) + "  " + Model.title(cleanRow.modelData); width: parent.width; elide: Text.ElideRight }
                    Label {
                      width: parent.width; elide: Text.ElideRight
                      text: (cleanRow.modelData.protected ? "protected  ·  " : "") + Model.detail(cleanRow.modelData) + "  ·  " + Model.bytes(cleanRow.modelData.memory)
                      color: cleanRow.modelData.leak ? root.warning : root.subdued; font.pixelSize: Style.font.caption
                    }
                  }
                }
                MouseArea {
                  anchors.fill: parent
                  hoverEnabled: true
                  cursorShape: cleanRow.modelData.protected ? Qt.ArrowCursor : Qt.PointingHandCursor
                  onEntered: root.cursor = cleanRow.index
                  onClicked: root.toggleSelected(cleanRow.modelData)
                }
              }
            }
            Row {
              spacing: Style.space(6)
              readonly property int count: Model.selectedCount(root.selected, root.servers)
              ActionButton {
                text: root.armed === "cleanup" ? "Confirm: stop " + parent.count : "Stop " + parent.count + " selected"
                urgent: root.armed === "cleanup"
                enabled: parent.count > 0
                onClicked: root.confirm("cleanup", root.stopSelected)
              }
              ActionButton { text: "Cancel"; onClicked: root.showList() }
            }
          }

          // ── Details ───────────────────────────────────────────
          Column {
            width: parent.width
            spacing: Style.space(10)
            visible: root.view === "details"

            ActionButton { text: "‹ All servers"; onClicked: root.showList() }

            Label {
              visible: !root.detail
              text: "This server has stopped."
              color: root.subdued
            }

            Column {
              width: parent.width
              spacing: Style.space(10)
              visible: !!root.detail
              readonly property var server: root.detail || ({ports: [0], processes: [], history: {memory: [], cpu: []}})

              Row {
                width: parent.width
                spacing: Style.space(8)
                Rectangle { width: Style.space(10); height: width; radius: width / 2; color: Model.portColor(parent.parent.server.ports[0]); anchors.verticalCenter: parent.verticalCenter }
                Label { text: Model.ports(parent.parent.server); font.pixelSize: Style.font.heading; font.bold: true; color: Model.portColor(parent.parent.server.ports[0]) }
                Label {
                  width: parent.width - x
                  anchors.verticalCenter: parent.verticalCenter
                  text: parent.parent.server.name || ""
                  font.pixelSize: Style.font.subtitle; font.bold: true; elide: Text.ElideRight
                }
              }

              Label {
                visible: !!parent.server.leak
                width: parent.width; wrapMode: Text.WordWrap
                text: "⚠ " + (parent.server.leak || "")
                color: root.warning; font.bold: true
              }

              Flow {
                width: parent.width
                spacing: Style.space(6)
                readonly property var server: parent.server
                ActionButton { text: "Open"; onClicked: root.service.open(parent.server) }
                ActionButton {
                  text: root.armed === "restart:" + parent.server.id ? "Confirm restart" : "Restart"
                  urgent: root.armed === "restart:" + parent.server.id
                  onClicked: { var s = parent.server; root.confirm("restart:" + s.id, function() { root.service.restart(s) }) }
                }
                ActionButton {
                  text: root.armed === "stop:" + parent.server.id ? (parent.server.protected ? "Confirm: stop protected" : "Confirm stop") : "Stop"
                  urgent: root.armed === "stop:" + parent.server.id
                  onClicked: { var s = parent.server; root.confirm("stop:" + s.id, function() { root.service.stop(s); root.showList() }) }
                }
                ActionButton { text: "Editor"; onClicked: root.service.openFolder(parent.server) }
                ActionButton { text: "Terminal"; onClicked: root.service.openTerminal(parent.server, "") }
                ActionButton {
                  visible: !!(parent.server.agent && parent.server.agent.resume)
                  text: "Resume " + (parent.server.agent ? parent.server.agent.label : "")
                  onClicked: root.service.openTerminal(parent.server, parent.server.agent.resume)
                }
              }

              StatRow { label: "Framework"; value: parent.server.framework || parent.server.runtime || "" }
              StatRow {
                label: "Agent"
                value: parent.server.agent ? parent.server.agent.label + (parent.server.agent.host ? " in " + parent.server.agent.host : "") : "None detected"
              }
              StatRow {
                visible: !!parent.server.branch
                label: "Branch"
                value: (parent.server.branch || "") + (parent.server.worktree ? "  (worktree)" : "")
              }
              StatRow { label: "Up"; value: Model.duration(parent.server.uptime) + (parent.server.idle_seconds > 60 ? "  ·  idle " + Model.duration(parent.server.idle_seconds) : "") }
              StatRow { label: "Connections"; value: String(parent.server.connections || 0) }
              StatRow { label: "Listening on"; value: (parent.server.addresses || []).join("  ") }
              Column {
                width: parent.width
                spacing: Style.space(3)
                SectionLabel { text: parent.parent.server.cwd_missing ? "FOLDER · DELETED" : "FOLDER" }
                Label { width: parent.width; wrapMode: Text.WrapAnywhere; font.pixelSize: Style.font.bodySmall; text: Model.home(parent.parent.server.cwd, root.homeDir) }
              }
              Column {
                width: parent.width
                spacing: Style.space(3)
                SectionLabel { text: "COMMAND" }
                Label { width: parent.width; wrapMode: Text.WrapAnywhere; font.pixelSize: Style.font.bodySmall; maximumLineCount: 4; elide: Text.ElideRight; text: parent.parent.server.command || "" }
              }

              Chart {
                title: "MEMORY"
                value: Model.bytes(parent.server.memory) + (parent.server.growth > 0 ? "  ·  +" + Model.bytes(parent.server.growth) + " in " + root.snapshot.limits.window_minutes + " min" : "")
                values: parent.server.history.memory
                tint: parent.server.leak ? root.warning : root.serverColor
                maximum: Math.max(root.snapshot.limits.memory_limit, ...parent.server.history.memory) * 1.05
              }
              Chart {
                title: "CPU"
                value: Model.cpu(parent.server.cpu)
                values: parent.server.history.cpu
                tint: "#7dcfff"
                maximum: Math.max(25, ...parent.server.history.cpu) * 1.1
              }
              Label { text: "Last 10 minutes, summed across the server's process tree. 100% CPU is one core."; color: root.subdued; font.pixelSize: Style.font.caption; width: parent.width; wrapMode: Text.WordWrap }

              SectionLabel { text: "PROCESSES  ·  ● LISTENING" }
              Repeater {
                model: root.view === "details" && root.detail ? root.detail.processes : []
                StatRow {
                  required property var modelData
                  label: (modelData.listening ? "● " : "○ ") + modelData.name + " · " + modelData.pid
                  value: Model.bytes(modelData.memory) + "  " + Model.cpu(modelData.cpu)
                }
              }
            }
          }
        }
      }
    }
  }

  component Label: Text {
    textFormat: Text.PlainText
    color: root.ink
    font.family: root.bar ? root.bar.fontFamily : Style.font.family
    font.pixelSize: Style.font.body
  }
  component SectionLabel: Label {
    font.pixelSize: Style.font.caption
    font.bold: true
    font.letterSpacing: 1
    color: root.subdued
  }
  component Legend: Row {
    property color tint: root.serverColor
    property string text: ""
    spacing: Style.space(5)
    Rectangle { width: Style.space(6); height: width; radius: width / 2; color: parent.tint; border.width: 1; border.color: root.faint; anchors.verticalCenter: parent.verticalCenter }
    Label { text: parent.text; color: root.subdued; font.pixelSize: Style.font.caption }
  }
  component StatRow: Item {
    property string label: ""
    property string value: ""
    width: parent.width
    implicitHeight: Math.max(left.implicitHeight, right.implicitHeight)
    Label { id: left; text: parent.label; color: root.subdued; width: Math.min(implicitWidth, parent.width * 0.45); elide: Text.ElideRight; font.pixelSize: Style.font.bodySmall }
    Label {
      id: right
      text: parent.value
      anchors.right: parent.right
      width: Math.min(implicitWidth, parent.width - left.width - Style.space(12))
      horizontalAlignment: Text.AlignRight
      elide: Text.ElideMiddle
      font.pixelSize: Style.font.bodySmall
    }
  }
  component ActionButton: Button {
    property bool urgent: false
    foreground: urgent ? root.warning : root.ink
    fontFamily: root.bar ? root.bar.fontFamily : Style.font.family
    fontSize: Style.font.caption
    bordered: true
    active: urgent
  }
  component Chart: Column {
    property string title: ""
    property string value: ""
    property var values: []
    property color tint: root.serverColor
    property real maximum: 100
    width: parent.width
    spacing: Style.space(5)
    StatRow { label: parent.title; value: parent.value }
    Sparkline {
      width: parent.width; height: Style.space(60)
      values: parent.values; lineColor: parent.tint
      maximum: parent.maximum; points: 60
    }
  }
  component ServerRow: Rectangle {
    id: row
    property var server
    property bool current: false
    // A HoverHandler stays hovered over the row's own buttons, unlike MouseArea.
    readonly property bool hovered: hover.hovered
    signal clicked()
    width: contents.width
    height: rowContent.implicitHeight + Style.space(12)
    radius: Style.space(4)
    color: server.leak ? Qt.rgba(root.warning.r, root.warning.g, root.warning.b, current ? 0.18 : 0.10)
      : current ? root.faint : "transparent"

    HoverHandler { id: hover }
    MouseArea {
      anchors.fill: parent
      cursorShape: Qt.PointingHandCursor
      onClicked: row.clicked()
    }
    Row {
      id: rowContent
      anchors.verticalCenter: parent.verticalCenter
      x: Style.space(8)
      width: parent.width - Style.space(16)
      spacing: Style.space(10)
      Label {
        id: portLabel
        width: Style.space(52)
        text: ":" + row.server.ports[0] + (row.server.ports.length > 1 ? "+" : "")
        color: Model.portColor(row.server.ports[0])
        font.bold: true
        anchors.verticalCenter: parent.verticalCenter
      }
      Column {
        width: parent.width - portLabel.width - trailing.width - parent.spacing * 2
        anchors.verticalCenter: parent.verticalCenter
        spacing: Style.space(2)
        Label {
          width: parent.width; elide: Text.ElideRight
          text: Model.title(row.server)
          opacity: row.server.cwd_missing ? 0.6 : 1
        }
        Label {
          width: parent.width; elide: Text.ElideRight
          text: (row.server.protected ? "protected  ·  " : "") + Model.detail(row.server)
          color: row.server.leak ? root.warning : root.subdued
          font.pixelSize: Style.font.caption
        }
      }
      Item {
        id: trailing
        width: Style.space(110)
        height: stats.implicitHeight
        anchors.verticalCenter: parent.verticalCenter
        Column {
          id: stats
          visible: !row.hovered
          anchors.right: parent.right
          Label { anchors.right: parent.right; text: Model.bytes(row.server.memory); font.pixelSize: Style.font.bodySmall; color: row.server.leak ? root.warning : root.ink }
          Label { anchors.right: parent.right; text: Model.cpu(row.server.cpu) + " CPU"; font.pixelSize: Style.font.caption; color: root.subdued }
        }
        Row {
          visible: row.hovered
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          spacing: Style.space(4)
          ActionButton { text: "Open"; onClicked: root.service.open(row.server) }
          ActionButton {
            text: root.armed === "stop:" + row.server.id ? "Sure?" : "Stop"
            urgent: root.armed === "stop:" + row.server.id
            onClicked: { var s = row.server; root.confirm("stop:" + s.id, function() { root.service.stop(s) }) }
          }
        }
      }
    }
  }
}
