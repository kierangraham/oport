import QtQuick

// A 3×3 dot grid; one lit dot per running server, up to nine.
Item {
  id: root
  property int count: 0
  property color foreground: "white"
  property color alert: "#e0af68"
  property bool alerting: false
  readonly property real dot: Math.max(2, Math.floor(Math.min(width, height) / 4.2))
  readonly property real gap: (Math.min(width, height) - dot * 3) / 2

  Repeater {
    model: 9
    Rectangle {
      required property int index
      readonly property bool lit: index < root.count
      x: (index % 3) * (root.dot + root.gap)
      y: Math.floor(index / 3) * (root.dot + root.gap)
      width: root.dot; height: root.dot; radius: root.dot / 2
      color: root.alerting && lit ? root.alert : root.foreground
      opacity: lit ? 1 : 0.28
    }
  }
}
