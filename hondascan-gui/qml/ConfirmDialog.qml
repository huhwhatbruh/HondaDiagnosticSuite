import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// The question asked before every actuator test and every write: what will happen, what must hold, and a box the person
// ticks. The confirm button is off until it is ticked (and while `blocked`); the Rust side refuses an unticked request too.
Sheet {
    id: root
    property string title
    property string body
    property var lines: []          // monospace facts: what is sent, what is read back
    property var notes: []          // quieter sentences
    property string conditionsLead: ""
    property var conditions: []
    property string statusLine: ""  // e.g. the VIN line
    property bool statusBad: false
    property string tick
    property string confirmText: "Start"
    property bool blocked: false
    property bool ticked: false
    signal confirmed(bool ticked)

    width: 520
    padding: 20
    onAboutToShow: ticked = false


    contentItem: Column {
        spacing: 8
        Text { width: parent.width; text: root.title; wrapMode: Text.WordWrap; color: Theme.foreground; font { family: Theme.sans; pixelSize: 16; weight: Font.DemiBold } }
        Text { width: parent.width; text: root.body; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
        Repeater { model: root.lines; Text { required property string modelData; width: root.width - 40; text: modelData; wrapMode: Text.WordWrap; color: Theme.foreground; font { family: Theme.mono; pixelSize: 11 } } }
        Repeater { model: root.notes; Text { required property string modelData; width: root.width - 40; text: modelData; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } } }
        Text { visible: root.conditionsLead !== ""; width: parent.width; text: root.conditionsLead; wrapMode: Text.WordWrap; color: Theme.foreground; font { family: Theme.sans; pixelSize: 12 } }
        Repeater { model: root.conditions; Row { required property string modelData; spacing: 8
            Text { text: "•"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
            Text { width: root.width - 56; text: parent.modelData; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } } } }
        Text { visible: root.statusLine !== ""; width: parent.width; text: root.statusLine; wrapMode: Text.WordWrap; color: root.statusBad ? Theme.fault : Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }

        Check {
            wrap: true
            width: parent.width
            text: root.tick
            checked: root.ticked
            onToggled: root.ticked = checked
        }

        Row {
            anchors.right: parent.right
            spacing: 8
            Button { text: "Cancel"; onClicked: root.close() }
            Button {
                text: root.confirmText
                primary: true
                enabled: root.ticked && !root.blocked
                onClicked: { root.confirmed(root.ticked); root.close() }
            }
        }
    }
}
