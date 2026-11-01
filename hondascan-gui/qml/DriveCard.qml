import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// This drive: distance, fuel and economy (the numbers are made in drive.rs, in US units).
Panel {
    id: root
    readonly property var d: JSON.parse(AppModel.drive)
    readonly property var stats: d.stats ? d.stats : []
    readonly property int cols: width > 1000 ? 6 : width > 600 ? 4 : 2

    title: "This drive"
    note: d.note ? d.note : ""

    Item {
        width: parent.width
        height: root.stats.length === 0 ? 40 : Math.ceil(root.stats.length / root.cols) * 66
        Text {
            visible: root.stats.length === 0
            x: Theme.inset; anchors.verticalCenter: parent.verticalCenter
            text: root.d.message ? root.d.message : ""
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: 13 }
        }
        Repeater {
            model: root.stats
            Item {
                required property int index
                required property var modelData
                readonly property real cw: (root.width - 2) / root.cols
                readonly property bool lastCol: (index + 1) % root.cols === 0 || index === root.stats.length - 1
                readonly property bool lastRow: Math.floor(index / root.cols) === Math.ceil(root.stats.length / root.cols) - 1
                x: (index % root.cols) * cw
                y: Math.floor(index / root.cols) * 66
                width: cw; height: 66
                Text { x: Theme.inset; y: 12; width: parent.width - Theme.inset - 8; elide: Text.ElideRight; text: modelData.label; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Row {
                    x: Theme.inset; y: 30; spacing: 6
                    Text { id: val; text: modelData.text; color: Theme.foreground; font { family: Theme.sans; pixelSize: 21; weight: Font.DemiBold; features: { "tnum": 1 } } }
                    Text { text: modelData.unit; color: Theme.mutedFg; anchors.baseline: val.baseline; font { family: Theme.sans; pixelSize: 12 } }
                }
                Rectangle { visible: !lastCol; anchors.right: parent.right; width: 1; height: parent.height; color: Theme.border }
                Rectangle { visible: !lastRow; anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                HoverHandler { id: sh }
                ToolTip.visible: sh.hovered
                ToolTip.delay: 500
                ToolTip.text: modelData.hint
                Accessible.name: modelData.label + " " + modelData.text + " " + modelData.unit
                Accessible.description: modelData.hint
            }
        }
    }
}
