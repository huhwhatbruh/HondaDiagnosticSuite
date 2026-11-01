import QtQuick
import QtQuick.Shapes
import com.hondascan.app

// A system's symbol, drawn on a 24-unit grid in one stroke weight so every symbol reads as one family (icons.md: one visual
// language). The shapes follow the dashboard telltales drivers already know where one exists (engine, ABS, tire pressure,
// airbag, steering, battery, parking brake); the rest are plain pictures of the part. `zone` is car.rs's zone name.
Item {
    id: root
    property string zone
    property color color: Theme.foreground
    property real weight: 1.6     // stroke, in grid units

    readonly property var paths: ({
        // engine block with its intake on top and the exhaust stub on the left: the check-engine telltale
        engine: "M9 5.5h6 M12 5.5V8 M5.5 10H7V8h8.5l2 2H19V8.5h1.5V16H19v-1.5h-2L14.5 18H9l-2-2H5.5z M3 11v4 M3 13h2.5",
        // the H of a gear lever's pattern
        transmission: "M6 5v14 M12 5v14 M18 5v7 M6 12h12 M6 5m-0.01 0h0.02 M12 5m-0.01 0h0.02 M18 5m-0.01 0h0.02",
        // a disc in its arcs: the ABS telltale without its letters
        brakes: "M12 6a6 6 0 1 0 0.01 0 M12 10a2 2 0 1 0 0.01 0 M4.2 6.5a9.5 9.5 0 0 0 0 11 M19.8 6.5a9.5 9.5 0 0 1 0 11",
        // a steering wheel: rim, hub and three spokes
        steering: "M12 3.5a8.5 8.5 0 1 0 0.01 0 M12 10a2 2 0 1 0 0.01 0 M3.8 10.5L10 12 M20.2 10.5L14 12 M12 14v6.5",
        // a seated person and the bag in front: the SRS telltale
        airbags: "M8 2.7a1.9 1.9 0 1 0 0.01 0 M8 8l1 6h4.5l1.2 5.5 M5.6 8.5l1 8.5H11 M16.5 5a3.5 3.5 0 1 0 0.01 0",
        // a dial with its needle
        meter: "M4 17a8 8 0 1 1 16 0 M12 17l4-5.5 M12 17m-0.01 0h0.02 M4 20h16",
        // a windscreen with the picture on it
        hud: "M3 17l2.5-10h13L21 17z M9 11h6 M10 13.5h4",
        // a tyre's section with the warning mark: the TPMS telltale
        tires: "M7 5C4.3 8.6 4.3 15.4 7 19l1.2 1.2h7.6L17 19c2.7-3.6 2.7-10.4 0-14 M8.5 20.2v1.5 M12 20.2v1.5 M15.5 20.2v1.5 M12 8v5.5 M12 16.2v0.1",
        // one node joined to three
        gateway: "M12 9.5a2.5 2.5 0 1 0 0.01 0 M5 4.5a1.6 1.6 0 1 0 0.01 0 M19 4.5a1.6 1.6 0 1 0 0.01 0 M12 18a1.6 1.6 0 1 0 0.01 0 M6.2 7.2l4 3.6 M17.8 7.2l-4 3.6 M12 14.5V18",
        // a camera body and its lens
        camera: "M3.5 8h4l1.5-2h6l1.5 2h4v10.5h-17z M12 10a3 3 0 1 0 0.01 0",
        // a speaker and its sound
        front: "M4 9.5h3.5L12 6v12l-4.5-3.5H4z M15.5 9a4 4 0 0 1 0 6 M18 6.5a7.5 7.5 0 0 1 0 11",
        // four wheels on two driven axles
        rear: "M5 4h3v5H5z M16 4h3v5h-3z M5 15h3v5H5z M16 15h3v5h-3z M8 6.5h8 M8 17.5h8 M12 6.5v11",
        // a battery with its terminals and signs: the charging telltale
        battery: "M3.5 8h17v11h-17z M6.5 8V6h3v2 M14.5 8V6h3v2 M6.5 13.5h3 M14.5 13.5h3 M16 12v3",
        // a bolt: the motor
        motor: "M13 3L6 13.5h5.5L10.5 21l7.5-11h-5.5z",
        // a P in a ring with its arcs: the parking-brake telltale
        parking: "M12 5.5a6.5 6.5 0 1 0 0.01 0 M10.2 15.5v-7h2.4a2.1 2.1 0 0 1 0 4.2h-2.4 M4.2 6.5a9.5 9.5 0 0 0 0 11 M19.8 6.5a9.5 9.5 0 0 1 0 11",
        // a lever's knob on its gate
        shifter: "M12 3.5a3 3 0 1 0 0.01 0 M12 9.5v7 M7 16.5h10l1 4H6z",
        // any other computer: a chip
        ecu: "M7 7h10v10H7z M10 3.5V7 M14 3.5V7 M10 17v3.5 M14 17v3.5 M3.5 10H7 M3.5 14H7 M17 10h3.5 M17 14h3.5"
    })

    implicitWidth: 24
    implicitHeight: 24

    Shape {
        width: 24; height: 24
        scale: root.width / 24
        transformOrigin: Item.TopLeft
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: root.color
            strokeWidth: root.weight
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: root.paths[root.zone] || root.paths.ecu }
        }
    }
}
