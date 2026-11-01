import QtQuick
import com.hondascan.app

// The first thing on the Overview: one sentence about how the engine is doing, and the numbers that tell you
// (engine speed, coolant, spark advance, knock).
Rectangle {
    id: root
    gradient: Theme.cardFill
    radius: Theme.rCard
    border.color: Theme.edge
    border.width: 1
    implicitHeight: col.implicitHeight + 46

    readonly property var ecus: JSON.parse(AppModel.ecus)
    readonly property var codeState: JSON.parse(AppModel.dtcs)
    readonly property var engine: JSON.parse(AppModel.engine)
    readonly property var alerts: { const s = JSON.parse(AppModel.status); return s.alerts ? s.alerts : [] }

    readonly property var codes: {
        let all = []
        for (const e of ecus) { const d = codeState[e.key]; if (d && d.dtcs) all = all.concat(d.dtcs) }
        return all
    }
    readonly property int confirmed: codes.filter(c => c.kind === "confirmed").length
    readonly property bool reading: AppModel.scanning || ecus.some(e => (codeState[e.key] || {}).loading === true)
    readonly property int unread: { const h = JSON.parse(AppModel.health); return h.unread ? h.unread.length : 0 }
    readonly property bool running: engine.NE !== undefined && engine.NE.v !== null && engine.NE.v > 400

    readonly property color tone: alerts.length > 0 ? (alerts[0].level === "fault" ? Theme.fault : Theme.warn)
        : confirmed > 0 ? Theme.fault : (codes.length > 0 || unread > 0) ? Theme.warn : Theme.ok
    readonly property var words: {
        if (alerts.length > 0) {
            const more = alerts.length > 1 ? " (" + (alerts.length - 1) + " more warning" + (alerts.length > 2 ? "s" : "") + " in the bar above.)" : ""
            return [alerts[0].title + ".", alerts[0].text + more]
        }
        if (reading) return ["Reading your car…", "Asking every system for its codes."]
        if (codes.length > 0) return [codes.length + " trouble code" + (codes.length > 1 ? "s" : "") + " to look at.", "Open one on the right for what Honda says can cause it."]
        if (unread > 0) return [unread + " system" + (unread > 1 ? "s" : "") + " did not give " + (unread > 1 ? "their" : "its") + " codes.",
                                "It is not known whether " + (unread > 1 ? "they hold" : "it holds") + " codes: open a system to read again."]
        if (running) return ["Nothing flagged right now.", "No active warnings or trouble codes — the bar above stays empty while all is well."]
        return ["Ignition on, engine off.", "Start it and these numbers come alive."]
    }

    HexMesh { anchors.fill: parent }

    Column {
        id: col
        x: 27; y: 22
        width: parent.width - 51
        spacing: 4

        Row {
            spacing: 14
            Item {   // the state as a lit point: its colour, a soft ring around it
                width: 14; height: 14
                anchors.verticalCenter: parent.verticalCenter
                Rectangle { anchors.centerIn: parent; width: 14; height: 14; radius: 7; color: root.tone; opacity: 0.22 }
                Rectangle { anchors.centerIn: parent; width: 8; height: 8; radius: 4; color: root.tone }
            }
            Text {
                width: col.width - 28
                elide: Text.ElideRight
                text: root.words[0]
                color: Theme.foreground
                font { family: Theme.sans; pixelSize: 28; weight: Font.DemiBold; letterSpacing: -0.4 }
            }
        }
        Text {
            x: 28
            width: col.width - 28
            wrapMode: Text.WordWrap
            text: root.words[1]
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: 14 }
        }
        Item { width: 1; height: 24 }

        // three dials and the knock meters share the width in equal cells, each centred in its own; the dials stay modest
        // (at most 196 wide): the car is the screen's picture
        Item {
            id: dials
            width: parent.width
            readonly property real cell: width / 4
            readonly property real dial: Math.min(cell - 20, 196)
            height: speed.fitHeight(dial)
            Gauge { id: speed; x: (dials.cell - width) / 2; width: dials.dial; height: parent.height; label: "Engine speed"; fig: root.engine.NE || ({}); fallbackMax: 8000 }
            Gauge { x: dials.cell + (dials.cell - width) / 2; width: dials.dial; height: parent.height; label: "Coolant"; fig: root.engine.ECT || ({}); fallbackMin: 100; fallbackMax: 260 }
            Gauge { x: 2 * dials.cell + (dials.cell - width) / 2; width: dials.dial; height: parent.height; label: "Spark advance"; fig: root.engine.IG || ({}); fallbackMin: -10; fallbackMax: 50 }
            Knock { x: 3 * dials.cell + 12; width: dials.cell - 12; height: parent.height; captionY: speed.rowY; engine: root.engine }
        }
    }

    // Knock: one level meter per cylinder where the car has them, else one for the single figure it reports for the engine.
    // A meter runs from 0 to the fault threshold, coloured in tiers as macOS level indicators are (gauges.md, Desktop):
    // plain below warnAt, amber from it, red at faultAt. Its caption sits on the dials' name row, so the four cells line up.
    component Knock: Item {
        id: k
        property var engine: ({})
        property real captionY: height - 18
        readonly property real warnAt: 2    // deg; hondascan.alerts' KNOCK_WARN and KNOCK_FAULT: a single reading is only coloured
        readonly property real faultAt: 6
        readonly property bool perCylinder: engine.KNRTD1 !== undefined && engine.KNRTD1.v !== null
        readonly property var figs: perCylinder ? [1, 2, 3, 4].map(n => engine["KNRTD" + n] || ({})) : [engine.KNRTD || ({})]
        function colorOf(v) { return v >= faultAt ? Theme.fault : v >= warnAt ? Theme.warn : Theme.foreground }

        Column {
            width: Math.min(parent.width, 220)
            y: (k.captionY - height) / 2
            spacing: k.perCylinder ? 14 : 10
            Text {   // the single figure, large, where there is no breakdown
                visible: !k.perCylinder
                text: k.figs[0].text !== undefined ? k.figs[0].text + "°" : "—"
                color: k.figs[0].v !== undefined && k.figs[0].v !== null ? k.colorOf(k.figs[0].v) : Theme.foreground
                font { family: Theme.sans; pixelSize: 40; weight: Font.DemiBold; features: { "tnum": 1 } }
            }
            Repeater {
                model: k.figs.length
                Item {
                    id: meter
                    required property int index
                    readonly property var f: k.figs[index]
                    readonly property bool has: f.v !== undefined && f.v !== null
                    property real shown: has ? Math.max(0, f.v) : 0
                    Behavior on shown { SmoothedAnimation { velocity: k.faultAt * 2.5 } }
                    width: parent.width; height: 16
                    Accessible.role: Accessible.Indicator
                    Accessible.name: (k.perCylinder ? "Cylinder " + (index + 1) : "Knock retard") + ": " + (has ? f.v.toFixed(1) + " degrees" : "no reading")
                    Text {
                        visible: k.perCylinder
                        anchors.verticalCenter: parent.verticalCenter
                        text: meter.index + 1
                        color: Theme.mutedFg
                        font { family: Theme.mono; pixelSize: 12 }
                    }
                    Rectangle {   // the track, with the warning threshold marked on it
                        id: track
                        x: k.perCylinder ? 18 : 0
                        width: parent.width - x - (k.perCylinder ? 52 : 0); height: 6; radius: 3
                        anchors.verticalCenter: parent.verticalCenter
                        color: Theme.secondary
                        Rectangle {
                            visible: meter.has && meter.shown > 0.01
                            width: Math.max(height, parent.width * Math.min(1, meter.shown / k.faultAt)); height: parent.height; radius: 3
                            color: k.colorOf(meter.shown)
                            Behavior on color { ColorAnimation { duration: Theme.normal } }
                        }
                        Rectangle { x: parent.width * k.warnAt / k.faultAt; y: -3; width: 1; height: 12; color: Theme.hover }
                    }
                    Text {
                        visible: k.perCylinder
                        anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                        text: meter.has ? meter.f.v.toFixed(1) + "°" : "—"
                        color: meter.has && meter.f.v >= k.warnAt ? k.colorOf(meter.f.v) : Theme.foreground
                        font { family: Theme.mono; pixelSize: 13; weight: meter.has && meter.f.v >= k.warnAt ? Font.DemiBold : Font.Normal }
                    }
                }
            }
        }
        Text {
            y: k.captionY - 1
            text: "Knock retard"
            color: Theme.foreground
            opacity: 0.8
            font { family: Theme.sans; pixelSize: 13; weight: Font.Medium }
        }
    }
}
