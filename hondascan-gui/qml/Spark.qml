import QtQuick
import QtQuick.Shapes
import com.hondascan.app

// A line of the last readings, scaled to their own range. A line that did not move is a plain bar (most signals sit still);
// only a moving one is a path, which costs a re-tessellation at every update.
Item {
    id: root
    property var values: []
    property color color: Theme.mutedFg

    readonly property var v: values ? values : []
    readonly property real lo: v.length > 0 ? Math.min.apply(null, v) : 0
    readonly property real hi: v.length > 0 ? Math.max.apply(null, v) : 1
    readonly property bool flat: v.length > 1 && hi === lo
    readonly property var points: {
        const n = v.length
        if (n < 2 || hi === lo) return []
        const span = hi - lo
        return v.map((x, i) => Qt.point(i * width / (n - 1), height - (x - lo) / span * (height - 2) - 1))
    }

    Rectangle {
        visible: root.flat
        y: root.height / 2; width: parent.width; height: 1.5
        color: root.color
    }
    Shape {
        anchors.fill: parent
        visible: root.points.length > 1
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: root.color
            strokeWidth: 1.5
            fillColor: "transparent"
            PathPolyline { path: root.points }
        }
    }
}
