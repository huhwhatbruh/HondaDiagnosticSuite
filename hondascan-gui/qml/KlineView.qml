import QtQuick
import com.hondascan.app

// A car on the K-line instead of CAN: the systems that answered (older body systems), each one's codes and live values, or, on a
// generic OBD-II adapter, the check engine light, the codes, the VIN and the live values. Reading only; nothing here clears.
Item {
    id: root
    implicitHeight: col.implicitHeight
    readonly property var kl: JSON.parse(AppModel.kline)
    readonly property bool busy: kl.busy !== ""
    readonly property var open: kl.open

    component Note: Text {
        x: Theme.inset; width: parent.width - 2 * Theme.inset; topPadding: 12; bottomPadding: 12
        wrapMode: Text.WordWrap
        color: Theme.mutedFg
        font { family: Theme.sans; pixelSize: 12 }
    }

    Column {
        id: col
        width: parent.width
        spacing: 12

        Note { visible: root.busy; text: root.kl.busy; color: Theme.foreground }
        Note { visible: root.kl.error !== ""; text: root.kl.error; color: Theme.fault }

        // -- VKWP systems ------------------------------------------------------------------------------------------------------
        Panel {
            visible: root.kl.mode === "kwp"
            width: parent.width
            title: "K-line systems"
            action: [ Button { text: "Look again"; enabled: !root.busy; onClicked: AppModel.klineScan() } ]
            Column {
                width: parent.width
                Note { visible: root.kl.scanned && root.kl.found.length > 0; text: "Addresses that answered, with the systems HDS lists at each. Choose one to read its codes and live values." }
                Note { visible: root.kl.scanned && root.kl.found.length === 0 && !root.busy; text: "No address answered on the K-line. Check that the ignition is ON and the adapter is on the car’s diagnostic port." }
                Repeater {
                    model: root.kl.found
                    Rectangle {
                        required property var modelData
                        readonly property bool chosen: root.open !== null && root.open.address === modelData.address && root.open.system === modelData.system
                        width: parent.width; height: 34
                        color: chosen ? Theme.accent : tap.hovered ? Theme.muted : "transparent"
                        Text { x: Theme.inset; anchors.verticalCenter: parent.verticalCenter; text: modelData.label; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                        Text { anchors { right: parent.right; rightMargin: Theme.inset; verticalCenter: parent.verticalCenter } text: modelData.system; color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 12 } }
                        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                        Tappable { id: tap; anchors.fill: parent; radius: 0; enabled: !root.busy; text: modelData.label; onClicked: AppModel.klineOpen(modelData.address, modelData.system) }
                    }
                }
            }
        }
        Panel {
            visible: root.kl.mode === "kwp" && root.open !== null
            width: parent.width
            title: root.open ? "Trouble codes · " + root.open.system : ""
            Column {
                width: parent.width
                Table { visible: !!root.open && root.open.codes && root.open.codes.length > 0; width: parent.width; headers: ["Code", "State", "Description"]; weights: [1, 1, 5]; rows: root.open && root.open.codes ? root.open.codes : [] }
                Note { visible: !!root.open && root.open.codesRead && root.open.codes.length === 0; text: "No trouble codes stored in this system." }
                Note { visible: !!root.open && !root.open.codesRead && !root.busy; text: "The codes were not read." }
            }
        }
        Panel {
            visible: root.kl.mode === "kwp" && root.open !== null
            width: parent.width
            title: root.open ? "Live data · " + root.open.system : ""
            Column {
                width: parent.width
                Table { visible: !!root.open && root.open.live && root.open.live.length > 0; width: parent.width; headers: ["Signal", "Value"]; weights: [3, 2]; rows: root.open && root.open.live ? root.open.live : [] }
                Note { visible: !!root.open && root.open.liveRead && root.open.live.length === 0; text: "HDS lists no live data for this system." }
                Note { visible: !!root.open && !root.open.liveRead && !root.busy; text: "The live values were not read." }
            }
        }

        // -- generic OBD-II ----------------------------------------------------------------------------------------------------
        Panel {
            visible: root.kl.mode === "obd"
            width: parent.width
            title: "Check engine light"
            action: [ Button { text: "Read again"; enabled: !root.busy; onClicked: AppModel.obdRead() } ]
            Column {
                width: parent.width
                Repeater {
                    model: root.kl.obd ? root.kl.obd.lights : []
                    Text { required property string modelData; x: Theme.inset; height: 34; verticalAlignment: Text.AlignVCenter; text: modelData; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                }
                Text { visible: root.kl.obd && root.kl.obd.vin !== ""; x: Theme.inset; height: 34; verticalAlignment: Text.AlignVCenter; text: root.kl.obd ? "VIN " + root.kl.obd.vin : ""; color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 13 } }
                Note { visible: root.kl.obd && root.kl.obd.lights.length === 0 && !root.busy; text: "No ECU answered. Check that the ignition is ON and the adapter is on the car’s diagnostic port." }
            }
        }
        Panel {
            visible: root.kl.mode === "obd" && root.kl.obd !== null
            width: parent.width
            title: "Trouble codes"
            Column {
                width: parent.width
                Table { visible: root.kl.obd && root.kl.obd.codes.length > 0; width: parent.width; headers: ["Code", "State", "Description"]; weights: [1, 1, 5]; rows: root.kl.obd ? root.kl.obd.codes : [] }
                Note { visible: root.kl.obd && root.kl.obd.codes.length === 0; text: "No stored or pending trouble codes." }
            }
        }
        Panel {
            visible: root.kl.mode === "obd" && root.kl.obd !== null && root.kl.obd.read
            width: parent.width
            title: "Live data"
            Table { width: parent.width; headers: ["Signal", "Value"]; weights: [3, 2]; rows: root.kl.obd && root.kl.obd.readings ? root.kl.obd.readings : [] }
        }
    }
}
