import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Effects
import QtQuick.Shapes
import com.hondascan.app

// A 240-degree gauge for one live figure (`fig` as the model publishes it: v, text, unit, min, max, tone), drawn as Apple's
// gauges page describes one: a path for the range, an indicator where the value is, the range's two endpoints in text, and
// the path filled with a gradient (gauges.md, Best practices) that brightens toward the indicator. The name sits between the
// endpoints in the dial's opening, so the whole gauge is the dial and nothing hangs under it. No reading: no fill, a dash.
// Built on Qt 6.8 (the release container), so the fill is a filled ring segment, not a stroke gradient (6.12).
Item {
    id: root

    property string label
    property var fig: ({})
    property real fallbackMin: 0     // the scale when the figure carries none
    property real fallbackMax: 100

    readonly property real lo: fig.min !== undefined && fig.min !== null ? fig.min : fallbackMin
    readonly property real hi: fig.max !== undefined && fig.max !== null ? fig.max : fallbackMax
    readonly property bool has: fig.v !== undefined && fig.v !== null
    property real shown: has ? fig.v : lo
    Behavior on shown { SmoothedAnimation { velocity: (root.hi - root.lo) * 2.5 } }
    readonly property real valueFraction: Math.max(0, Math.min(1, (shown - lo) / (hi - lo)))
    // key-on sweep: the first reading after the car comes on runs the dial to full scale and back to the value, once
    property real swept: -1                       // the sweep's own position while it runs, else -1
    readonly property real fraction: swept >= 0 ? swept : valueFraction
    onHasChanged: if (has && Theme.ignition) keyOn.start()
    SequentialAnimation {
        id: keyOn
        NumberAnimation { target: root; property: "swept"; from: 0; to: 1; duration: 520; easing.type: Easing.InOutCubic }
        PauseAnimation { duration: 90 }
        NumberAnimation { target: root; property: "swept"; to: root.valueFraction; duration: 560; easing.type: Easing.OutCubic }
        ScriptAction { script: root.swept = -1 }
    }
    readonly property color tone: Theme.tone(fig.tone === undefined ? 0 : fig.tone)
    property color toneShown: tone
    Behavior on toneShown { ColorAnimation { duration: Theme.normal } }

    // The height a gauge of width `w` needs: callers size their cells with it so no space is left over.
    function fitHeight(w) { const r = w / (2 + k); return Math.ceil(r * (1.5 + k) + gap + lineFor(r)) }
    function lineFor(r) { return Math.max(15, Math.round(r * 0.17) + 4) }   // the endpoint row's height

    implicitWidth: 200
    implicitHeight: fitHeight(implicitWidth)

    // geometry: one centre and one radius for the track, the fill and the indicator, so they cannot drift apart
    readonly property real k: 0.11                 // stroke / radius
    readonly property real gap: 5                  // from the caps to the endpoint row
    readonly property real line: lineFor(rad)
    readonly property real start: 150              // degrees, clockwise from 3 o'clock: 8 o'clock round to 4 o'clock
    readonly property real sweep: 240
    readonly property real rad: Math.max(10, Math.min(width / (2 + k), (height - gap - 15) / (1.5 + k), (height - gap - 4) / (1.67 + k)))
    readonly property real stroke: rad * k
    readonly property real cx: width / 2
    readonly property real cy: (height - (rad * (1.5 + k) + gap + line)) / 2 + rad + stroke / 2
    readonly property real fillSweep: sweep * fraction
    function at(deg, r) { const a = deg * Math.PI / 180; return Qt.point(cx + r * Math.cos(a), cy + r * Math.sin(a)) }
    function abbrev(x) { return Math.abs(x) >= 1000 ? (x / 1000).toFixed(x % 1000 === 0 ? 0 : 1) + "k" : Math.round(x).toString() }

    Accessible.role: Accessible.Indicator
    Accessible.name: label + ": " + (has ? fig.text + " " + (fig.unit || "") : "no reading") + ", range " + abbrev(lo) + " to " + abbrev(hi)

    Shape {   // the range: never changes with the value, so it is tessellated once
        anchors.fill: parent
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeColor: Theme.secondary
            strokeWidth: root.stroke
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc { centerX: root.cx; centerY: root.cy; radiusX: root.rad; radiusY: root.rad; startAngle: root.start; sweepAngle: root.sweep }
        }
    }
    Repeater {   // the scale: a tick every fortieth of the range, every fifth one longer, as on a cluster's tachometer
        model: 41
        Rectangle {
            required property int index
            readonly property bool major: index % 5 === 0
            readonly property real a: root.start + root.sweep * index / 40
            readonly property real len: root.stroke * (major ? 0.62 : 0.32)
            readonly property point p: root.at(a, root.rad - root.stroke * 1.0 - len / 2)
            width: len; height: Math.max(1, root.rad * (major ? 0.016 : 0.01))
            x: p.x - width / 2; y: p.y - height / 2
            rotation: a
            antialiasing: true
            color: major ? Theme.mutedFg : Theme.tick
        }
    }
    Shape {   // up to the value: a ring segment with round ends, dim at the start of the range and full at the indicator
        anchors.fill: parent
        visible: root.has && root.fraction > 0.002
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            id: fill
            readonly property real ro: root.rad + root.stroke / 2
            readonly property real ri: root.rad - root.stroke / 2
            readonly property real cap: root.stroke / 2
            readonly property point outerEnd: root.at(root.start + root.fillSweep, ro)
            readonly property point innerEnd: root.at(root.start + root.fillSweep, ri)
            readonly property point outerStart: root.at(root.start, ro)
            strokeColor: "transparent"
            fillGradient: ConicalGradient {
                // the conical gradient turns the other way round from the path: position 0 is the indicator and it rises
                // back along the fill to the start of the range
                centerX: root.cx; centerY: root.cy
                angle: -(root.start + root.fillSweep)
                GradientStop { position: 0; color: root.toneShown }
                GradientStop { position: Math.max(0.001, root.fillSweep / 360); color: Qt.rgba(root.toneShown.r, root.toneShown.g, root.toneShown.b, 0.3) }
                GradientStop { position: 1; color: root.toneShown }
            }
            startX: outerStart.x; startY: outerStart.y
            PathAngleArc { centerX: root.cx; centerY: root.cy; radiusX: fill.ro; radiusY: fill.ro; startAngle: root.start; sweepAngle: root.fillSweep; moveToStart: false }
            PathArc { x: fill.innerEnd.x; y: fill.innerEnd.y; radiusX: fill.cap; radiusY: fill.cap }
            PathAngleArc { centerX: root.cx; centerY: root.cy; radiusX: fill.ri; radiusY: fill.ri; startAngle: root.start + root.fillSweep; sweepAngle: -root.fillSweep; moveToStart: false }
            PathArc { x: fill.outerStart.x; y: fill.outerStart.y; radiusX: fill.cap; radiusY: fill.cap }
        }
    }
    Rectangle {   // the indicator: a knob on the path, cut out of it by a ring in the card colour
        readonly property point p: root.at(root.start + root.fillSweep, root.rad)
        visible: root.has
        width: root.stroke * 1.55; height: width; radius: width / 2
        x: p.x - width / 2; y: p.y - height / 2
        color: root.toneShown
        border { color: Theme.card; width: Math.max(2, root.stroke * 0.28) }
        // it stands on the path: a short soft shadow, rendered once (the knob's look does not change, only where it is)
        layer.enabled: true
        layer.effect: MultiEffect { shadowEnabled: true; shadowColor: Theme.background; shadowOpacity: 0.9; shadowBlur: 0.5; shadowVerticalOffset: 2; blurMax: 12 }
    }

    Column {   // the reading, on the dial's centre
        x: root.cx - width / 2; y: root.cy - value.height * 0.62
        spacing: -Math.round(root.rad * 0.04)
        Text {
            id: value
            anchors.horizontalCenter: parent.horizontalCenter
            width: root.rad * 1.5
            horizontalAlignment: Text.AlignHCenter
            fontSizeMode: Text.HorizontalFit
            minimumPixelSize: 12
            text: root.fig.text !== undefined ? root.fig.text : "—"
            color: root.fig.tone ? root.toneShown : Theme.foreground
            font { family: Theme.display; pixelSize: Math.round(root.rad * 0.64); weight: Font.DemiBold; features: { "tnum": 1 } }
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: root.fig.unit !== undefined ? root.fig.unit : ""
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: Math.max(11, Math.round(root.rad * 0.15)) }
        }
    }

    // the endpoints under the two ends of the path, the name between them
    readonly property real rowY: cy + rad * 0.5 + stroke / 2 + gap
    Repeater {
        id: ends
        model: [0, 1]
        Text {
            required property int modelData
            readonly property point p: root.at(root.start + root.sweep * modelData, root.rad)
            x: p.x - width / 2; y: root.rowY
            text: root.abbrev(modelData === 0 ? root.lo : root.hi)
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: Math.max(11, Math.round(root.rad * 0.13)); features: { "tnum": 1 } }
        }
    }
    Text {
        x: root.cx - width / 2; y: root.rowY - 1
        // the opening between the two endpoints' inner edges, kept symmetric about the centre
        readonly property real wideEnd: Math.max(ends.itemAt(0) ? ends.itemAt(0).implicitWidth : 20, ends.itemAt(1) ? ends.itemAt(1).implicitWidth : 20)
        width: root.rad * 1.732 - wideEnd - 14
        horizontalAlignment: Text.AlignHCenter
        elide: Text.ElideRight
        text: root.label
        color: Theme.foreground
        opacity: 0.8
        font { family: Theme.sans; pixelSize: Math.max(11, Math.round(root.rad * 0.15)); weight: Font.Medium }
        HoverHandler { id: nameHover; enabled: parent.truncated }
        ToolTip.visible: nameHover.hovered
        ToolTip.delay: 500
        ToolTip.text: root.label
    }
}
