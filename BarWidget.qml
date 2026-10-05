import QtQuick
import Quickshell
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "io.github.tcballard.gardengate"

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: ""
    hasVisualContent: true
    labelVisible: false
    fixedWidth: root.vertical ? button.barSize : Style.space(34)
    fixedHeight: root.vertical ? Style.space(34) : button.barSize
    tooltipText: "Garden Gate · iCloud Drive\nConnect your account or open your inbox"

    // Draw the gate directly: no font glyph dependency or fixed theme colour.
    Canvas {
      anchors.centerIn: parent
      width: Style.space(19)
      height: width
      property color ink: button.foreground
      onInkChanged: requestPaint()
      onWidthChanged: requestPaint()
      onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        ctx.scale(width / 24, height / 24)
        ctx.strokeStyle = ink
        ctx.lineWidth = 1.8
        ctx.lineCap = "round"
        ctx.lineJoin = "round"
        ctx.beginPath()
        ctx.moveTo(3, 21); ctx.lineTo(3, 4)
        ctx.moveTo(21, 21); ctx.lineTo(21, 4)
        ctx.moveTo(3, 7); ctx.lineTo(21, 7)
        ctx.moveTo(3, 19); ctx.lineTo(21, 19)
        ctx.moveTo(7, 7); ctx.lineTo(7, 19)
        ctx.moveTo(12, 7); ctx.lineTo(12, 19)
        ctx.moveTo(17, 7); ctx.lineTo(17, 19)
        ctx.moveTo(4, 18); ctx.lineTo(20, 8)
        ctx.stroke()
      }
    }

    onPressed: function(mouseButton) {
      if (mouseButton === Qt.LeftButton)
        Quickshell.execDetached([Quickshell.env("HOME") + "/.local/bin/gardengate", "manage"])
    }
  }
}
