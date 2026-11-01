import QtQuick
import com.hondascan.app

// The open system's bi-directional tests and write functions: a card per item, and the confirmation before any of them runs.
Column {
    id: root
    property string kind: "tests"      // "tests" or "writes"
    spacing: 12

    readonly property var a: JSON.parse(AppModel.actuator)
    readonly property var w: JSON.parse(AppModel.writes)
    readonly property bool isTests: kind === "tests"
    readonly property var info: isTests ? a : w
    readonly property var items: isTests ? (a.tests ? a.tests : []) : (w.writes ? w.writes : [])

    property var asking: null          // {item, value?}

    Text {
        width: parent.width; wrapMode: Text.WordWrap
        text: root.items.length === 0 ? (root.info.none ? root.info.none : "") : (root.info.intro ? root.info.intro : "")
        color: Theme.mutedFg
        font { family: Theme.sans; pixelSize: 13 }
    }

    Repeater {
        model: root.items
        Rectangle {
            id: card
            required property var modelData
            width: root.width
            height: col.implicitHeight + 32
            gradient: Theme.cardFill
            radius: Theme.rCard; border.color: Theme.edge; border.width: 1
            Column {
                id: col
                x: Theme.inset; y: 16; width: parent.width - 2 * Theme.inset; spacing: 6
                Item {
                    width: parent.width; height: 32
                    Text { id: name; anchors.verticalCenter: parent.verticalCenter; text: card.modelData.name; color: Theme.foreground; font { family: Theme.sans; pixelSize: 16; weight: Font.DemiBold } }
                    Text { id: meta; anchors { left: name.right; leftMargin: 10; baseline: name.baseline } text: root.isTests ? card.modelData.did + " · " + card.modelData.hold : card.modelData.when; color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 11 } }
                    Text { visible: !root.isTests && !!card.modelData.no_stop; anchors { left: meta.right; leftMargin: 10; baseline: name.baseline } text: card.modelData.no_stop || ""; color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 11 } }
                    Row {
                        anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                        spacing: 8
                        Repeater {
                            model: root.isTests ? card.modelData.values : [ { label: "Run…" } ]
                            Button {
                                required property var modelData
                                text: modelData.label
                                enabled: root.isTests ? !root.a.busy : (card.modelData.can_run && AppModel.writeRunning === "")
                                onClicked: { root.asking = { item: card.modelData, value: modelData }; dialog.open() }
                            }
                        }
                    }
                }
                Caption { text: root.isTests ? "Must hold before it starts" : "HDS checks before it will write" }
                Repeater { model: card.modelData.conditions; Row { required property string modelData; spacing: 8
                    Text { text: "•"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                    Text { width: col.width - 16; text: parent.modelData; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } } } }
                Text { visible: root.isTests && !!card.modelData.effect; text: card.modelData.effect || ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Text { visible: !root.isTests; text: card.modelData.read_back || ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
                Text { text: "Where this comes from: " + card.modelData.source; width: col.width; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 11 } }
            }
        }
    }

    ConfirmDialog {
        id: dialog
        readonly property var it: root.asking ? root.asking.item : null
        readonly property var v: root.asking ? root.asking.value : null
        title: !it ? "" : root.isTests ? v.title : it.dialog.title
        body: !it ? "" : root.isTests ? v.body : it.dialog.body
        lines: !it || root.isTests ? [] : [ it.dialog.sends ]
        notes: !it || root.isTests ? [] : [ it.dialog.reads_back, it.dialog.stop_note ].filter(s => s !== "")
        conditionsLead: !it || root.isTests ? "" : it.dialog.conditions_lead
        conditions: it ? it.conditions : []
        statusLine: !it || root.isTests ? "" : it.dialog.vin_line
        statusBad: !root.isTests && !root.w.vin_known
        tick: root.info.tick ? root.info.tick : ""
        confirmText: root.isTests ? "Start test" : "Run"
        blocked: !root.isTests && !root.w.vin_known
        onConfirmed: ticked => {
            if (root.isTests) AppModel.actStart(it.key, v.value, ticked)
            else AppModel.writeRun(it.key, ticked)
        }
    }
}
