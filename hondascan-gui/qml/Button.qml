import QtQuick
import QtQuick.Controls.Basic as C
import com.hondascan.app

// A button: a raised face (ControlFace); "primary" is the red one. A real Controls Button underneath, so Tab,
// Space, Return and the screen reader come from Qt; this draws it.
C.Button {
    id: root
    property bool primary: false
    property string hint

    implicitWidth: contentItem.implicitWidth + leftPadding + rightPadding
    implicitHeight: 32
    leftPadding: 14
    rightPadding: 14
    hoverEnabled: true
    opacity: enabled ? 1 : 0.45

    background: ControlFace {
        enabled: root.enabled
        hovered: root.hovered
        down: root.down
        primary: root.primary
        focusRing: root.visualFocus
    }
    contentItem: Text {
        text: root.text
        color: root.primary ? Theme.redText : Theme.foreground
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        font { family: Theme.sans; pixelSize: 13; weight: Font.Medium }
        transform: Translate { y: root.down ? 1 : 0 }   // goes down with the face
    }
    HoverHandler { cursorShape: Qt.PointingHandCursor }
    C.ToolTip.visible: hovered && hint !== ""
    C.ToolTip.delay: 500
    C.ToolTip.text: hint
}
