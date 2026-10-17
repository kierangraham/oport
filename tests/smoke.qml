import QtQuick
import Quickshell
import "plugin" as Oport

ShellRoot {
  id: root
  property int step: 0
  property int ticks: 0
  property int pending: 0
  readonly property string output: Quickshell.env("OPORT_SMOKE_OUTPUT")

  Oport.Service { id: service }
  QtObject {
    id: shellApi
    function serviceFor(id) { return service }
    function summon(id, payload) { widget.open() }
    function hide(id) { widget.close() }
    function toggle(id, payload) { widget.toggle() }
  }
  QtObject {
    id: barApi
    property var shell: shellApi
    property color foreground: "#c0caf5"
    property color barForeground: foreground
    property color background: "#1a1b26"
    property color urgent: "#f7768e"
    property string fontFamily: "monospace"
    property string position: "top"
    property bool vertical: false
    property int barSize: 35
    property bool foregroundAnimationEnabled: false
    property var activePopout: null
    property var clickTargets: []
    function registerClickTarget(target) {}
    function unregisterClickTarget(target) {}
    function hideTooltip(target) {}
    function showTooltip(target, text) {}
    function requestPopout(owner) {}
    function releasePopout(owner) {}
  }
  // Visible only for offscreen renders; the Wayland smoke run stays hidden.
  FloatingWindow {
    visible: !!root.output
    implicitWidth: 560; implicitHeight: 820
    color: "#101315"
    Rectangle { width: parent.width; height: 35; color: "#1a1b26" }
    Oport.BarWidget { id: widget; bar: barApi; x: 520 - width; height: 35 }
  }

  function fail(message) { console.error("FAIL: " + message); Qt.quit() }
  function card() {
    var item = widget.panelContents
    while (item && item.objectName !== "panelShim") item = item.parent
    return item
  }
  function grab(name) {
    if (!output) return
    pending++
    card().grabToImage(function(result) {
      result.saveToFile(output + "/" + name + ".png")
      pending--
    })
  }

  Timer {
    interval: 250; repeat: true; running: true
    onTriggered: {
      root.ticks++
      if (root.ticks > 80) return root.fail("timed out at step " + root.step)
      if (!service.ready || root.pending) return
      var demo = service.servers.filter(function(s) { return s.ports.indexOf(5199) >= 0 })[0]
      switch (root.step) {
      case 0:
        if (!demo) return
        if (demo.name !== "demo-web" || demo.command.indexOf("npm") !== 0) return root.fail("demo server misidentified: " + JSON.stringify(demo))
        if (widget.implicitWidth <= 0 || widget.stale) return root.fail("bar widget has no live data")
        // Opening maps a real layer-shell surface, so only the offscreen render run does it.
        if (root.output) widget.open()
        break
      case 1: root.grab("list"); break
      case 2:
        widget.showDetails(demo.id)
        if (!widget.detail || widget.detail.id !== demo.id) return root.fail("details did not open")
        break
      case 3: root.grab("details"); break
      case 4:
        widget.startCleanup()
        if (widget.view !== "cleanup") return root.fail("clean up did not open")
        widget.toggleSelected(demo)
        if (!widget.selected[demo.id]) return root.fail("selection did not toggle")
        break
      case 5: root.grab("cleanup"); break
      case 6:
        widget.showList()
        widget.close()
        barApi.vertical = true
        widget.settings = {showCount: false, hideWhenEmpty: true}
        if (widget.showCount || !widget.hideWhenEmpty) return root.fail("settings did not apply")
        if (JSON.parse(service.statusJson()).servers < 1) return root.fail("IPC status is empty")
        break
      case 7:
        console.log("PASS: live scanner, list, details, clean up, settings and vertical layout")
        Qt.quit()
      }
      root.step++
    }
  }
}
