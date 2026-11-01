import QtQuick
import QtQuick.Shapes
import com.hondascan.app

// A small symbol drawn as a path, so it looks the same on every machine (the bundled fonts have none of these shapes).
// `kind`: "check", "cross", "down" (a disclosure arrow open) or "right" (closed). Sized by `width`; the stroke scales with it.
Item {
    id: root
    property string kind: "check"
    property color color: Theme.foreground
    implicitWidth: 12
    implicitHeight: 12
    readonly property var paths: ({
        check: "M2 6.4L4.9 9.2L10 3",
        cross: "M2.5 2.5L9.5 9.5 M9.5 2.5L2.5 9.5",
        down: "M2.6 4.4L6 7.8L9.4 4.4",
        right: "M4.4 2.6L7.8 6L4.4 9.4"
    })
    Shape {
        width: 12; height: 12
        scale: root.width / 12
        transformOrigin: Item.TopLeft
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: root.color
            strokeWidth: 1.5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: root.paths[root.kind] || "" }
        }
    }
}
