import QtQuick
import QtQuick.Effects
import QtQuick.Shapes
import com.hondascan.app

// The car seen straight down, nose to the right: the model's wires over the body outline, and every found system a button
// at its place on it, its symbol (SystemIcon) in the colour of its trouble-code state with a count. Where the buttons stand is decided in car.rs.
Panel {
    id: root
    signal open(string key)

    title: "Vehicle"
    note: "Select a system for its codes and live data"

    readonly property real sc: width / 1000
    readonly property real side: Math.max(64, 52 / sc)     // a button's cell is at least 52 px on screen (the disc 42, its symbol 23)
    readonly property var wires: JSON.parse(AppModel.carWires)
    readonly property var body: JSON.parse(AppModel.carBody)
    readonly property var codes: JSON.parse(AppModel.dtcs)
    readonly property var keys: { AppModel.ecus; return JSON.parse(AppModel.carKeys(side)) }

    function stateOf(key) {
        const d = codes[key]
        if (!d || d.loading || d.dtcs === null || d.dtcs === undefined) return { n: 0, color: Theme.mutedFg }
        const n = d.dtcs.length
        return { n: n, color: d.dtcs.some(c => c.kind === "confirmed") ? Theme.fault : n > 0 ? Theme.warn : Theme.ok }
    }

    // the pane's height with the drawing alone (Panel: the 40-point bar, its rule, the body, the foot); Overview makes the pane
    // as tall as the codes beside it, and the drawing is centred in what is left
    readonly property real naturalHeight: 41 + (width - 2) * 0.468 + 2 + Theme.rCard / 2

    Item {
        width: parent.width
        height: width * 0.468
        y: Math.max(0, (root.height - root.naturalHeight) / 2)
        clip: true

        // the frame: 1000 x 440 units, shown 14 units down (the viewBox 0 -14 1000 468)
        Item {
            id: frame
            y: 14 * root.sc
            scale: root.sc
            transformOrigin: Item.TopLeft
            width: 1000; height: 440

            Shape {   // the body outline (blueprint space, put in the frame by a matrix)
                x: root.body.b || 0; y: root.body.d || 0
                scale: root.body.a || 1
                transformOrigin: Item.TopLeft
                preferredRendererType: Shape.CurveRenderer
                ShapePath {
                    strokeColor: Qt.rgba(1, 1, 1, 0.35); strokeWidth: 1.4 / ((root.body.a || 1) * root.sc)
                    fillColor: Theme.card
                    PathSvg { path: root.body.path || "" }
                }
            }
            Repeater {
                model: root.wires
                Shape {
                    required property var modelData
                    anchors.fill: parent
                    preferredRendererType: Shape.CurveRenderer   // anti-aliased by its own coverage at any scale
                    ShapePath {
                        // plain light grey (0.85), the group's baked strength halved, lines 0.75 px thin
                        strokeColor: Qt.rgba(0.85, 0.85, 0.85, Math.min(1, modelData.strength * 0.4))
                        strokeWidth: 0.75 / root.sc
                        fillColor: "transparent"
                        PathSvg { path: modelData.path }
                    }
                }
            }

            Repeater {
                model: root.keys
                Item {
                    id: key
                    required property var modelData
                    readonly property var st: root.stateOf(modelData.key)
                    readonly property bool hot: tap.hovered || tap.visualFocus
                    x: modelData.x - root.side / 2; y: modelData.y - root.side / 2
                    width: root.side; height: root.side

                    // a raised disc over the drawing: lit from above, a soft shadow under it, lifted a little under the pointer;
                    // the system's symbol on it in the colour of its code state
                    Item {
                        id: disc
                        anchors.centerIn: parent
                        width: root.side * 0.8; height: width
                        scale: tap.down ? 0.96 : key.hot ? 1.06 : 1
                        Behavior on scale { NumberAnimation { duration: Theme.fast; easing.type: Easing.OutCubic } }
                        Rectangle {
                            id: face
                            anchors.fill: parent
                            radius: width / 2
                            gradient: Gradient {
                                GradientStop { position: 0; color: tap.down ? Theme.secondary : key.hot ? Theme.discHot : Theme.disc }
                                GradientStop { position: 1; color: tap.down ? Theme.accent : Theme.panel }
                            }
                            border.color: key.hot ? Qt.rgba(1, 1, 1, 0.45) : Qt.rgba(1, 1, 1, 0.10); border.width: 1 / root.sc
                            layer.enabled: true
                            layer.effect: MultiEffect {
                                shadowEnabled: true
                                shadowColor: Theme.background
                                shadowOpacity: key.hot ? 0.95 : 0.8
                                shadowBlur: 0.6
                                shadowVerticalOffset: (key.hot ? 5 : 3) / root.sc
                                blurMax: 24
                            }
                        }
                        SystemIcon {
                            anchors.centerIn: parent
                            width: parent.width * 0.56; height: width
                            zone: key.modelData.zone
                            color: key.st.color
                        }
                    }
                    Rectangle {
                        visible: key.st.n > 0
                        x: parent.width * 0.6; y: parent.height * 0.04
                        width: Math.max(17 / root.sc, cnt.implicitWidth + 8 / root.sc); height: 17 / root.sc; radius: height / 2
                        color: key.st.color
                        border { color: Theme.card; width: 2 / root.sc }
                        Text { id: cnt; anchors.centerIn: parent; text: key.st.n; color: Theme.background; font { family: Theme.sans; pixelSize: Math.round(11 / root.sc); weight: Font.Bold } }
                    }
                    Rectangle {   // the name, under the pointer
                        visible: key.hot
                        x: (parent.width - width) / 2
                        y: key.modelData.y > 300 ? -height - 4 : parent.height + 4   // above the disc where there is no room below it
                        width: nm.implicitWidth + 14; height: nm.implicitHeight + 8
                        radius: Theme.rControl; color: Theme.popover; border.color: Theme.outline; border.width: 1
                        z: 10
                        Text { id: nm; anchors.centerIn: parent; text: key.modelData.name; color: Theme.foreground; font { family: Theme.sans; pixelSize: Math.round(13 / root.sc) } }
                    }
                    Tappable {
                        id: tap
                        anchors.centerIn: parent
                        width: root.side * 0.8; height: width; radius: width / 2
                        text: key.modelData.name + (key.st.n > 0 ? ", " + key.st.n + (key.st.n === 1 ? " code" : " codes") : "")
                        onClicked: root.open(key.modelData.key)
                    }
                }
            }
        }
    }
}
