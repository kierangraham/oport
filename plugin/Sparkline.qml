import QtQuick

Canvas {
  id: root
  property var values: []
  property var secondary: []
  property color lineColor: "#7dcfff"
  property color secondaryColor: "#bb9af7"
  property real maximum: 100
  property bool grid: true
  property bool stacked: false
  property int points: 60
  readonly property real ceiling: maximum > 0 ? maximum : Math.max(1024, ...values.slice(-points), ...secondary.slice(-points)) * 1.1

  onValuesChanged: requestPaint()
  onSecondaryChanged: requestPaint()
  onLineColorChanged: requestPaint()
  onSecondaryColorChanged: requestPaint()
  onMaximumChanged: requestPaint()
  onPointsChanged: requestPaint()
  onGridChanged: requestPaint()
  onStackedChanged: requestPaint()
  onWidthChanged: requestPaint()
  onHeightChanged: requestPaint()
  onVisibleChanged: if (visible) requestPaint()

  onPaint: {
    var ctx = getContext("2d")
    ctx.clearRect(0, 0, width, height)
    if (grid) {
      ctx.strokeStyle = Qt.rgba(lineColor.r, lineColor.g, lineColor.b, 0.10)
      ctx.lineWidth = 1
      for (var g = 1; g <= 3; g++) {
        var y = Math.round(height * g / 4) + 0.5
        ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(width, y); ctx.stroke()
      }
    }
    function draw(series, tint, filled) {
      var data = series.slice(-root.points)
      if (data.length < 2) return
      var offset = root.points - data.length
      function x(i) { return (i + offset) / (root.points - 1) * width }
      function y(i) { return height - 2 - Math.min(1, Math.max(0, data[i] / root.ceiling)) * (height - 4) }
      ctx.beginPath()
      ctx.moveTo(x(0), y(0))
      for (var i = 1; i < data.length; i++) ctx.lineTo(x(i), y(i))
      if (filled) {
        ctx.lineTo(x(data.length - 1), height); ctx.lineTo(x(0), height); ctx.closePath()
        var gradient = ctx.createLinearGradient(0, 0, 0, height)
        gradient.addColorStop(0, Qt.rgba(tint.r, tint.g, tint.b, 0.25))
        gradient.addColorStop(1, Qt.rgba(tint.r, tint.g, tint.b, 0.015))
        ctx.fillStyle = gradient; ctx.fill()
        ctx.beginPath(); ctx.moveTo(x(0), y(0))
        for (var j = 1; j < data.length; j++) ctx.lineTo(x(j), y(j))
      }
      ctx.strokeStyle = tint; ctx.lineWidth = root.grid ? 1.8 : 1.3; ctx.lineJoin = "round"; ctx.stroke()
    }
    if (root.stacked) {
      var bottom = root.values.slice(-root.points)
      var top = root.secondary.slice(-root.points)
      var count = Math.min(bottom.length, top.length)
      if (count < 2) return
      var offset = root.points - count
      function sx(i) { return (i + offset) / (root.points - 1) * width }
      function sy(value) { return height - 2 - Math.min(1, Math.max(0, value / root.ceiling)) * (height - 4) }
      function band(upper, lower, tint) {
        ctx.beginPath(); ctx.moveTo(sx(0), sy(upper[0]))
        for (var i = 1; i < count; i++) ctx.lineTo(sx(i), sy(upper[i]))
        for (var j = count - 1; j >= 0; j--) ctx.lineTo(sx(j), sy(lower[j]))
        ctx.closePath()
        ctx.fillStyle = Qt.rgba(tint.r, tint.g, tint.b, 0.3); ctx.fill()
        ctx.beginPath(); ctx.moveTo(sx(0), sy(upper[0]))
        for (var k = 1; k < count; k++) ctx.lineTo(sx(k), sy(upper[k]))
        ctx.strokeStyle = tint; ctx.lineWidth = 1.6; ctx.lineJoin = "round"; ctx.stroke()
      }
      band(bottom.map(function(value, i) { return value + top[i] }), bottom, root.secondaryColor)
      band(bottom, bottom.map(function() { return 0 }), root.lineColor)
    } else {
      draw(root.values, root.lineColor, true)
      draw(root.secondary, root.secondaryColor, false)
    }
  }
}
