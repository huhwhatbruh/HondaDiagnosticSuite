import QtQuick
import com.hondascan.app

// Before a car is connected: three steps, and the simulated car for trying the app.
Item {
    id: root
    readonly property var steps: [
        { t: "Plug in", d: "Connect the adapter to the car’s diagnostic port (under the dashboard, driver’s side)." },
        { t: "Ignition on", d: "Turn the key to ON (II). The engine may stay off." },
        { t: "Connect", d: "Pick the adapter above and press Connect. Hondascan finds every system by itself." }
    ]
    implicitHeight: col.implicitHeight

    Column {
        id: col
        width: Math.min(parent.width, 940)
        anchors.horizontalCenter: parent.horizontalCenter
        topPadding: 150
        spacing: 6

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: "Let’s look at your car"
            color: Theme.foreground
            font { family: Theme.sans; pixelSize: 22; weight: Font.DemiBold }
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: "Honda diagnostics: every system, its trouble codes and all live data."
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: 13 }
        }
        Item { width: 1; height: 8 }
        Row {
            id: stepsRow
            spacing: 12
            readonly property real tall: {   // the cards share the height of the tallest, which grows when the words wrap
                let m = 92
                for (let i = 0; i < steps.count; ++i) { const c = steps.itemAt(i); if (c) m = Math.max(m, c.need) }
                return m
            }
            Repeater {
                id: steps
                model: root.steps
                Rectangle {
                    required property int index
                    required property var modelData
                    readonly property real need: card.implicitHeight + 28
                    width: (col.width - 24) / 3
                    height: stepsRow.tall
                    gradient: Theme.cardFill
                    radius: Theme.rCard
                    border.color: Theme.edge
                    border.width: 1
                    Column {
                        id: card
                        x: 14; y: 14
                        width: parent.width - 28
                        spacing: 6
                        Row {
                            spacing: 8
                            Text { text: "0" + (index + 1); color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 12 } }
                            Text { text: modelData.t; color: Theme.foreground; font { family: Theme.sans; pixelSize: 14; weight: Font.DemiBold } }
                        }
                        Text { width: parent.width; text: modelData.d; color: Theme.mutedFg; wrapMode: Text.WordWrap; font { family: Theme.sans; pixelSize: 12 } }
                    }
                }
            }
        }
        Item { width: 1; height: 12 }
        SelfCheck { width: parent.width }
        Item { width: 1; height: 12 }
        Button {
            anchors.horizontalCenter: parent.horizontalCenter
            text: "No car nearby? Try the simulated one"
            enabled: AppModel.link === 0
            onClicked: AppModel.connectAdapter("sim:drive")
        }
    }
}
