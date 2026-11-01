import QtQuick
import QtQuick.Effects

// The startup animation: the baked frames of the camera gliding in on the cluster (assets/intro, tools/bakeintro.py), played over
// the app while it starts behind, then dissolved into it. Any click or key skips it. Self-contained: the frames' folder, the
// window's corner radius, the sequence and the signals are all it knows of the app.
Item {
    id: root
    property url base: "qrc:/assets/intro/"
    property string sequence   // sequence.json's text (QML may not read it from the resources itself: XMLHttpRequest refuses local files, qrc included)
    property real corner: 12
    signal started
    signal failed(string reason)   // it cannot play: the sequence or a frame did not load (it then dissolves at once)
    signal dissolving   // the last frame is up and the app is about to show through
    signal finished

    visible: false
    focus: true

    // sequence.json: which file shows at each tick of `fps`; holds are one file over many ticks
    property var seq: []
    property int fps: 60
    property int files: 0
    property real tick: 0
    property bool ending: false
    readonly property real overlap: 0.3   // seconds: the dissolve starts this long before the glide ends, so the frames are still moving as the app shows through
    property real fadeAt: 0               // the tick it starts at
    onTickChanged: if (visible && !ending && seq.length > 0 && tick >= fadeAt) dissolve(450, false)
    readonly property int ring: 8   // frames kept decoded: the one on show and the ones coming
    readonly property int ahead: 5
    readonly property int wanted: seq.length === 0 ? 0 : seq[Math.min(seq.length - 1, Math.floor(tick))]   // the file this tick shows
    property int shown: -1   // the newest of those ready to draw, so a frame that is late is held back, not a hole
    onWantedChanged: { feed(); settle() }

    function pad(n) { return ("000" + n).slice(-3) }

    function load() {
        try {
            const j = JSON.parse(sequence)
            seq = j.seq; fps = j.fps; files = j.files
            fadeAt = Math.max(0, seq.indexOf(files - 1) - overlap * fps)   // the last file shows from the tick the glide ends; what follows is a hold
        } catch (e) { failed("the sequence did not parse: " + e); return false }
        if (seq.length === 0) { failed("the sequence is empty"); return false }
        return true
    }

    // point the ring's slots at the next few files, so each has decoded (off the main thread) by the time its tick comes
    function feed() {
        for (let k = 0; k <= ahead; ++k) {
            const f = wanted + k
            const slot = frames.itemAt(f % ring)
            if (f < files && slot && slot.file !== f) { slot.file = f; slot.source = base + pad(f) + ".jpg" }
        }
    }
    function settle() {
        for (let f = wanted; f > wanted - ring && f >= 0; --f) {
            const slot = frames.itemAt(f % ring)
            if (slot && slot.file === f && slot.status === Image.Ready) { shown = f; return }
        }
    }

    function start() {
        if (!load()) { finished(); return }
        shown = -1; ending = false; tick = 0; opacity = 1; visible = true
        forceActiveFocus()
        started()
        feed()
        play.restart()
    }

    // let the app show through over `ms`; `stop`: the frames stop with it (a skip), otherwise they play on under the fade
    function dissolve(ms, stop) {
        if (ending) return
        ending = true
        if (stop) play.stop()
        dissolving()
        fade.duration = ms
        fade.restart()
    }
    function skip() { if (visible) dissolve(200, true) }

    NumberAnimation { id: play; target: root; property: "tick"; from: 0; to: root.seq.length; duration: root.seq.length * 1000 / root.fps; onFinished: root.dissolve(450, true) }   // (only if the fade did not start: it does, earlier)
    NumberAnimation { id: fade; target: root; property: "opacity"; to: 0; easing.type: Easing.InOutQuad; onFinished: { play.stop(); root.visible = false; root.finished() } }

    Keys.onPressed: event => { skip(); event.accepted = true }
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onPressed: root.skip() }

    // the window is drawn with round corners: the picture is cut to them
    Item {
        anchors.fill: parent
        layer.enabled: true
        layer.effect: MultiEffect { maskEnabled: true; maskSource: mask }
        Rectangle { anchors.fill: parent; color: "#000000" }
        Repeater {
            id: frames
            model: root.ring
            Image {
                property int file: -1
                anchors.fill: parent
                asynchronous: true
                cache: false
                fillMode: Image.PreserveAspectCrop
                visible: file === root.shown
                onStatusChanged: {
                    if (status === Image.Ready) root.settle()
                    else if (status === Image.Error) { root.failed("frame failed to load: " + source); root.skip() }
                }
            }
        }
    }
    Item {
        id: mask
        anchors.fill: parent
        visible: false
        layer.enabled: true
        Rectangle { anchors.fill: parent; radius: root.corner; color: "#000000" }
    }
}
