import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// Observed engine maps: what the PCM reported at each engine speed and load, one table per signal, under a headline finding.
// Every sentence, tone and cell value is made in maps.rs.
Panel {
    id: root
    readonly property var m: JSON.parse(AppModel.maps)
    readonly property var t: m.table ? m.table : null
    function tone(t) { return t === 2 ? Theme.fault : t === 1 ? Theme.warn : t === 4 ? Theme.ok : t === 3 ? Theme.mutedFg : Theme.foreground }

    title: m.title ? m.title : "Engine maps"
    note: m.subtitle ? m.subtitle : ""

    Column {
        width: parent.width
        spacing: 8
        topPadding: 12; bottomPadding: 12

        Text { visible: !!m.waiting; x: Theme.inset; text: m.waiting ? m.waiting : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }

        Row {
            visible: !!m.headline
            x: Theme.inset; spacing: 16
            Text { id: big; text: m.headline ? m.headline.big : ""; color: m.headline ? root.tone(m.headline.tone) : Theme.foreground; font { family: Theme.sans; pixelSize: 34; weight: Font.DemiBold; features: { "tnum": 1 } } }
            Column {
                anchors.verticalCenter: big.verticalCenter
                width: root.width - 2 * Theme.inset - big.implicitWidth - 16
                Text { width: parent.width; wrapMode: Text.WordWrap; text: m.headline ? m.headline.text : ""; color: Theme.foreground; font { family: Theme.sans; pixelSize: 15; weight: Font.DemiBold } }
                Text { width: parent.width; wrapMode: Text.WordWrap; text: m.headline ? m.headline.sub : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
            }
        }
        Text { visible: !!m.history; x: Theme.inset; width: root.width - 2 * Theme.inset; wrapMode: Text.WordWrap; text: m.history ? m.history.text : ""; color: m.history ? root.tone(m.history.tone) : Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }

        Flow {
            visible: !!(m.tabs && m.tabs.length > 0)
            x: Theme.inset; width: root.width - 2 * Theme.inset; spacing: 4
            Repeater {
                model: m.tabs ? m.tabs : []
                Chip {
                    required property var modelData
                    text: modelData.label
                    on: root.m.selected === modelData.symbol
                    dim: !modelData.has_data
                    onClicked: AppModel.selectMap(modelData.symbol)
                }
            }
        }

        Text { visible: !!(root.t && root.t.about); x: Theme.inset; width: root.width - 2 * Theme.inset; wrapMode: Text.WordWrap; text: root.t ? root.t.about : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        Text { visible: !!m.empty; x: Theme.inset; width: root.width - 2 * Theme.inset; wrapMode: Text.WordWrap; text: m.empty ? m.empty : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
        Text { visible: !!(root.t && root.t.axes_error); x: Theme.inset; width: root.width - 2 * Theme.inset; wrapMode: Text.WordWrap; text: root.t && root.t.axes_error ? root.t.axes_error : ""; color: Theme.fault; font { family: Theme.sans; pixelSize: 12 } }

        // the grid: engine speed down the side, load across the top
        Item {
            id: grid
            visible: !!(root.t && root.t.cells && root.t.cells.length > 0)
            x: Theme.inset; width: root.width - 2 * Theme.inset
            readonly property var cells: root.t && root.t.cells ? root.t.cells : []
            readonly property var loads: root.t && root.t.load ? root.t.load : []
            readonly property var rpms: root.t && root.t.rpm ? root.t.rpm : []
            readonly property int firstRow: cells.length > 0 ? cells[0].r : 0
            readonly property int rowCount: loads.length > 0 ? Math.round(cells.length / loads.length) : 0
            readonly property real labelW: 64
            readonly property real cw: loads.length > 0 ? (width - labelW) / loads.length : 40
            height: 22 + rowCount * 26

            Repeater {
                model: grid.loads
                Text { required property int index; required property real modelData; x: grid.labelW + index * grid.cw; width: grid.cw; horizontalAlignment: Text.AlignHCenter
                       text: modelData.toFixed(2); color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 11 } }
            }
            Repeater {
                model: grid.rowCount
                Text { required property int index; y: 22 + index * 26; height: 26; width: grid.labelW - 8; horizontalAlignment: Text.AlignRight; verticalAlignment: Text.AlignVCenter
                       text: Math.round(grid.rpms[grid.firstRow + index] || 0); color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 11 } }
            }
            Repeater {
                model: grid.cells
                Rectangle {
                    id: cell
                    required property int index
                    required property var modelData
                    readonly property color base: modelData.tone === 2 ? Theme.fault : Theme.warn
                    x: grid.labelW + modelData.l * grid.cw; y: 22 + (modelData.r - grid.firstRow) * 26
                    width: grid.cw - 1; height: 25
                    color: modelData.v === null ? "transparent" : Qt.tint(Theme.card, Qt.rgba(base.r, base.g, base.b, modelData.dev * 0.7))
                    opacity: modelData.thin ? 0.5 : 1
                    border.color: modelData.outlined ? Theme.blue : "transparent"; border.width: 2
                    Text { anchors.centerIn: parent; text: modelData.text; color: modelData.v === null ? Theme.border : Theme.foreground; font { family: Theme.mono; pixelSize: 12 } }
                    HoverHandler { id: ch }
                    ToolTip.visible: ch.hovered && modelData.v !== null
                    ToolTip.delay: 500
                    ToolTip.text: modelData.hover ? modelData.hover : ""
                }
            }
        }
        Text { visible: !!(root.t && root.t.legend); x: Theme.inset; width: root.width - 2 * Theme.inset; wrapMode: Text.WordWrap; text: root.t && root.t.legend ? root.t.legend : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 11 } }
        Text { visible: !!(root.t && root.t.note); x: Theme.inset; width: root.width - 2 * Theme.inset; wrapMode: Text.WordWrap; text: root.t && root.t.note ? root.t.note : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 11 } }
    }
}
