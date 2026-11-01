import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// Everything a system can report, in HDS's own terms: the data group each signal lives in, its symbol and HDS description, next to the
// plain name. The rows are asked for (a long table) whenever the search changes or the tab opens (reference.rs).
Rectangle {
    id: root
    gradient: Theme.cardFill
    radius: Theme.rCard; border.color: Theme.edge; border.width: 1
    implicitHeight: col.implicitHeight + 24

    property var ref: ({ loading: true })
    function refresh() { ref = JSON.parse(AppModel.reference(search.text)) }
    onVisibleChanged: if (visible) refresh()
    Timer { interval: 300; repeat: true; running: root.visible && !!root.ref.loading; onTriggered: root.refresh() }

    // the columns share the width in these proportions (fixed pixel widths ran past the pane on a narrow window and hid
    // the group and unit columns)
    readonly property var weights: [230, 300, 340, 110, 70, 70]
    readonly property real colSpace: col.width - 8 * (weights.length - 1)
    readonly property var widths: weights.map(w => Math.floor(w / weights.reduce((a, b) => a + b, 0) * colSpace))

    Column {
        id: col
        x: Theme.inset; y: 12; width: parent.width - 2 * Theme.inset; spacing: 8
        Text { visible: !!root.ref.loading; text: "Loading…"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
        Row {
            visible: !root.ref.loading
            spacing: 12
            Field { id: search; width: 300; placeholderText: root.ref.hint ? root.ref.hint : ""; onTextEdited: root.refresh() }
            Text { anchors.verticalCenter: parent.verticalCenter; text: root.ref.counts ? root.ref.counts : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        }
        Text { visible: !root.ref.loading; width: parent.width; wrapMode: Text.WordWrap; text: root.ref.note ? root.ref.note : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        Row {
            visible: !root.ref.loading
            spacing: 8
            Repeater { model: root.ref.headers ? root.ref.headers : []; Text { required property int index; required property string modelData; width: root.widths[index]; text: modelData; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } } }
        }
        ListView {
            visible: !root.ref.loading
            width: parent.width; height: Math.min(560, contentHeight); clip: true
            model: root.ref.rows ? root.ref.rows : []
            boundsBehavior: Flickable.StopAtBounds
            delegate: Row {
                required property var modelData
                height: 24; spacing: 8
                Text { width: root.widths[0]; elide: Text.ElideRight; text: modelData.name; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                Text { width: root.widths[1]; elide: Text.ElideRight; text: modelData.description; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
                Text { width: root.widths[2]; elide: Text.ElideRight; text: modelData.about; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
                Text { width: root.widths[3]; elide: Text.ElideRight; text: modelData.symbol; color: Theme.foreground; font { family: Theme.mono; pixelSize: 12 } }
                Text { width: root.widths[4]; text: modelData.group; color: Theme.foreground; font { family: Theme.mono; pixelSize: 12 } }
                Text { width: root.widths[5]; text: modelData.unit; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
            }
        }
    }
}
