import QtQuick
import com.hondascan.app

// The car at a glance: how the engine is doing, the health of every system, and every trouble code.
Item {
    id: root
    signal open(string key)
    signal readRequested
    readonly property bool onKline: AppModel.link === 2 && JSON.parse(AppModel.kline).mode !== undefined && JSON.parse(AppModel.kline).mode !== ""
    implicitHeight: onKline ? kline.implicitHeight : AppModel.link === 2 ? col.implicitHeight : welcome.implicitHeight

    Welcome { id: welcome; visible: AppModel.link !== 2; width: parent.width }

    KlineView { id: kline; visible: root.onKline; width: parent.width }

    Column {
        id: col
        visible: AppModel.link === 2 && !root.onKline
        width: parent.width
        spacing: 12
        readonly property var glanceLive: JSON.parse(AppModel.glancesLive)

        Hero { width: parent.width }
        Row {   // the two panes end on one line
            id: pair
            width: parent.width; spacing: 12
            readonly property real tall: Math.max(car.naturalHeight, codes.implicitHeight)
            CarCard { id: car; width: parent.width - 372; height: pair.tall; onOpen: key => root.open(key) }
            CodesList { id: codes; width: 360; height: pair.tall; onOpen: key => root.open(key); onReadRequested: root.readRequested() }
        }
        Health { width: parent.width; onOpen: key => root.open(key) }
        DriveCard { width: parent.width }
        MapsCard { width: parent.width }
        Repeater {
            model: JSON.parse(AppModel.glances)   // the layout: the same between updates, so the cards stay
            GlanceCard {
                required property int index
                required property var modelData
                width: col.width
                card: modelData
                live: col.glanceLive[index] || ({})
            }
        }
    }
}
