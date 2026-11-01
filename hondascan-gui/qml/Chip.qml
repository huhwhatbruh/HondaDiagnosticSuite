import QtQuick
import com.hondascan.app

// A choice in a row of choices (a map's signal, a kind of service marker): the chosen one is tinted. Reachable by keyboard.
Rectangle {
    id: root
    property string text
    property bool on: false
    property bool dim: false
    signal clicked

    width: label.implicitWidth + 24
    height: 28
    color: on ? Theme.accent : tap.hovered ? Theme.hover : Theme.secondary
    radius: Theme.rControl
    border.color: Theme.outline
    border.width: 1
    opacity: dim ? 0.55 : 1
    Text { id: label; anchors.centerIn: parent; text: root.text; color: Theme.foreground; font { family: Theme.sans; pixelSize: 12 } }
    Tappable {
        id: tap
        anchors.fill: parent
        radius: Theme.rControl
        text: root.text
        Accessible.checkable: true
        Accessible.checked: root.on
        onClicked: root.clicked()
    }
}
