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
    text: root.vertical ? "GG" : "Garden Gate"
    tooltipText: "Open Garden Gate · iCloud Drive downloads\nRequires the Garden Gate companion to be installed"

    onPressed: function(mouseButton) {
      if (mouseButton === Qt.LeftButton)
        Quickshell.execDetached([Quickshell.env("HOME") + "/.local/bin/gardengate", "manage"])
    }
  }
}
