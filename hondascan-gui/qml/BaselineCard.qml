import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// The newest frozen healthy baseline: when it was taken, what changed since, and the three actions (the words and the reasons
// an action may be off are written in baseline.rs). Reads and stores nothing about the car by itself.
Item {
    id: root
    signal freezeRequested
    signal markerRequested

    readonly property var c: JSON.parse(AppModel.maintenance)
    readonly property var snap: c.snapshot ? c.snapshot : null
    implicitHeight: col.implicitHeight + 20

    function toneColor(t) { return t === 1 ? Theme.warn : t === 2 ? Theme.fault : t === 4 ? Theme.ok : Theme.mutedFg }
    function when(t) { return new Date(t * 1000).toLocaleString(Qt.locale(), "MMM d, yyyy, h:mm AP") }

    Column {
        id: col
        x: Theme.inset; y: 10; width: parent.width - 2 * Theme.inset; spacing: 6

        Row {
            spacing: 10
            Caption { text: "Reference snapshot" }
            Text { text: root.c.hint_connect ? root.c.hint_connect : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        }
        Text { visible: !!root.c.err; width: parent.width; wrapMode: Text.WordWrap; text: root.c.err ? root.c.err : ""; color: Theme.fault; font { family: Theme.sans; pixelSize: 12 } }

        Text { visible: !root.snap; width: parent.width; wrapMode: Text.WordWrap; text: root.c.none_text ? root.c.none_text : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }

        Column {
            visible: !!root.snap
            width: parent.width; spacing: 4
            Row {
                spacing: 10
                Text { text: root.snap ? "Saved " + root.when(root.snap.created) : ""; color: Theme.foreground; font { family: Theme.sans; pixelSize: 14; weight: Font.DemiBold } }
                Text { visible: !!(root.snap && root.snap.label); text: root.snap ? "“" + root.snap.label + "”" : ""; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                Text { text: root.snap ? root.snap.counts : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
            }
            Text { visible: !!(root.snap && root.snap.tampered); width: parent.width; wrapMode: Text.WordWrap; text: root.snap ? root.snap.tampered_text : ""; color: Theme.fault; font { family: Theme.sans; pixelSize: 12 } }
            Text { visible: !!(root.snap && root.snap.no_change); text: root.snap ? root.snap.no_change_text : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
            Flow {
                width: parent.width; spacing: 8
                Repeater {
                    model: root.snap ? root.snap.diff : []
                    Row {
                        required property int index
                        required property var modelData
                        spacing: 8
                        Text {
                            text: modelData.text; color: root.toneColor(modelData.tone); font { family: Theme.sans; pixelSize: 13 }
                            HoverHandler { id: dh }
                            ToolTip.visible: dh.hovered && modelData.tip !== ""
                            ToolTip.text: modelData.tip
                        }
                        Text { visible: index < root.snap.diff.length - 1; text: "·"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
                    }
                }
            }
        }

        Item { width: 1; height: 2 }
        Flow {
            width: parent.width; spacing: 8
            Button { text: "Save reference snapshot"; primary: true; enabled: !!root.c.can_freeze; hint: root.c.freeze_hint ? root.c.freeze_hint : ""; onClicked: root.freezeRequested() }
            Button { text: "Add service marker"; enabled: !!root.c.can_marker; hint: root.c.marker_hint ? root.c.marker_hint : ""; onClicked: root.markerRequested() }
            Button { text: "Print health report"; enabled: !!root.c.can_report; hint: root.c.report_hint ? root.c.report_hint : ""; onClicked: AppModel.printReport() }
        }
        Row {
            visible: !!root.c.report
            spacing: 10
            Text { text: root.c.report ? root.c.report.text : ""; color: root.c.report ? root.toneColor(root.c.report.tone) : Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
            Text { text: root.c.report ? root.c.report.summary : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
            Button { visible: !!(root.c.report && root.c.report.path); text: "Open"; onClicked: AppModel.openPath(root.c.report.path) }
        }
    }
}
