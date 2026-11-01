import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// The header: which adapter, connect / disconnect, and the car's VIN and link state.
Rectangle {
    id: root
    color: Theme.panel
    implicitHeight: 48
    // the window is drawn by the app (rounded, with its own lights at the right end): empty space drags it, double click zooms it
    property real corner: 8
    topLeftRadius: corner; topRightRadius: corner
    signal closeRequested
    signal minimizeRequested
    signal zoomRequested
    signal sidebarToggled
    property bool sidebarShown: true

    // empty space drags the window and double click zooms it: only the space between the controls, so a press on a control never moves it
    Item {
        anchors { left: controls.right; right: status.left; top: parent.top; bottom: parent.bottom }
        DragHandler { target: null; onActiveChanged: if (active) root.Window.window.startSystemMove() }
        TapHandler { acceptedButtons: Qt.LeftButton; gesturePolicy: TapHandler.ReleaseWithinBounds; onDoubleTapped: root.zoomRequested() }
    }

    readonly property var list: JSON.parse(AppModel.adapters)
    property int choice: 0
    property string custom: "j2534:/usr/lib/libj2534.so"
    readonly property string spec: list.length === 0 ? "" : (list[Math.min(choice, list.length - 1)].spec === "custom" ? custom.trim() : list[Math.min(choice, list.length - 1)].spec)
    readonly property bool connected: AppModel.link === 2

    // select the adapter used last time, once the list and the settings are both here
    function pickSaved() {
        const last = AppModel.lastAdapter
        for (let i = 0; i < list.length; ++i) if (list[i].spec === last) { choice = i; return }
        if (last !== "") { choice = list.length - 1; custom = last }
    }
    onListChanged: pickSaved()
    Connections {
        target: AppModel
        function onLastAdapterChanged() { root.pickSaved() }
        // connected by the backend (HONDASCAN_ADAPTER) or the welcome button: show the adapter that is actually in use
        function onAdapterChanged() {
            const i = root.list.findIndex(a => a.spec === AppModel.adapter)
            if (i >= 0) root.choice = i
            else { root.choice = root.list.length - 1; root.custom = AppModel.adapter }
        }
    }

    Row {
        id: controls
        anchors { left: parent.left; leftMargin: 14; verticalCenter: parent.verticalCenter }
        spacing: 10

        Tappable {   // show / hide the sidebar (Ctrl+B): the HIG asks that people can hide it
            width: 28; height: 28; radius: Theme.rControl
            text: root.sidebarShown ? "Hide sidebar" : "Show sidebar"
            hint: text + "  Ctrl+B"
            onClicked: root.sidebarToggled()
            anchors.verticalCenter: parent.verticalCenter
            Rectangle {   // a window with its sidebar: an outline and a rule a third of the way across
                anchors.centerIn: parent
                width: 16; height: 12; radius: 3
                color: "transparent"
                border { width: 1.5; color: parent.hovered ? Theme.foreground : Theme.mutedFg }
                Rectangle { x: 5; y: 0; width: 1.5; height: parent.height; color: parent.border.color }
            }
        }

        ComboBox {
            id: combo
            width: 330
            height: 32
            enabled: AppModel.link === 0
            model: root.list
            textRole: "label"
            currentIndex: root.choice
            onActivated: i => root.choice = i
            hoverEnabled: true
            background: ControlFace { enabled: combo.enabled; hovered: combo.hovered; down: combo.pressed || combo.popup.visible; focusRing: combo.visualFocus }
            contentItem: Text {
                leftPadding: 12
                text: combo.displayText
                color: Theme.foreground
                font { family: Theme.sans; pixelSize: 13 }
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
            }
            indicator: Glyph { x: combo.width - width - 12; anchors.verticalCenter: parent.verticalCenter; kind: "down"; color: Theme.mutedFg }
            delegate: ItemDelegate {
                width: combo.width
                height: 30
                highlighted: combo.highlightedIndex === index
                contentItem: Text { text: modelData.label; color: Theme.foreground; font { family: Theme.sans; pixelSize: 13 } verticalAlignment: Text.AlignVCenter; leftPadding: 4 }
                background: Rectangle { color: parent.highlighted ? Theme.hover : Theme.popover }
            }
            popup: Popup {
                y: combo.height
                width: combo.width
                implicitHeight: Math.min(contentItem.implicitHeight, 360)
                padding: 0
                contentItem: ListView { clip: true; implicitHeight: contentHeight; model: combo.popup.visible ? combo.delegateModel : null; currentIndex: combo.highlightedIndex }
                background: Rectangle { radius: Theme.rControl; color: Theme.popover; border.color: Theme.outline; border.width: 1 }
            }
        }

        Field {   // only while there is no connection: the field is the adapter to open, and the room is needed for the buttons
            visible: AppModel.link === 0 && root.list.length > 0 && root.list[Math.min(root.choice, root.list.length - 1)].spec === "custom"
            width: 260
            text: root.custom
            onTextEdited: root.custom = text
            font { family: Theme.mono; pixelSize: 12 }
        }

        Button {
            text: AppModel.link === 1 ? "Connecting…" : root.connected ? "Disconnect" : "Connect"
            primary: !root.connected
            enabled: AppModel.link !== 1 && (root.connected || root.spec !== "")
            onClicked: root.connected ? AppModel.disconnect() : AppModel.connectAdapter(root.spec)
        }
        Button {
            visible: root.connected
            text: AppModel.scanning ? "Scanning…" : "Scan again"
            enabled: !AppModel.scanning
            onClicked: AppModel.rescan()
        }
    }

    Row {
        id: status
        anchors { right: lights.left; rightMargin: 16; verticalCenter: parent.verticalCenter }
        spacing: 14
        Rectangle {   // a simulated car is on the other end: nothing shown or recorded is the person's car
            visible: AppModel.link > 0 && (AppModel.adapter === "sim" || AppModel.adapter.indexOf("sim:") === 0)
            anchors.verticalCenter: parent.verticalCenter
            width: simLabel.implicitWidth + 20; height: 22; radius: Theme.rControl
            color: Theme.warn
            Accessible.role: Accessible.StaticText
            Accessible.name: "Simulated car: the readings are made up"
            Text { id: simLabel; anchors.centerIn: parent; text: "SIMULATED CAR"; color: "#000000"; font { family: Theme.sans; pixelSize: 11; bold: true; letterSpacing: 0.6 } }
        }
        Text {
            visible: AppModel.vin !== ""
            anchors.verticalCenter: parent.verticalCenter
            text: AppModel.vin
            color: Theme.mutedFg
            font { family: Theme.mono; pixelSize: 12 }
        }
        Row {
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6
            Rectangle {
                width: 8; height: 8; radius: 4
                anchors.verticalCenter: parent.verticalCenter
                color: root.connected ? Theme.ok : AppModel.link === 1 ? Theme.warn : Theme.mutedFg
            }
            Text {
                text: root.connected ? "Connected" : AppModel.link === 1 ? "Connecting" : "Not connected"
                color: Theme.mutedFg
                font { family: Theme.sans; pixelSize: 12 }
            }
        }
    }
    // close, minimise and zoom as a Mac window has them: three round lights at the right end, their glyphs shown while the pointer is over the group
    Row {
        id: lights
        anchors { right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter }
        spacing: 0
        HoverHandler { id: group }
        Repeater {
            model: [ { color: Theme.lightZoom, glyph: "+", name: "Zoom window", sig: 2 }, { color: Theme.lightMinimise, glyph: "−", name: "Minimise window", sig: 1 }, { color: Theme.lightClose, glyph: "×", name: "Close window", sig: 0 } ]
            Tappable {   // the light is 13 points; its hit area is 20 x 20, the HIG's smallest macOS control
                required property var modelData
                width: 20; height: 20; radius: 10
                text: modelData.name
                hint: modelData.name
                onClicked: modelData.sig === 0 ? root.closeRequested() : modelData.sig === 1 ? root.minimizeRequested() : root.zoomRequested()
                Rectangle {
                    anchors.centerIn: parent
                    width: 13; height: 13; radius: 6.5
                    color: root.Window.window && !root.Window.window.active && !group.hovered ? Theme.secondary : modelData.color
                    Text { anchors.centerIn: parent; visible: group.hovered; text: modelData.glyph; color: Theme.lightGlyph; font { family: Theme.sans; pixelSize: 11; weight: Font.Bold } }
                }
            }
        }
    }
    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.border }
}
