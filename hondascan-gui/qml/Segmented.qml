import QtQuick
import QtQuick.Controls.Basic as C
import com.hondascan.app

// Tabs as a segmented control: a real Controls TabBar (arrow keys move between segments, Tab reaches it, the screen reader
// reads it) drawn as one track with one highlight that slides to the chosen segment. `items` is [{id, label}], `current` the id.
C.TabBar {
    id: root
    property var items: []
    property string current
    signal picked(string id)

    readonly property real cell: items.length > 0 ? (width - 4) / items.length : 0

    implicitWidth: items.length * 104 + 4
    implicitHeight: 34
    // The index follows `current`. Only the person picks: a click (the buttons' onClicked) or the arrow keys while the bar has
    // focus. The TabBar also moves its index by itself when its buttons are rebuilt (the list is re-made whenever the tabs'
    // data republishes); that move is undone, or a person on the fourth tab is thrown back to the first.
    readonly property int shown: Math.max(0, items.findIndex(i => i.id === current))
    currentIndex: shown
    onCurrentIndexChanged: {
        const it = items[currentIndex]
        if (!it || it.id === current) return
        if (activeFocus) picked(it.id)
        else Qt.callLater(() => { if (!activeFocus) currentIndex = Qt.binding(() => root.shown) })
    }

    background: Rectangle {
        radius: Theme.rControl + 2
        color: Theme.panel   // a track set into the pane, darker than it
        border.color: Theme.outline
        border.width: 1
        ControlFace {   // the chosen segment stands up out of the track
            y: 2; height: parent.height - 5
            width: root.cell
            x: 2 + root.currentIndex * root.cell
            Behavior on x { NumberAnimation { duration: Theme.normal; easing.type: Easing.OutCubic } }
        }
    }
    contentItem: ListView {
        model: root.contentModel
        currentIndex: root.currentIndex
        orientation: ListView.Horizontal
        interactive: false
        leftMargin: 2; topMargin: 2
        highlightMoveDuration: 0
    }

    Repeater {
        model: root.items
        C.TabButton {
            id: seg
            required property var modelData
            readonly property bool on: root.current === modelData.id
            width: root.cell; height: root.height - 4
            text: modelData.label
            hoverEnabled: true
            onClicked: root.picked(modelData.id)
            background: Rectangle {
                radius: Theme.rControl
                color: Theme.hover
                opacity: seg.hovered && !seg.on ? 0.55 : 0
                Behavior on opacity { NumberAnimation { duration: Theme.fast } }
                Rectangle {
                    anchors.fill: parent
                    radius: Theme.rControl
                    color: "transparent"
                    border { color: Theme.focus; width: 2 }
                    visible: seg.visualFocus
                }
            }
            contentItem: Text {
                text: seg.text
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                color: seg.on ? Theme.foreground : Theme.mutedFg
                Behavior on color { ColorAnimation { duration: Theme.fast } }
                font { family: Theme.sans; pixelSize: 13; weight: seg.on ? Font.DemiBold : Font.Normal }
            }
            HoverHandler { cursorShape: Qt.PointingHandCursor }
        }
    }
}
