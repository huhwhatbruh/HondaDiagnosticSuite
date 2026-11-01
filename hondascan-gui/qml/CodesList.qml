import QtQuick
import com.hondascan.app

// All trouble codes of every system, in one list; nothing is read until the person chooses the systems (ReadCodesDialog).
Panel {
    id: root
    signal open(string key)
    signal readRequested

    readonly property var ecus: JSON.parse(AppModel.ecus)
    readonly property var state: JSON.parse(AppModel.dtcs)
    readonly property var rows: {
        let all = []
        for (const e of ecus) {
            const d = state[e.key]
            if (d && d.dtcs) for (const c of d.dtcs) all.push({ e: e, c: c })
        }
        return all
    }
    readonly property int confirmed: rows.filter(r => r.c.kind === "confirmed").length
    readonly property bool reading: AppModel.scanning || ecus.some(e => (state[e.key] || {}).loading === true)
    readonly property var unreadNames: { const h = JSON.parse(AppModel.health); return h.unread ? h.unread : [] }

    title: "Trouble codes"
    note: ecus.length + (ecus.length === 1 ? " system" : " systems")
    action: [ Button { text: "Read codes"; enabled: root.ecus.length > 0; onClicked: root.readRequested() } ]

    readonly property color tone: reading ? Theme.mutedFg : confirmed > 0 ? Theme.fault : (rows.length > 0 || unreadNames.length > 0) ? Theme.warn : Theme.ok
    readonly property string headline: reading ? "Reading…"
        : rows.length === 0 && unreadNames.length === 0 ? "All clear"
        : rows.length === 0 ? unreadNames.length + " not read"
        : rows.length + " code" + (rows.length > 1 ? "s" : "")

    Column {
        width: parent.width
        Item {
            width: parent.width; height: 48
            Text { x: Theme.inset; anchors.verticalCenter: parent.verticalCenter; text: root.headline; color: root.tone; font { family: Theme.sans; pixelSize: 20; weight: Font.DemiBold } }
            Text {
                anchors { right: parent.right; rightMargin: Theme.inset; verticalCenter: parent.verticalCenter }
                visible: root.rows.length > 0
                text: root.confirmed + " confirmed · " + (root.rows.length - root.confirmed) + " pending"
                color: Theme.mutedFg
                font { family: Theme.sans; pixelSize: 12 }
            }
        }
        Rectangle { width: parent.width; height: 1; color: Theme.border }
        Repeater {
            model: root.rows
            Rectangle {
                required property var modelData
                width: root.width - 2
                height: 34 + desc.implicitHeight
                color: tap.hovered ? Theme.muted : "transparent"
                Rectangle { x: Theme.inset + 1; y: 17; width: 6; height: 6; radius: 3; color: modelData.c.kind === "confirmed" ? Theme.fault : Theme.warn }
                Text { x: Theme.inset + 18; y: 8; text: modelData.c.full; color: Theme.foreground; font { family: Theme.mono; pixelSize: 13; weight: Font.DemiBold } }
                Text { anchors.right: parent.right; anchors.rightMargin: Theme.inset; y: 9; text: modelData.e.name; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Text { id: desc; x: Theme.inset + 18; y: 30; width: parent.width - 2 * Theme.inset - 18; text: modelData.c.text; color: Theme.mutedFg; wrapMode: Text.WordWrap; font { family: Theme.sans; pixelSize: 13 } }
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                Tappable { id: tap; anchors.fill: parent; radius: 0; text: modelData.c.full + ", " + modelData.e.name; onClicked: root.open(modelData.e.key) }
            }
        }
        Text {
            visible: root.unreadNames.length > 0 && !root.reading
            x: Theme.inset; width: parent.width - 2 * Theme.inset; topPadding: 12
            wrapMode: Text.WordWrap
            text: "Codes not read yet: " + root.unreadNames.join(", ") + ". Nothing is read until you choose the systems."
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: 12 }
        }
        Text {
            visible: root.rows.length === 0 && root.unreadNames.length === 0 && !root.reading
            x: Theme.inset; topPadding: 12
            text: "No stored or pending codes in any system."
            color: Theme.mutedFg
            font { family: Theme.sans; pixelSize: 12 }
        }
        Item { width: 1; height: 12 }
    }
}
