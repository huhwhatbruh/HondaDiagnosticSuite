import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// Which systems to read codes from. Every system starts ticked; nothing is read until Read is pressed.
Sheet {
    id: root
    width: 420
    padding: 0

    readonly property var ecus: JSON.parse(AppModel.ecus)
    readonly property var ticked: JSON.parse(AppModel.readChecked)
    readonly property int count: ecus.filter(e => ticked[e.key]).length


    contentItem: Column {
        spacing: 0
        Item {
            width: parent.width; height: 44
            Text { x: 16; anchors.verticalCenter: parent.verticalCenter; text: "Read trouble codes"; color: Theme.foreground; font { family: Theme.sans; pixelSize: 15; weight: Font.DemiBold } }
        }
        Text { x: 16; width: parent.width - 32; wrapMode: Text.WordWrap; text: "Choose the systems to ask. Reading codes changes nothing in the car."; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } bottomPadding: 10 }
        Repeater {
            model: root.ecus
            Check {
                required property var modelData
                x: 16; height: 32
                text: modelData.name
                checked: root.ticked[modelData.key] === true
                onToggled: AppModel.setChecked(modelData.key, checked)
            }
        }
        Item {
            width: parent.width; height: 56
            Row {
                anchors { right: parent.right; rightMargin: 16; verticalCenter: parent.verticalCenter }
                spacing: 8
                Button { text: root.count === root.ecus.length ? "Untick all" : "Tick all"; onClicked: AppModel.setAllChecked(root.count !== root.ecus.length) }
                Button { text: "Cancel"; onClicked: root.close() }
                Button {
                    text: "Read " + root.count + " system" + (root.count === 1 ? "" : "s")
                    primary: true
                    enabled: root.count > 0
                    onClicked: { AppModel.readSelected(); root.close() }
                }
            }
        }
    }
}
