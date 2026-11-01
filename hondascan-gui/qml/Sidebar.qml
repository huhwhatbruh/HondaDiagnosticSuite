import QtQuick
import QtQuick.Controls.Basic as C
import com.hondascan.app

// Navigation: the overview, and every system found with its health at a glance.
Rectangle {
    id: root
    color: Theme.panel
    property real corner: 8
    bottomLeftRadius: corner

    property string selected: "overview"   // "overview" or a system key
    signal pick(string key)

    readonly property var ecus: JSON.parse(AppModel.ecus)
    readonly property var codes: JSON.parse(AppModel.dtcs)

    function stateOf(key) {
        const d = codes[key]
        if (!d || d.loading || d.dtcs === null || d.dtcs === undefined) return { n: 0, tone: "unknown" }
        const n = d.dtcs.length
        return { n: n, tone: d.dtcs.some(c => c.kind === "confirmed") ? "fault" : n > 0 ? "warn" : "ok" }
    }
    function toneColor(t) { return t === "fault" ? Theme.fault : t === "warn" ? Theme.warn : t === "ok" ? Theme.ok : Theme.mutedFg }

    component NavRow: C.ItemDelegate {
        id: row
        property string key
        property string title
        property string sub
        property var badge: null    // {n, tone} or null
        readonly property bool active: root.selected === key
        width: root.width - 16
        height: sub !== "" ? 40 : 30
        padding: 0
        hoverEnabled: true
        text: title
        onClicked: root.pick(key)
        background: Rectangle {
            radius: Theme.rRow
            color: row.active ? Theme.secondary : row.hovered ? Theme.muted : "transparent"
            Behavior on color { ColorAnimation { duration: Theme.fast } }
            Rectangle {   // the keyboard focus ring
                anchors.fill: parent
                radius: Theme.rRow
                color: "transparent"
                border { color: Theme.focus; width: 2 }
                visible: row.visualFocus
            }
        }
        contentItem: Item {
            Column {
                anchors { left: parent.left; leftMargin: 12; verticalCenter: parent.verticalCenter }
                Text { text: row.title; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13; weight: row.active ? Font.DemiBold : Font.Normal } }
                Text { visible: row.sub !== ""; text: row.sub; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 11 } }
            }
            Row {
                visible: row.badge !== null
                anchors { right: parent.right; rightMargin: 12; verticalCenter: parent.verticalCenter }
                spacing: 7
                Text {   // the count where there are codes; a lit point always, so the state is a shape as well as a colour
                    visible: row.badge !== null && row.badge.n > 0
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.badge ? row.badge.n : ""
                    color: row.badge ? root.toneColor(row.badge.tone) : "transparent"
                    font { family: Theme.mono; pixelSize: 12; weight: Font.DemiBold }
                }
                Rectangle {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 7; height: 7; radius: row.badge && row.badge.tone === "fault" ? 1.5 : 3.5   // faults are square, the rest round
                    color: row.badge ? root.toneColor(row.badge.tone) : "transparent"
                    opacity: row.badge && row.badge.tone === "ok" ? 0.8 : 1
                }
            }
        }
        HoverHandler { cursorShape: Qt.PointingHandCursor }
    }

    Flickable {   // a car with many systems scrolls above the footer instead of running over it
        id: list
        anchors { fill: parent; margins: 8; bottomMargin: 48 }
        contentWidth: width
        contentHeight: items.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
    Column {
        id: items
        width: list.width
        spacing: 2
        NavRow { key: "overview"; title: "Overview" }
        NavRow { key: "history"; title: "History" }
        Item { width: 1; height: 12 }
        Item {
            width: parent.width; height: 20
            Caption { anchors { left: parent.left; leftMargin: 12; verticalCenter: parent.verticalCenter } text: "Systems" }   // in line with the rows' titles
            Text {
                anchors { right: parent.right; rightMargin: 12; verticalCenter: parent.verticalCenter }   // and with their badges
                text: AppModel.scanning ? "Scanning…" : root.ecus.length > 0 ? root.ecus.length : ""
                color: Theme.mutedFg
                font { family: Theme.sans; pixelSize: 11 }
            }
        }
        Repeater {
            model: root.ecus
            NavRow {
                required property var modelData
                key: modelData.key
                title: modelData.name
                sub: modelData.shared ? "Powertrain computer" : ""
                badge: root.stateOf(modelData.key)
            }
        }
        Text {
            visible: root.ecus.length === 0 && !AppModel.scanning
            width: parent.width
            topPadding: 24
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            text: "Connect and scan to list the car’s systems."
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: 12 }
        }
    }
    }
    Rectangle { anchors { left: parent.left; right: parent.right; bottom: parent.bottom; bottomMargin: 40 } height: 1; color: Theme.border }
    Check {   // the startup animation: on the first start of each day, unless this is unticked (no switch where the tree has no frames)
        visible: AppModel.introSequence !== ""
        anchors { left: parent.left; leftMargin: 12; bottom: parent.bottom; bottomMargin: 12 }   // in line with the rows' titles
        text: "Startup animation"
        checked: AppModel.introOn
        onToggled: AppModel.chooseIntro(checked)
    }
    Rectangle { anchors.right: parent.right; width: 1; height: parent.height; color: Theme.border }
}
