import QtQuick
import com.hondascan.app

// On screen for as long as a test or a write runs, on every screen: what is running, and Stop / Cancel.
Column {
    id: root
    readonly property var act: JSON.parse(AppModel.act)
    readonly property bool testRunning: act !== null && act.text !== undefined
    readonly property bool writing: AppModel.writeRunning !== ""

    component Bar: Rectangle {
        id: bar
        property string text
        property string action
        signal pressed
        width: root.width; height: 36
        color: Theme.warn
        Text { x: 16; width: parent.width - btn.width - 40; elide: Text.ElideRight; anchors.verticalCenter: parent.verticalCenter; text: bar.text; color: Theme.background; font { family: Theme.sans; pixelSize: 13; weight: Font.DemiBold } }
        Button { id: btn; anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter } text: bar.action; primary: true; onClicked: bar.pressed() }
    }

    Bar { visible: root.testRunning; text: root.testRunning ? root.act.text : ""; action: "Stop now"; onPressed: AppModel.actStop() }
    Bar { visible: root.writing; text: "Writing: " + AppModel.writeRunning + " · the routine stops and the session is left if you cancel"; action: "Cancel"; onPressed: AppModel.writeCancel() }
}
