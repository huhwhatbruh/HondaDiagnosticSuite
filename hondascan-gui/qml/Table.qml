import QtQuick
import com.hondascan.app

// A table of text: a muted header row, then one row per entry; a row is a list of cells, each `{text, tone}` (tone 0 normal, 1 warn,
// 2 fault). `widths` are relative column weights; a tapped row is reported by its index.
Item {
    id: root
    property var headers: []
    property var rows: []                 // [[{text, tone}, ...], ...]
    property var weights: []              // relative widths, equal when empty
    property int selected: -1
    property bool tappable: false
    signal rowTapped(int index)

    implicitHeight: 28 + rows.length * 30

    readonly property real total: headers.reduce((a, _, i) => a + (weights[i] || 1), 0)
    function colX(i) { let x = 0; for (let k = 0; k < i; ++k) x += (weights[k] || 1) / total * (width - 2 * Theme.inset); return Theme.inset + x }
    function colW(i) { return (weights[i] || 1) / total * (width - 2 * Theme.inset) - 8 }
    function toneColor(t) { return t === 1 ? Theme.warn : t === 2 ? Theme.fault : Theme.foreground }

    Item {
        width: parent.width; height: 28
        Repeater {
            model: root.headers
            Text { required property int index; required property string modelData; x: root.colX(index); width: root.colW(index); anchors.verticalCenter: parent.verticalCenter
                   text: modelData; elide: Text.ElideRight; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        }
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
    }
    Repeater {
        model: root.rows
        Rectangle {
            id: row
            required property int index
            required property var modelData
            y: 28 + index * 30; width: root.width; height: 30
            color: root.selected === index ? Theme.accent : (tap.hovered && root.tappable) ? Theme.muted : "transparent"
            Repeater {
                model: row.modelData
                Text { required property int index; required property var modelData; x: root.colX(index); width: root.colW(index); anchors.verticalCenter: parent.verticalCenter
                       text: modelData.text; elide: Text.ElideRight; color: root.toneColor(modelData.tone); font { family: Theme.sans; pixelSize: 13 } }
            }
            Rectangle { visible: row.index < root.rows.length - 1; anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
            Tappable { id: tap; visible: root.tappable; anchors.fill: parent; radius: 0; text: row.modelData.map(c => c.text).join(", "); onClicked: root.rowTapped(row.index) }
        }
    }
}
