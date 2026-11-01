import QtQuick
import com.hondascan.app

// One system: what it is, its live data and its trouble codes (reference and the tests come with the next screens).
Column {
    id: root
    property string key
    spacing: 12

    readonly property var ecu: JSON.parse(AppModel.ecus).find(e => e.key === key)
    readonly property var d: JSON.parse(AppModel.dtcs)[key]
    property string tab: "live"
    readonly property var tabs: {
        const t = [ { id: "live", label: "Live data" }, { id: "codes", label: "Codes" }, { id: "reference", label: "Reference" } ]
        const a = JSON.parse(AppModel.actuator), w = JSON.parse(AppModel.writes)
        if (a.tests && a.tests.length > 0) t.push({ id: "tests", label: "Tests" })
        if (w.writes && w.writes.length > 0) t.push({ id: "writes", label: "Writes" })
        return t
    }

    // live reads run only while this screen is the one showing
    readonly property bool active: visible && key !== "" && key !== "overview" && key !== "history"
    onActiveChanged: active ? AppModel.watchSystem(key) : AppModel.unwatchSystem()
    onKeyChanged: if (active) {
        AppModel.watchSystem(key)
        tab = d && d.dtcs && d.dtcs.length > 0 ? "codes" : "live"
    }

    Rectangle {
        width: parent.width; height: 64
        gradient: Theme.cardFill
        radius: Theme.rCard; border.color: Theme.edge; border.width: 1
        Column {
            x: Theme.inset; anchors.verticalCenter: parent.verticalCenter; spacing: 2
            Text { text: root.ecu ? root.ecu.name : "System"; color: Theme.foreground; font { family: Theme.sans; pixelSize: 18; weight: Font.DemiBold } }
            Text { text: root.ecu ? root.ecu.blurb : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        }
        Text {
            anchors { right: parent.right; rightMargin: Theme.inset; verticalCenter: parent.verticalCenter }
            text: root.ecu ? root.ecu.system_type + " · address 0x" + root.ecu.address.toString(16).toUpperCase() : ""
            color: Theme.mutedFg
            font { family: Theme.mono; pixelSize: 12 }
        }
    }

    // a tab view (tab-views.md): the tabbed control on the top edge of an enclosed content area, the panes inset inside it
    Item {
        id: tabview
        width: parent.width
        height: pane.y + pane.height + 16
        Rectangle {
            y: tabs.height / 2
            width: parent.width; height: parent.height - y
            radius: Theme.rCard
            color: "transparent"
            border { color: Theme.edge; width: 1 }
        }
        Segmented { id: tabs; anchors.horizontalCenter: parent.horizontalCenter; items: root.tabs; current: root.tab; onPicked: id => root.tab = id }
        Column {
            id: pane
            x: 16; y: tabs.height + 14
            width: parent.width - 32
            spacing: 12
            LivePanel { visible: root.tab === "live"; width: parent.width }
            ReferenceTab { visible: root.tab === "reference"; width: parent.width }
            OpsTabs { visible: root.tab === "tests"; kind: "tests"; width: parent.width }
            OpsTabs { visible: root.tab === "writes"; kind: "writes"; width: parent.width }

            Panel {
                visible: root.tab === "codes"
                width: parent.width
                title: "Trouble codes"
                note: root.ecu ? root.ecu.name : ""
                action: [ Button { text: root.d && root.d.loading ? "Reading…" : "Read codes"; enabled: !(root.d && root.d.loading); onClicked: AppModel.readCodes(root.key) } ]

                Column {
                    width: parent.width
                    Text {
                        x: Theme.inset; topPadding: 12; bottomPadding: 12; width: parent.width - 2 * Theme.inset; wrapMode: Text.WordWrap
                        visible: !root.d || (!root.d.loading && !root.d.dtcs)
                        text: root.d && root.d.error ? "Could not read: " + root.d.error : "Codes not read yet. Press Read codes."
                        color: root.d && root.d.error ? Theme.fault : Theme.mutedFg
                        font { family: Theme.sans; pixelSize: 12 }
                    }
                    Repeater {
                        model: root.d && root.d.dtcs ? root.d.dtcs : []
                        Rectangle {
                            required property int index
                            required property var modelData
                            width: pane.width - 2
                            height: 40 + desc.implicitHeight
                            color: "transparent"
                            Rectangle { x: Theme.inset + 1; y: 17; width: 6; height: 6; radius: 3; color: modelData.kind === "confirmed" ? Theme.fault : Theme.warn }
                            Text { x: Theme.inset + 18; y: 8; text: modelData.full; color: Theme.foreground; font { family: Theme.mono; pixelSize: 13; weight: Font.DemiBold } }
                            Text { x: Theme.inset + 18 + 110; y: 9; text: modelData.kind; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                            Text { id: desc; x: Theme.inset + 18; y: 30; width: parent.width - 2 * Theme.inset - 18; text: modelData.text; color: Theme.mutedFg; wrapMode: Text.WordWrap; font { family: Theme.sans; pixelSize: 13 } }
                            Rectangle { visible: index < root.d.dtcs.length - 1; anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                        }
                    }
                    Text {
                        visible: !!(root.d && root.d.dtcs && root.d.dtcs.length === 0)
                        x: Theme.inset; topPadding: 12; bottomPadding: 12
                        text: "No stored or pending codes."
                        color: Theme.mutedFg
                        font { family: Theme.sans; pixelSize: 12 }
                    }
                }
            }

            Panel {
                id: selftests
                readonly property var st: JSON.parse(AppModel.selftests)
                visible: root.tab === "codes" && st !== null
                width: parent.width
                title: "Self-tests"
                Column {
                    width: parent.width; topPadding: 8; bottomPadding: 10; spacing: 4
                    Text { x: Theme.inset; text: selftests.st ? selftests.st.headline : ""; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                    Text { x: Theme.inset; width: parent.width - 2 * Theme.inset; wrapMode: Text.WordWrap; text: selftests.st ? selftests.st.note : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                    Repeater {
                        model: selftests.st ? selftests.st.lists : []
                        Column {
                            id: list
                            required property var modelData
                            x: Theme.inset; width: parent.width - 2 * Theme.inset; spacing: 2; topPadding: 6
                            Caption { text: list.modelData.title }
                            Repeater {
                                model: list.modelData.items
                                Row {
                                    required property var modelData
                                    spacing: 12
                                    Text { text: modelData.code; color: list.modelData.passed ? Theme.ok : Theme.mutedFg; font { family: Theme.mono; pixelSize: 12 } }
                                    Text { text: modelData.text; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
