import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// Save a reference snapshot: what it stores and what blocks it. The backend re-checks the gate when it runs.
Sheet {
    id: root
    readonly property var info: JSON.parse(AppModel.freezeInfo)
    width: 520
    padding: 20
    onAboutToShow: label.text = ""

    contentItem: Column {
        spacing: 8
        Text { text: "Save a reference snapshot?"; color: Theme.foreground; font { family: Theme.sans; pixelSize: 16; weight: Font.DemiBold } }
        Text {
            width: parent.width; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 }
            text: "This stores what the car reports right now (trouble codes, which systems answered, readiness and self-tests) as the reference the next health report is compared against: what this car looked like on the day. It is kept as it is: a snapshot can never be edited or deleted, only added to. Stored (history) codes are recorded as they are; a confirmed code, a pending code or an unread system blocks saving, because then there is no clean state to compare against."
        }
        Text { text: root.info.vin_line ? root.info.vin_line : ""; color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 12 } }
        Text { visible: !!root.info.not_run; width: parent.width; wrapMode: Text.WordWrap; text: root.info.not_run ? root.info.not_run : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        Text { text: "Label (optional)"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        Field { id: label; width: parent.width; placeholderText: "Reference snapshot" }
        Text { visible: !!root.info.blocked; width: parent.width; wrapMode: Text.WordWrap; text: root.info.blocked ? root.info.blocked : ""; color: Theme.fault; font { family: Theme.sans; pixelSize: 12 } }
        Row {
            anchors.right: parent.right; spacing: 8
            Button { text: "Cancel"; onClicked: root.close() }
            Button { text: "Save snapshot"; primary: true; enabled: !root.info.blocked; onClicked: { AppModel.freezeBaseline(label.text); root.close() } }
        }
    }
}
