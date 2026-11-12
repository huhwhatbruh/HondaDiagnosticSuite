import QtQuick
import QtQuick.Controls.Basic
import com.hondascan.app

// Asked once, the first time the app is opened: whether to send the project a summary of the car and adapter, so it can see
// which cars work. Nothing goes until Share is pressed; closing the card without answering counts as No thanks, so it is never
// asked twice. The same choice stays in the sidebar.
Sheet {
    id: root
    width: 480
    padding: 20
    property bool answered: false
    onAboutToShow: { answered = false; shown = false; AppModel.sharePreview() }
    onClosed: if (!answered) AppModel.shareAnswer(false)
    property bool shown: false

    contentItem: Column {
        spacing: 10
        Text { width: parent.width; text: "Help hondascan work on more cars?"; wrapMode: Text.WordWrap; color: Theme.foreground; font { family: Theme.sans; pixelSize: 16; weight: Font.DemiBold } }
        Text {
            width: parent.width; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 13 }
            text: "When something goes wrong, or a car works with hondascan for the first time, it can send a short report. That is how problems on other cars and computers get found and fixed."
        }
        Text { width: parent.width; text: "What is sent"; color: Theme.foreground; font { family: Theme.sans; pixelSize: 12; weight: Font.DemiBold } }
        Repeater {
            model: [
                "Errors the app ran into (without your folders or port names)",
                "The car’s make, model, year and factory (not the full VIN)",
                "Which control units answered, and their part numbers",
                "Trouble codes, and how the scan went",
                "App version and operating system"
            ]
            Row { required property string modelData; spacing: 8
                Text { text: "•"; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } }
                Text { width: root.width - 56; text: parent.modelData; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 } } }
        }
        Text {
            width: parent.width; wrapMode: Text.WordWrap; color: Theme.mutedFg; font { family: Theme.sans; pixelSize: 12 }
            text: "Your VIN, name, address and driving data stay on this computer. A report goes out only when there is something new, at most five a day, and you can turn it off any time in the sidebar."
        }
        Text {
            visible: AppModel.shareText !== ""
            width: parent.width
            text: root.shown ? "Hide the exact report" : "See the exact report"
            color: Theme.accent; font { family: Theme.sans; pixelSize: 12; underline: true }
            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: root.shown = !root.shown }
        }
        Flickable {
            visible: root.shown && AppModel.shareText !== ""
            width: parent.width; height: 160; clip: true
            contentWidth: width; contentHeight: report.implicitHeight
            Text { id: report; width: parent.width; text: AppModel.shareText; wrapMode: Text.WrapAnywhere; color: Theme.foreground; font { family: Theme.mono; pixelSize: 10 } }
            ScrollBar.vertical: ScrollBar {}
        }
        Row {
            anchors.right: parent.right
            spacing: 8
            Button { text: "Share"; primary: true; focus: true; onClicked: { root.answered = true; AppModel.shareAnswer(true); root.close() } }
            Button { text: "No thanks"; onClicked: { root.answered = true; AppModel.shareAnswer(false); root.close() } }
        }
    }
}
