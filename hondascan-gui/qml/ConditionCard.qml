import QtQuick
import com.hondascan.app

// What the newest recorded drive says about the engine, against this car's own earlier drives: one row per check with its
// verdict, what was seen, why it matters, what to do and where the signal comes from. Reads and stores nothing by itself;
// the words are written in condition.rs.
Item {
    id: root
    readonly property var c: JSON.parse(AppModel.condition)
    readonly property var findings: c.findings ? c.findings : []
    implicitHeight: col.implicitHeight + 20

    function toneColor(t) { return t === 1 ? Theme.warn : t === 2 ? Theme.fault : t === 4 ? Theme.ok : Theme.mutedFg }

    Column {
        id: col
        x: Theme.inset; y: 10; width: parent.width - 2 * Theme.inset; spacing: 6

        Row {
            spacing: 10
            Caption { text: "Engine condition" }
            Text { visible: root.c.state === "ok"; text: root.c.earlier + " earlier drives · idle " + root.c.idle_min + " min · cruise " + root.c.cruise_min + " min"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
        }
        Text {
            visible: root.c.state === "ok"
            width: parent.width; wrapMode: Text.WordWrap
            text: root.c.headline ? root.c.headline : ""
            color: root.toneColor(root.c.tone); font { family: Theme.sans; pixelSize: 14; weight: Font.DemiBold }
        }
        Text {
            visible: root.c.state === "none" || root.c.state === "loading"
            width: parent.width; wrapMode: Text.WordWrap
            text: root.c.state === "loading" ? "Reading the recorded drives…" : (root.c.text ? root.c.text : "")
            color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 }
        }
        Text { width: parent.width; wrapMode: Text.WordWrap; text: root.c.gate ? root.c.gate : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }

        Repeater {
            model: root.findings
            Column {
                required property var modelData
                width: col.width; spacing: 2
                Row {
                    spacing: 10
                    Text { text: modelData.title; color: Theme.foreground; font { family: Theme.sans; pixelSize: 14; weight: Font.DemiBold } }
                    Text { text: modelData.verdict + (modelData.hypothesis ? " · hypothesis" : "") + " · " + modelData.confidence + " confidence"; color: root.toneColor(modelData.tone); font { family: Theme.sans; pixelSize: 12 } }
                }
                Text { width: parent.width; wrapMode: Text.WordWrap; text: "Saw: " + modelData.saw; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                Text { visible: modelData.tone === 1 || modelData.tone === 2; width: parent.width; wrapMode: Text.WordWrap; text: "Why it matters: " + modelData.why; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Text { visible: modelData.tone === 1 || modelData.tone === 2; width: parent.width; wrapMode: Text.WordWrap; text: "What to do: " + modelData.action; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Text { width: parent.width; wrapMode: Text.WordWrap; text: "Source: " + modelData.source; color: Theme.mutedFg; font { family: Theme.mono; pixelSize: 11 } }
            }
        }
    }
}
