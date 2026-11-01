import QtQuick
import com.hondascan.app

// Every recorded session of each car this machine knows, with the codes seen over time, the service record, the saved reference
// snapshots, the write audit, the last health report and the backup. What each card says is decided in history.rs.
Column {
    id: root
    spacing: 12

    readonly property var h: JSON.parse(AppModel.history)
    function cells(list) { return list.map(r => r.cells) }

    Text {
        visible: !!h.loading || !!h.empty
        width: parent.width; wrapMode: Text.WordWrap
        text: h.loading ? "Loading…" : (h.empty ? h.empty : "")
        color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 }
    }

    Column {
        visible: !h.loading && !h.empty
        width: parent.width; spacing: 12

        Panel {
            width: parent.width
            title: "History"; note: h.cars_count ? h.cars_count : ""
            Column {
                width: parent.width
                Flow {
                    x: Theme.inset; y: 0; width: parent.width - 2 * Theme.inset; spacing: 6; topPadding: 10; bottomPadding: 10
                    Repeater {
                        model: h.vins ? h.vins : []
                        Chip { required property var modelData; text: modelData.vin; on: modelData.selected; onClicked: AppModel.historySelect(modelData.vin) }
                    }
                }
                Text { visible: !!h.error; x: Theme.inset; width: parent.width - 2 * Theme.inset; bottomPadding: 10; wrapMode: Text.WordWrap; text: h.error ? h.error : ""; color: Theme.fault; font { family: Theme.sans; pixelSize: 12 } }
            }
        }

        Flow {
            visible: !!(h.trends && h.trends.length > 0)
            id: tiles
            width: parent.width; spacing: 12
            readonly property int cols: Math.max(1, Math.floor((width + spacing) / 230))
            Repeater {
                model: h.trends ? h.trends : []
                Rectangle {
                    required property var modelData
                    width: (tiles.width - (tiles.cols - 1) * tiles.spacing) / tiles.cols; height: 110; gradient: Theme.cardFill; radius: Theme.rCard; border.color: Theme.edge; border.width: 1
                    Text { x: Theme.inset; y: 10; text: modelData.name; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                    Row { x: Theme.inset; y: 28; spacing: 6
                        Text { id: tv; text: modelData.value; color: Theme.foreground; font { family: Theme.sans; pixelSize: 22; weight: Font.DemiBold; features: { "tnum": 1 } } }
                        Text { text: modelData.unit; color: Theme.mutedFg; anchors.baseline: tv.baseline; font { family: Theme.sans; pixelSize: 12 } } }
                    Spark { x: Theme.inset; y: 64; width: parent.width - 2 * Theme.inset; height: 24; values: modelData.values }
                    Text { x: Theme.inset; y: 90; width: parent.width - 2 * Theme.inset; elide: Text.ElideRight; text: modelData.note; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 11 } }
                }
            }
        }

        Panel {
            visible: !!h.sessions
            width: parent.width; title: "Sessions"
            Column {
                width: parent.width
                Table {
                    width: parent.width
                    headers: h.sessions ? h.sessions.headers : []
                    rows: h.sessions ? root.cells(h.sessions.rows) : []
                    weights: [2.2, 1, 1, 1, 0.8, 2]
                    tappable: true
                    selected: h.sessions && h.sessions.selected !== null ? h.sessions.rows.findIndex(r => r.id === h.sessions.selected) : -1
                    onRowTapped: i => AppModel.historySession(h.sessions.rows[i].id)
                }
                Column {
                    visible: !!h.open
                    x: Theme.inset; width: parent.width - 2 * Theme.inset; spacing: 4; topPadding: 12; bottomPadding: 12
                    Row { spacing: 10
                        Text { text: h.open ? h.open.title : ""; color: Theme.foreground; font { family: Theme.sans; pixelSize: 14; weight: Font.DemiBold } }
                        Button { visible: !!(h.open && h.open.folder); text: "Open folder"; onClicked: AppModel.openPath(h.open.folder) } }
                    Text { width: parent.width; wrapMode: Text.WordWrap; textFormat: Text.MarkdownText; text: h.open ? h.open.summary : ""; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                }
            }
        }

        Panel {
            visible: !!h.codes
            width: parent.width; title: "Trouble-code history"
            Table { width: parent.width; headers: h.codes ? h.codes.headers : []; rows: h.codes ? root.cells(h.codes.rows) : []; weights: [1, 1.4, 3, 1.6, 1.6, 0.8, 1] }
        }

        Panel {
            visible: !!h.markers
            width: parent.width; title: "Service record"; note: h.markers ? h.markers.count : ""
            Column {
                width: parent.width
                Text { visible: !!(h.markers && h.markers.empty); x: Theme.inset; topPadding: 10; bottomPadding: 10; text: h.markers && h.markers.empty ? h.markers.empty : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
                Repeater {
                    model: h.markers ? h.markers.rows : []
                    Rectangle {
                        required property var modelData
                        width: parent.width; height: 34; color: "transparent"
                        Text { x: Theme.inset; anchors.verticalCenter: parent.verticalCenter; text: modelData.when; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                        Text { x: 190; anchors.verticalCenter: parent.verticalCenter; text: modelData.kind; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13; weight: Font.DemiBold } }
                        Text { x: 280; width: parent.width - 530; anchors.verticalCenter: parent.verticalCenter; elide: Text.ElideRight; text: modelData.note; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } }
                        Text { anchors { right: rm.left; rightMargin: 16; verticalCenter: parent.verticalCenter } text: modelData.miles; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                        Button { id: rm; anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter } text: "Remove"; onClicked: AppModel.removeMarker(modelData.id) }
                        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
                    }
                }
            }
        }

        Panel {
            visible: !!h.baselines
            width: parent.width; title: "Reference snapshots"; note: h.baselines ? h.baselines.count : ""
            Column {
                width: parent.width
                Text { visible: !!(h.baselines && h.baselines.empty); x: Theme.inset; topPadding: 10; bottomPadding: 10; text: h.baselines && h.baselines.empty ? h.baselines.empty : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
                Table {
                    visible: !!(h.baselines && h.baselines.rows.length > 0)
                    width: parent.width
                    headers: ["Saved", "Label", "Systems", "Integrity"]
                    rows: h.baselines ? h.baselines.rows.map(b => [ { text: b.saved }, { text: b.label }, { text: String(b.systems) }, { text: b.integrity, tone: b.tone } ]) : []
                    weights: [2, 2, 1, 1]
                }
            }
        }

        Panel {
            visible: !!h.audit
            width: parent.width; title: "Write audit"; note: h.audit ? h.audit.count : ""
            Column {
                width: parent.width
                Text { visible: !!(h.audit && h.audit.empty); x: Theme.inset; width: parent.width - 2 * Theme.inset; topPadding: 10; bottomPadding: 10; wrapMode: Text.WordWrap; text: h.audit && h.audit.empty ? h.audit.empty : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 } }
                Table {
                    visible: !!(h.audit && h.audit.rows.length > 0)
                    width: parent.width
                    headers: ["Time", "System", "Function", "Outcome"]
                    rows: h.audit ? h.audit.rows.map(r => [ { text: r.time }, { text: r.system }, { text: r["function"] }, { text: r.outcome, tone: r.tone } ]) : []
                    weights: [2, 1.5, 2, 3]
                }
            }
        }

        Panel {
            visible: !!h.report
            width: parent.width; title: h.report ? h.report.title : ""; note: h.report ? h.report.subtitle : ""
            Row {
                x: Theme.inset; topPadding: 10; bottomPadding: 10; spacing: 10
                Text { text: h.report ? h.report.verdict_text : ""; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13; weight: Font.DemiBold } }
                Text { text: h.report ? h.report.summary : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Button { visible: !!(h.report && h.report.path); text: "Open"; onClicked: AppModel.openPath(h.report.path) }
            }
        }

        Panel {
            visible: !!h.backup
            width: parent.width; title: h.backup ? h.backup.title : ""; note: h.backup ? h.backup.subtitle : ""
            Column {
                width: parent.width; spacing: 8; topPadding: 10; bottomPadding: 12
                Text { x: Theme.inset; width: parent.width - 2 * Theme.inset; wrapMode: Text.WordWrap; text: h.backup ? h.backup.note : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Row { x: Theme.inset; spacing: 8
                    Button { text: "Back up and export"; onClicked: AppModel.exportData() } }
                Repeater {
                    model: h.backup && h.backup.paths ? h.backup.paths : []
                    Row { required property var modelData; x: Theme.inset; spacing: 8
                        Text { text: modelData.label + ":"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                        Text { text: modelData.path; color: Theme.foreground; font { family: Theme.mono; pixelSize: 11 } }
                        Button { text: "Open"; onClicked: AppModel.openPath(modelData.path) } }
                }
                Text { visible: !!(h.backup && h.backup.stats); x: Theme.inset; text: h.backup && h.backup.stats ? h.backup.stats : ""; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
            }
        }
    }
}
