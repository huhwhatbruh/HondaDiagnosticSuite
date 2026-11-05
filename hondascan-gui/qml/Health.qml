import QtQuick
import com.hondascan.app

// The car's health at a glance: one row per found system with its trouble-code counts, and the car's totals. Every row opens
// the system. A system that was not read (or whose read failed) is never folded into a clean bill.
Panel {
    id: root
    signal open(string key)

    readonly property var s: JSON.parse(AppModel.health)
    readonly property var systems: s.systems ? s.systems : []
    readonly property bool reading: AppModel.scanning || Object.values(JSON.parse(AppModel.dtcs)).some(d => d.loading)

    title: "Health"
    note: systems.length === 0 ? "nothing found yet" : (systems.length - s.unknown) + " of " + systems.length + " systems read"

    function plural(n, one, many) { return n + " " + (n === 1 ? one : many) }
    function countsOf(h) {
        if (h.unknown) return "not read"
        const bits = []
        if (h.confirmed > 0) bits.push(h.confirmed + " confirmed")
        if (h.pending > 0) bits.push(h.pending + " pending")
        if (h.stored > 0) bits.push(h.stored + " stored")
        const c = bits.length === 0 ? "no codes" : bits.join(" · ")
        return h.not_run > 0 ? c + " · " + plural(h.not_run, "self-test", "self-tests") + " not run" : c
    }
    function rowColor(h) { return h.unknown ? Theme.mutedFg : h.confirmed > 0 ? Theme.fault : (h.pending > 0 || h.stored > 0) ? Theme.warn : Theme.ok }

    readonly property color totalTone: systems.length === 0 || s.unknown > 0 ? Theme.warn : s.faults > 0 ? Theme.fault : s.pending > 0 ? Theme.warn : Theme.ok
    readonly property string totalText: systems.length === 0 ? "No systems found"
        : s.unknown > 0 ? plural(s.unknown, "system", "systems") + " not read"
        : s.faults > 0 ? plural(s.faults, "confirmed fault", "confirmed faults")
        : s.pending > 0 ? plural(s.pending, "pending code", "pending codes")
        : reading ? "Reading…" : "All clear"
    readonly property string totalBits: {
        const b = []
        if (s.faults > 0) b.push(s.faults + " confirmed")
        if (s.pending > 0) b.push(s.pending + " pending")
        if (s.stored > 0) b.push(s.stored + " stored")
        if (s.not_run > 0) b.push(plural(s.not_run, "self-test", "self-tests") + " not run")
        return b.join(" · ")
    }

    Column {
        width: parent.width
        Item {
            width: parent.width; height: 48
            Text {
                x: Theme.inset; anchors.verticalCenter: parent.verticalCenter
                text: root.totalText
                color: root.totalTone
                font { family: Theme.sans; pixelSize: 20; weight: Font.DemiBold }
            }
            Text {
                anchors { right: parent.right; rightMargin: Theme.inset; verticalCenter: parent.verticalCenter }
                text: root.totalBits
                color: Theme.mutedFg
                font { family: Theme.sans; pixelSize: 12 }
            }
        }
        Rectangle { width: parent.width; height: 1; color: Theme.border }
        Flow {   // one tile per system, as many to a line as fit
            x: Theme.inset; width: parent.width - 2 * Theme.inset
            topPadding: 12; bottomPadding: 12
            spacing: 8
            readonly property int cols: Math.max(1, Math.floor((width + spacing) / 230))
            readonly property real cell: (width - (cols - 1) * spacing) / cols
            Repeater {
                model: root.systems
                Rectangle {
                    required property var modelData
                    width: parent.cell
                    height: 52
                    radius: Theme.rControl
                    color: hover.hovered ? Theme.secondary : Theme.muted
                    Behavior on color { ColorAnimation { duration: Theme.fast } }
                    Column {
                        anchors { left: parent.left; leftMargin: 12; right: parent.right; rightMargin: 10; verticalCenter: parent.verticalCenter }
                        spacing: 2
                        Text { width: parent.width; elide: Text.ElideRight; text: modelData.name; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13; weight: Font.Medium } }
                        Text { width: parent.width; elide: Text.ElideRight; text: root.countsOf(modelData); color: root.rowColor(modelData); font { family: Theme.sans; pixelSize: 12 } }
                    }
                    Tappable {
                        id: hover
                        anchors.fill: parent
                        radius: Theme.rControl
                        text: modelData.name + ": " + root.countsOf(modelData)
                        onClicked: root.open(modelData.key)
                    }
                }
            }
        }
        Rectangle { width: parent.width; height: 1; color: Theme.border }
        BaselineCard { width: parent.width; onFreezeRequested: freeze.open(); onMarkerRequested: marker.open() }
        Rectangle { width: parent.width; height: 1; color: Theme.border }
        ConditionCard { width: parent.width }
    }

    FreezeDialog { id: freeze }
    MarkerDialog { id: marker }
}
