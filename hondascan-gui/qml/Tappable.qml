import QtQuick
import QtQuick.Controls.Basic as C
import com.hondascan.app

// A clickable area that draws nothing of its own but the keyboard focus ring: a real Controls AbstractButton underneath, so Tab
// reaches it, Space and Return press it, and the screen reader reads `text`. Put it over (anchors.fill) what it makes a button;
// `hovered` is for the owner's hover colour. Size it to at least 20 x 20 (the HIG's smallest macOS control).
C.AbstractButton {
    id: root
    property string hint
    property int radius: Theme.rRow

    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    padding: 0
    Accessible.name: text
    Accessible.role: Accessible.Button

    background: Rectangle {
        color: "transparent"
        radius: root.radius
        border { color: Theme.focus; width: 2 }
        visible: root.visualFocus
    }
    HoverHandler { cursorShape: Qt.PointingHandCursor }
    C.ToolTip.visible: hovered && hint !== ""
    C.ToolTip.delay: 500
    C.ToolTip.text: hint
}
