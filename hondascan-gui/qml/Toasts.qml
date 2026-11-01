import QtQuick
import com.hondascan.app

// Notices from the app, bottom right. A notice with the same id replaces the one showing; each goes away after its time.
Column {
    id: root
    spacing: 8
    width: 360

    ListModel { id: items }

    function show(level, id, title, body, seconds) {
        for (let i = 0; i < items.count; ++i) if (items.get(i).nid === id) { items.remove(i); break }
        items.append({ nid: id, level: level, title: title, body: body, seconds: seconds })
    }

    Connections {
        target: AppModel
        function onToast(level, id, title, body, seconds) { root.show(level, id, title, body, seconds) }
    }

    Repeater {
        model: items
        Rectangle {
            id: card
            required property int index
            required property string nid
            required property int level
            required property string title
            required property string body
            required property real seconds
            width: root.width
            height: col.implicitHeight + 24
            radius: Theme.rCard
            color: Theme.popover
            border.color: Theme.outline
            border.width: 1
            // arrives from the right edge and fades in
            property real shown: 0
            Component.onCompleted: shown = 1
            Behavior on shown { NumberAnimation { duration: Theme.normal; easing.type: Easing.OutCubic } }
            opacity: shown
            transform: Translate { x: (1 - card.shown) * 32 }

            // the level as a badge at the head of the notice: its colour and its symbol, so it never rests on colour alone
            readonly property color tone: level === 3 ? Theme.fault : level === 2 ? Theme.warn : level === 1 ? Theme.ok : Theme.foreground
            Rectangle {
                x: 14; y: 13
                width: 20; height: 20; radius: 10
                color: Qt.rgba(card.tone.r, card.tone.g, card.tone.b, 0.16)
                Glyph { anchors.centerIn: parent; width: 10; height: 10; visible: card.level === 3 || card.level === 1; kind: card.level === 3 ? "cross" : "check"; color: card.tone }
                Text {
                    anchors.centerIn: parent
                    visible: card.level === 2 || card.level === 0
                    text: card.level === 2 ? "!" : "i"
                    color: card.tone
                    font { family: Theme.sans; pixelSize: 12; weight: Font.Bold }
                }
            }
            Column {
                id: col
                x: 44; y: 14
                width: parent.width - 58
                spacing: 3
                Text {
                    width: parent.width
                    text: card.title
                    color: Theme.foreground
                    wrapMode: Text.WordWrap
                    font { family: Theme.sans; pixelSize: 13; weight: Font.DemiBold }
                }
                Text {
                    visible: card.body !== ""
                    width: parent.width
                    text: card.body
                    color: Theme.mutedFg
                    wrapMode: Text.WordWrap
                    font { family: Theme.sans; pixelSize: 12 }
                }
            }
            TapHandler { onTapped: items.remove(card.index) }
            Timer { interval: card.seconds * 1000; running: true; onTriggered: items.remove(card.index) }
            Accessible.role: Accessible.AlertMessage
            Accessible.name: card.title
        }
    }
}
