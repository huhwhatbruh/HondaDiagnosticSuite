import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// Every live signal of the open system, searchable and grouped by data identifier, with pinned gauges on top. Only open groups
// and pinned signals are read, so narrowing the view speeds up the update rate (the decisions are in sysview.rs).
Column {
    id: root
    spacing: 10

    readonly property var sv: JSON.parse(AppModel.system)            // the layout: kept between updates
    readonly property var lv: JSON.parse(AppModel.systemLive)         // the readings: new at every update
    function sig(m) { const s = lv.sig; return s && s[m.did + ":" + m.i] ? s[m.did + ":" + m.i] : ({}) }
    readonly property var pins: sv.pins ? sv.pins : []
    readonly property var groups: sv.groups ? sv.groups : []

    // pinned gauges: equal cells across the width (never more cells than gauges, so a few pins spread out instead of
    // bunching at the left), each dial centred in its cell and no taller than it needs
    Rectangle {
        id: pinned
        visible: root.pins.length > 0
        width: parent.width
        height: Math.ceil(root.pins.length / cols) * rowH + 16
        gradient: Theme.cardFill
        radius: Theme.rCard
        border.color: Theme.edge
        border.width: 1
        readonly property int cols: Math.max(1, Math.min(root.pins.length, Math.floor(width / 200)))
        readonly property real cell: width / cols
        readonly property real dial: Math.min(cell - 32, 196)
        readonly property real rowH: probe.fitHeight(dial) + 20
        Gauge { id: probe; visible: false; width: 0; height: 0 }   // only for fitHeight
        Repeater {
            model: root.pins
            Item {
                required property int index
                required property var modelData
                x: (index % pinned.cols) * pinned.cell
                y: 8 + Math.floor(index / pinned.cols) * pinned.rowH
                width: pinned.cell
                height: pinned.rowH
                HoverHandler { id: cellHover }
                Gauge {
                    id: g
                    anchors.centerIn: parent
                    width: pinned.dial; height: pinned.rowH - 20
                    label: modelData.name
                    fig: root.sig(modelData).hidden ? ({}) : root.sig(modelData)
                    fallbackMax: 100
                }
                Tappable {   // 24 x 24, above the HIG's 20-point floor for a macOS control; quiet until the gauge is pointed at
                    id: unpin
                    x: g.x + g.width - width / 2; y: g.y
                    width: 24; height: 24; radius: 12
                    text: "Unpin " + modelData.name
                    hint: "Unpin"
                    opacity: cellHover.hovered || unpin.visualFocus ? 1 : 0.35
                    Behavior on opacity { NumberAnimation { duration: Theme.fast } }
                    onClicked: AppModel.togglePin(modelData.did, modelData.i)
                    Glyph { anchors.centerIn: parent; width: 10; height: 10; kind: "cross"; color: unpin.hovered ? Theme.foreground : Theme.mutedFg }
                }
            }
        }
    }

    // search, open all / close all, pause
    Row {
        spacing: 8
        Field {
            id: search
            width: 360
            placeholderText: "Search " + (root.sv.total !== undefined ? root.sv.total : 0) + " signals…"
            onTextEdited: AppModel.systemQuery(text)
        }
        Button { text: root.sv.any_open ? "Close all" : "Open all"; onClicked: AppModel.openAllGroups(!root.sv.any_open) }
        Button { text: root.sv.running ? "Pause" : "Resume"; primary: !root.sv.running; onClicked: AppModel.setRunning(!root.sv.running) }
        Text {
            visible: !!root.sv.running && root.sv.polled > 0
            anchors.verticalCenter: parent.verticalCenter
            text: root.sv.polled + " group" + (root.sv.polled === 1 ? "" : "s") + " · " + (root.lv.ms || 0) + " ms per update"
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: 12 }
        }
    }

    Repeater {
        model: root.groups
        Column {
            id: group
            required property var modelData
            width: root.width

            Rectangle {
                width: parent.width; height: 32
                radius: Theme.rRow
                color: ghover.hovered ? Theme.secondary : Theme.muted
                Glyph { x: 12; anchors.verticalCenter: parent.verticalCenter; kind: group.modelData.open ? "down" : "right"; color: Theme.mutedFg }
                Row {
                    x: 30; anchors.verticalCenter: parent.verticalCenter; spacing: 8
                    Text { text: "Data group"; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13; weight: Font.DemiBold } }
                    Text { text: group.modelData.hex; color: Theme.foreground; font { family: Theme.mono; pixelSize: 13; weight: Font.DemiBold } }
                    Text { text: group.modelData.count + (group.modelData.count === 1 ? " signal" : " signals"); color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                }
                Text {
                    visible: !!(root.lv.err && root.lv.err[group.modelData.did])
                    anchors { right: parent.right; rightMargin: 12; verticalCenter: parent.verticalCenter }
                    text: (root.lv.err && root.lv.err[group.modelData.did]) || ""
                    color: Theme.fault
                    font { family: Theme.sans; pixelSize: 12 }
                }
                Tappable {
                    id: ghover
                    anchors.fill: parent
                    text: "Data group " + group.modelData.hex + (group.modelData.open ? ", open" : ", closed")
                    onClicked: AppModel.toggleGroup(group.modelData.did)
                }
            }
            Repeater {
                model: group.modelData.open ? group.modelData.signals : []
                Rectangle {
                    id: row
                    required property var modelData
                    readonly property var f: root.sig(modelData)
                    width: group.width; height: 38
                    color: rhover.hovered ? Theme.muted : "transparent"
                    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                    Column {
                        x: Theme.inset; anchors.verticalCenter: parent.verticalCenter
                        width: spark.x - Theme.inset - 12
                        Text { width: parent.width; elide: Text.ElideRight; text: row.modelData.name; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                        Text {
                            visible: !!row.f.note
                            text: row.f.note ? row.f.note : ""
                            color: Theme.mutedFg
                            font { family: Theme.sans; pixelSize: 11 }
                        }
                    }
                    // the trace sits against the value it draws; the values line up on the right in one column
                    Spark { id: spark; anchors { right: reading.left; rightMargin: 24; verticalCenter: parent.verticalCenter } width: 140; height: 20; values: row.f.hist || [] }
                    Item {
                        id: reading
                        anchors { right: pin.left; rightMargin: 8; top: parent.top; bottom: parent.bottom }
                        width: 134
                    }
                    Text {   // figures end on one edge, units in their own column after it
                        id: figure
                        anchors { right: reading.right; rightMargin: 54; verticalCenter: parent.verticalCenter }
                        text: row.f.text || ""
                        color: row.f.state === "on" ? Theme.ok : row.f.state === "off" ? Theme.mutedFg : Theme.tone(row.f.tone || 0)
                        font { family: Theme.mono; pixelSize: 14; weight: Font.DemiBold }
                    }
                    Text {
                        anchors { left: figure.right; leftMargin: 6; baseline: figure.baseline }
                        width: 48; elide: Text.ElideRight
                        text: row.f.unit || ""
                        color: Theme.mutedFg
                        font { family: Theme.sans; pixelSize: 12 }
                    }
                    Tappable {
                        id: pin
                        anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter }
                        width: 44; height: 24
                        background: Item {}   // the face below draws the focus ring
                        text: "Pin " + row.modelData.name
                        hint: "Show as a gauge"
                        onClicked: AppModel.togglePin(row.modelData.did, row.modelData.i)
                        ControlFace { anchors.fill: parent; hovered: pin.hovered; down: pin.down; focusRing: pin.visualFocus }
                        Text {
                            anchors.centerIn: parent
                            anchors.verticalCenterOffset: pin.down ? 1 : 0
                            text: "Pin"
                            color: Theme.foreground
                            font { family: Theme.sans; pixelSize: 12 }
                        }
                    }
                    HoverHandler { id: rhover }
                }
            }
        }
    }
}
