import QtQuick
import com.hondascan.app

// The warnings under the header, as inset notices; the screen below slides when one comes or goes. The backend decides what is
// a warning (hondascan/src/alerts.rs). A notice carries its status as a shape as well as a colour.
Item {
    id: root
    readonly property var alerts: { const s = JSON.parse(AppModel.status); return s.alerts ? s.alerts : [] }
    property var shown: []
    onAlertsChanged: if (alerts.length > 0) shown = alerts

    readonly property real row: 42   // a 34 px notice and 8 px between it and the next
    implicitHeight: alerts.length === 0 ? 0 : row * shown.length + 8
    height: implicitHeight
    Behavior on height { NumberAnimation { duration: 250; easing.type: Easing.OutCubic } }
    clip: true

    Repeater {
        model: root.shown
        Rectangle {
            required property int index
            required property var modelData
            readonly property bool fault: modelData.level === "fault"
            readonly property color tone: fault ? Theme.fault : Theme.warn
            x: 8; y: 8 + index * root.row
            width: root.width - 16; height: root.row - 8
            color: Qt.tint(Theme.background, Qt.rgba(tone.r, tone.g, tone.b, 0.12))
            radius: Theme.rControl
            border.color: Qt.tint(Theme.background, Qt.rgba(tone.r, tone.g, tone.b, 0.38))
            border.width: 1
            Glyph { x: 14; anchors.verticalCenter: parent.verticalCenter; width: 14; height: 14; visible: fault; kind: "cross"; color: tone }
            Text { x: 14; anchors.verticalCenter: parent.verticalCenter; visible: !fault; text: "!"; color: tone; font { family: Theme.sans; pixelSize: 16; weight: Font.DemiBold } }
            Text { x: 40; anchors.verticalCenter: parent.verticalCenter; width: parent.width - 54; elide: Text.ElideRight; text: modelData.text; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
            Accessible.role: Accessible.AlertMessage
            Accessible.name: modelData.title + ". " + modelData.text
        }
    }
}
