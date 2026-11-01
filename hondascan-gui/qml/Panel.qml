import QtQuick
import com.hondascan.app

// A pane: a title bar (title left, note right, an optional action) over a card. Content goes inside.
Item {
    id: root
    property string title
    property string note
    default property alias content: body.data
    property alias action: actionSlot.data

    implicitHeight: bar.height + body.childrenRect.height + 2 + Theme.rCard / 2

    Rectangle {
        anchors.fill: parent
        radius: Theme.rCard
        gradient: Theme.cardFill
        border.color: Theme.edge
        border.width: 1
    }
    Item {
        id: bar
        x: 1; y: 1
        width: parent.width - 2
        height: 40
        Caption { anchors { left: parent.left; leftMargin: Theme.inset; verticalCenter: parent.verticalCenter } text: root.title }
        Row {
            id: actionSlot
            anchors { right: parent.right; rightMargin: Theme.inset; verticalCenter: parent.verticalCenter }
            spacing: 8
            Text {
                anchors.verticalCenter: parent.verticalCenter
                visible: root.note !== ""
                text: root.note
                color: Theme.mutedFg
                font { family: Theme.sans; pixelSize: 12 }
            }
        }
    }
    Rectangle { x: 1; y: bar.height; width: parent.width - 2; height: 1; color: Theme.border }
    Item {
        id: body
        x: 1; y: bar.height + 1
        width: parent.width - 2
        height: childrenRect.height
    }
}
