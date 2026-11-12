import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

Window {
    id: win
    width: 1280
    height: 820
    minimumWidth: 960
    minimumHeight: 600
    visible: true
    color: "transparent"
    flags: Qt.Window | Qt.FramelessWindowHint
    title: "Hondascan"
    readonly property real corner: visibility === Window.Maximized || visibility === Window.FullScreen ? 0 : 8

    property bool sidebarShown: true
    property string view: "overview"   // "overview", "history" or a system key
    onViewChanged: {
        if (view === "history") AppModel.historyOpen()
        swap.restart()
    }
    // the app's own fonts (build.rs embeds them), so it looks the same on a machine that has none of them
    FontLoader { source: "qrc:/assets/fonts/NotoSans-Regular.ttf" }
    FontLoader { source: "qrc:/assets/fonts/NotoSans-Bold.ttf" }
    FontLoader { source: "qrc:/assets/fonts/NotoSansMono-Regular.ttf" }
    FontLoader { source: "qrc:/assets/fonts/NotoSansMono-Medium.ttf" }
    FontLoader { source: "qrc:/assets/fonts/BarlowSemiCondensed-SemiBold.ttf" }

    Component.onCompleted: { Theme.backdrop = app; AppModel.start() }
    // The startup animation plays once a day, over the app while it starts. Until the settings say whether it plays, the app is
    // behind black (it is black anyway), so a start with no animation shows no flash of the screens; the backend not answering
    // lifts that after a moment.
    property bool waitingForPlan: true
    Timer { interval: 1500; running: win.waitingForPlan; onTriggered: win.waitingForPlan = false }
    Timer { id: keyOn; interval: 2500; onTriggered: Theme.ignition = false }
    Connections {
        target: AppModel
        function onIntroPlanChanged() {
            win.waitingForPlan = false
            if (AppModel.introPlan === 1) { AppModel.introStarted(); intro.start() }
        }
        function onLinkChanged() {
            if (AppModel.link !== 2 && win.view !== "history") win.view = "overview"
            if (AppModel.link === 2) { Theme.ignition = true; keyOn.restart() }
        }
        function onKlineChanged() {   // HONDASCAN_OPEN=METER on a K-line adapter: open that system once the search has found it
            const k = JSON.parse(AppModel.kline), f = (k.found || []).find(x => x.system === AppModel.openHint())
            if (f && k.open === null && k.busy === "") AppModel.klineOpen(f.address, f.system)
        }
        function onEcusChanged() {
            const [hint, tab] = AppModel.openHint().split("/")   // "PGM-FI" or "PGM-FI/codes": a system, and which tab
            const e = hint === "" ? undefined : JSON.parse(AppModel.ecus).find(x => x.system_type === hint)
            if (hint === "history") win.view = "history"
            else if (hint === "read-codes" && !readDialog.opened) readDialog.open()
            else if (e && win.view !== e.key) { win.view = e.key; if (tab) system.tab = tab }
        }
    }

    Item {   // everything but the dialogs and notices: what a dialog blurs behind it (Sheet)
        id: app
        anchors.fill: parent
        Rectangle { anchors.fill: parent; color: Theme.background; radius: win.corner }
        TopBar {
            id: top; width: parent.width; height: 48; corner: win.corner
            onCloseRequested: win.close()
            onMinimizeRequested: win.showMinimized()
            onZoomRequested: win.visibility === Window.Maximized ? win.showNormal() : win.showMaximized()
            sidebarShown: win.sidebarShown
            onSidebarToggled: win.sidebarShown = !win.sidebarShown
        }
        Shortcut { sequence: "Ctrl+B"; onActivated: win.sidebarShown = !win.sidebarShown }
        Shortcut { sequence: "Ctrl+R"; enabled: AppModel.link === 2 && !AppModel.scanning; onActivated: AppModel.rescan() }
        AlertBar { id: alerts; y: top.height; width: parent.width }
        RunningBars { id: bars; y: top.height + alerts.height; width: parent.width }
        Sidebar {
            id: side
            y: top.height + alerts.height + bars.height
            width: win.sidebarShown ? 240 : 0; height: parent.height - y
            visible: width > 0
            clip: true
            Behavior on width { NumberAnimation { duration: Theme.normal; easing.type: Easing.OutCubic } }
            selected: win.view
            corner: win.corner
            onPick: key => win.view = key
        }

        Flickable {
            id: page
            x: side.width; y: side.y
            width: parent.width - side.width; height: parent.height - y
            contentWidth: width
            contentHeight: content.height + 32
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            interactive: AppModel.shotPath() === ""   // a screenshot run ignores stray input

            // a new screen settles in: a short fade and a rise of a few points
            ParallelAnimation {
                id: swap
                NumberAnimation { target: content; property: "opacity"; from: 0; to: 1; duration: Theme.normal; easing.type: Easing.OutCubic }
                NumberAnimation { target: content; property: "y"; from: 28; to: 16; duration: Theme.normal; easing.type: Easing.OutCubic }
            }
            Item {
                id: content
                x: 16; y: 16
                width: page.width - 32
                height: win.view === "overview" ? overview.implicitHeight : win.view === "history" ? history.implicitHeight : system.implicitHeight

                Overview {
                    id: overview
                    visible: win.view === "overview"
                    width: parent.width
                    onOpen: key => win.view = key
                    onReadRequested: readDialog.open()
                }
                HistoryView { id: history; visible: win.view === "history"; width: parent.width }
                SystemView {
                    id: system
                    visible: win.view !== "overview" && win.view !== "history"
                    width: parent.width
                    key: win.view
                }
            }
        }
    }

    ReadCodesDialog { id: readDialog }
    ShareDialog { id: shareDialog }
    // the first time the app is opened: the sharing question, once the startup animation (if any) is out of the way
    property bool introPlayed: false
    Timer { interval: 1500; running: AppModel.shareAsk && (AppModel.introPlan === 2 || win.introPlayed) && !shareDialog.opened; onTriggered: shareDialog.open() }
    // HONDASCAN_SHOT=file.png: save a picture of the window after a few seconds of live data and quit
    Timer {
        interval: 7000
        running: AppModel.shotPath() !== ""
        onTriggered: {
            page.contentY = Math.min(AppModel.scrollHint(), Math.max(0, page.contentHeight - page.height))
            win.contentItem.grabToImage(r => { r.saveToFile(AppModel.shotPath()); Qt.quit() })
        }
    }
    Toasts { anchors { right: parent.right; bottom: parent.bottom; margins: 16 } }
    Rectangle { anchors.fill: parent; radius: win.corner; color: Theme.background; visible: win.waitingForPlan }
    Intro { id: intro; anchors.fill: parent; corner: win.corner; sequence: AppModel.introSequence; onDissolving: swap.restart(); onFinished: win.introPlayed = true; onFailed: reason => { win.introPlayed = true; AppModel.introFailed(reason) } }

    // the window's edges and corners resize it (the frame has no border of its own)
    Repeater {
        model: [
            { e: Qt.LeftEdge, x: 0, y: 12, w: 5, h: -24, c: Qt.SizeHorCursor }, { e: Qt.RightEdge, x: -5, y: 12, w: 5, h: -24, c: Qt.SizeHorCursor },
            { e: Qt.TopEdge, x: 12, y: 0, w: -24, h: 5, c: Qt.SizeVerCursor }, { e: Qt.BottomEdge, x: 12, y: -5, w: -24, h: 5, c: Qt.SizeVerCursor },
            { e: Qt.LeftEdge | Qt.TopEdge, x: 0, y: 0, w: 12, h: 12, c: Qt.SizeFDiagCursor }, { e: Qt.RightEdge | Qt.TopEdge, x: -12, y: 0, w: 12, h: 12, c: Qt.SizeBDiagCursor },
            { e: Qt.LeftEdge | Qt.BottomEdge, x: 0, y: -12, w: 12, h: 12, c: Qt.SizeBDiagCursor }, { e: Qt.RightEdge | Qt.BottomEdge, x: -12, y: -12, w: 12, h: 12, c: Qt.SizeFDiagCursor }
        ]
        Item {
            required property var modelData
            enabled: win.visibility !== Window.Maximized && win.visibility !== Window.FullScreen
            x: modelData.x < 0 ? win.width + modelData.x : modelData.x
            y: modelData.y < 0 ? win.height + modelData.y : modelData.y
            width: modelData.w < 0 ? win.width + modelData.w - modelData.x : modelData.w
            height: modelData.h < 0 ? win.height + modelData.h - modelData.y : modelData.h
            HoverHandler { cursorShape: modelData.c }
            DragHandler { target: null; onActiveChanged: if (active) win.startSystemResize(modelData.e) }
        }
    }
}
