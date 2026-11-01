import QtQuick
import QtQuick.Controls.Basic as C
import com.hondascan.app

// A tick box with its words: a real Controls CheckBox (Space toggles it, Tab reaches it, the screen reader reads it).
// The caller binds `checked` and answers `toggled`.
C.CheckBox {
    id: root
    property bool wrap: false
    spacing: 10
    padding: 0
    hoverEnabled: true

    indicator: Rectangle {
        x: 0; y: root.wrap ? 2 : (root.height - height) / 2
        width: 16; height: 16
        radius: 4
        color: root.checked ? Theme.red : "transparent"
        Behavior on color { ColorAnimation { duration: Theme.fast } }
        border.color: root.visualFocus ? Theme.focus : root.checked ? Theme.red : root.hovered ? Theme.mutedFg : Theme.outline
        border.width: root.visualFocus ? 2 : 1
        Glyph { anchors.centerIn: parent; width: 12; height: 12; visible: root.checked; kind: "check"; color: Theme.redText }
    }
    contentItem: Text {
        leftPadding: root.indicator.width + root.spacing
        text: root.text
        wrapMode: root.wrap ? Text.WordWrap : Text.NoWrap
        verticalAlignment: Text.AlignVCenter
        color: Theme.foreground
        font { family: Theme.sans; pixelSize: root.wrap ? 12 : 13 }
    }
    HoverHandler { cursorShape: Qt.PointingHandCursor }
}
