import QtQuick
import com.hondascan.app

// The installation check on the welcome screen: one quiet line when all is well; a failure spells itself out. The lines and their
// wording come from selfcheck.rs (a shape as well as a colour for every status).
Column {
    id: root
    readonly property var c: JSON.parse(AppModel.selfcheck)
    property bool open: false
    spacing: 6

    function toneColor(t) { return t === 2 ? Theme.fault : t === 1 ? Theme.mutedFg : Theme.ok }

    // a status's shape: the backend sends text ("✓", "✗", "i"); the fonts have no tick or cross, so those two are drawn
    component Mark: Item {
        property string shape
        property color tone
        width: 16; height: 18
        Glyph { anchors.centerIn: parent; width: 13; height: 13; visible: parent.shape === "✓" || parent.shape === "✗"; kind: parent.shape === "✓" ? "check" : "cross"; color: parent.tone }
        Text { anchors.centerIn: parent; visible: parent.shape !== "✓" && parent.shape !== "✗"; text: parent.shape; color: parent.tone; font { family: Theme.sans; pixelSize: 15; weight: Font.DemiBold } }
    }

    component Line: Row {
        property var row
        spacing: 10
        Mark { shape: row.shape; tone: root.toneColor(row.tone) }
        Column {
            Text { text: row.title; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
            Text { visible: row.detail !== ""; width: Math.min(640, root.width - 40); wrapMode: Text.WordWrap; text: row.detail; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 11 } }
        }
    }

    Text { visible: !!root.c.pending; anchors.horizontalCenter: parent.horizontalCenter; text: root.c.waiting ? root.c.waiting : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }

    Row {
        visible: !root.c.pending && !!root.c.headline
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: 10
        Mark { shape: root.c.headline ? root.c.headline.shape : ""; tone: root.c.headline ? root.toneColor(root.c.headline.tone) : Theme.ok }
        Text { text: root.c.headline ? root.c.headline.text : ""; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13; weight: Font.DemiBold } }
        Text {
            id: toggle
            text: root.open ? "Hide details" : "Details"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 }
            Tappable {
                anchors { fill: parent; margins: -4 }
                text: toggle.text
                onClicked: root.open = !root.open
            }
        }
    }
    Repeater { model: root.c.rows ? root.c.rows : []; Line { required property var modelData; row: modelData; anchors.horizontalCenter: parent.horizontalCenter } }
    Column {
        visible: root.open
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: 6
        Repeater { model: root.c.details ? root.c.details : []; Line { required property var modelData; row: modelData } }
    }
}
