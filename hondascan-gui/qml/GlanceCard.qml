import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// The key numbers of a system type, as gauges and tiles, in a quiet card (what to show and how it reads is decided in glance.rs).
Panel {
    id: root
    property var card: ({})            // the layout: kept between updates
    property var live: ({})            // this card's readings: {note, sig}, new at every update
    function sig(m) { const s = live.sig; return s && s[m.did + ":" + m.i] ? s[m.did + ":" + m.i] : ({}) }
    readonly property var gauges: card.gauges ? card.gauges : []
    readonly property var tiles: card.tiles ? card.tiles : []
    readonly property int cols: Math.max(1, Math.floor(width / 170))
    readonly property real cell: width / cols
    // the gauges spread over the width in their own row (never more cells than gauges, so two gauges do not leave three
    // empty cells); the tiles below keep the grid. Separators are drawn between cells only, never against the card's edge.
    readonly property int gaugeCols: Math.max(1, Math.min(gauges.length, cols))
    readonly property real gaugeCell: width / gaugeCols
    readonly property real dial: Math.min(gaugeCell - 32, 176)
    readonly property real gaugeH: probe.fitHeight(dial) + 28
    Gauge { id: probe; visible: false; width: 0; height: 0 }   // only for fitHeight
    readonly property real tileH: 118
    readonly property real gaugesH: Math.ceil(gauges.length / gaugeCols) * gaugeH

    title: card.title ? card.title : ""
    note: live.note ? live.note : ""

    Item {
        width: parent.width
        height: root.gaugesH + Math.ceil(root.tiles.length / root.cols) * root.tileH + 1

        Repeater {
            model: root.gauges
            Item {
                required property int index
                required property var modelData
                readonly property int col: index % root.gaugeCols
                x: col * root.gaugeCell; y: Math.floor(index / root.gaugeCols) * root.gaugeH
                width: root.gaugeCell; height: root.gaugeH
                Gauge { anchors.centerIn: parent; width: root.dial; height: root.gaugeH - 28; label: modelData.name; fig: root.sig(modelData).hidden ? ({}) : root.sig(modelData); fallbackMax: 100 }
                Rectangle { visible: col < root.gaugeCols - 1; anchors.right: parent.right; y: 16; width: 1; height: parent.height - 32; color: Theme.border }
            }
        }
        Rectangle { visible: root.gauges.length > 0 && root.tiles.length > 0; y: root.gaugesH; width: parent.width; height: 1; color: Theme.border }
        Repeater {
            model: root.tiles
            Item {
                id: tile
                required property int index
                required property var modelData
                readonly property var f: root.sig(modelData)
                readonly property int col: index % root.cols
                readonly property int row: Math.floor(index / root.cols)
                x: col * root.cell; y: root.gaugesH + row * root.tileH
                width: root.cell; height: root.tileH
                Text { x: Theme.inset; y: 10; width: parent.width - Theme.inset - 8; elide: Text.ElideRight; text: modelData.name; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Row {
                    x: Theme.inset; y: 30; spacing: 6
                    Text {
                        id: val
                        text: tile.f.text || ""
                        color: tile.f.state === "on" ? Theme.ok : tile.f.state === "off" ? Theme.mutedFg : Theme.tone(tile.f.tone || 0)
                        font { family: Theme.sans; pixelSize: 24; weight: Font.DemiBold; features: { "tnum": 1 } }
                    }
                    Text { text: tile.f.unit || ""; color: Theme.mutedFg; anchors.baseline: val.baseline; font { family: Theme.sans; pixelSize: 12 } }
                }
                Text { x: Theme.inset; y: 66; width: parent.width - 2 * Theme.inset; elide: Text.ElideRight; visible: !!tile.f.note; text: tile.f.note ? tile.f.note : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 11 } }
                Spark { x: Theme.inset; y: parent.height - 36; width: parent.width - 2 * Theme.inset; height: 26; values: tile.f.hist || [] }
                Rectangle { visible: tile.col < root.cols - 1 && tile.index < root.tiles.length - 1; anchors.right: parent.right; y: 12; width: 1; height: parent.height - 24; color: Theme.border }
                Rectangle { visible: tile.row < Math.ceil(root.tiles.length / root.cols) - 1; anchors.bottom: parent.bottom; x: 12; width: parent.width - 24; height: 1; color: Theme.border }
                HoverHandler { id: th }
                ToolTip.visible: th.hovered && !!(modelData.about || modelData.raw)
                ToolTip.delay: 500
                ToolTip.text: [modelData.about || "", modelData.raw || ""].filter(x => x !== "").join("\n\n")
            }
        }
    }
}
