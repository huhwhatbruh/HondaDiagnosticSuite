import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// Add a service marker: a note kept with this car's history. It is written to the app's own record; nothing is sent to the car.
Sheet {
    id: root
    readonly property var kinds: JSON.parse(AppModel.maintenance).kinds || []
    property string kind: "oil"
    readonly property string problem: AppModel.markerCheck(kind, note.text, miles.text)
    width: 520
    padding: 20
    onAboutToShow: { kind = "oil"; note.text = ""; miles.text = "" }

    contentItem: Column {
        spacing: 8
        Text { text: "Add a service marker?"; color: Theme.foreground; font { family: Theme.sans; pixelSize: 16; weight: Font.DemiBold } }
        Text { width: parent.width; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 }
               text: "A note kept with this car’s history: what was done, when, and how far the car had gone. It is written to the app’s own record; nothing is sent to the car." }
        Text { text: "Kind"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        Flow {
            width: parent.width; spacing: 4
            Repeater {
                model: root.kinds
                Chip {
                    required property var modelData
                    text: modelData.label
                    on: root.kind === modelData.id
                    onClicked: root.kind = modelData.id
                }
            }
        }
        Item {
            width: parent.width; height: 16
            Text { text: "Note"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
            Text { anchors.right: parent.right; text: note.text.length + " / 2000"; color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 11 } }
        }
        Area { id: note; width: parent.width; height: 80; placeholderText: "5W-30 oil and filter, done at the dealer" }
        Text { text: "Mileage (optional, in miles)"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        Field { id: miles; width: 160; placeholderText: "82,400" }
        Text { visible: root.problem !== ""; width: parent.width; wrapMode: Text.WordWrap; text: root.problem; color: note.text === "" ? Theme.mutedFg : Theme.fault; font { family: Theme.sans; pixelSize: 12 } }
        Row {
            anchors.right: parent.right; spacing: 8
            Button { text: "Cancel"; onClicked: root.close() }
            Button { text: "Add marker"; primary: true; enabled: root.problem === ""; onClicked: { AppModel.addMarker(root.kind, note.text, miles.text); root.close() } }
        }
    }
}
