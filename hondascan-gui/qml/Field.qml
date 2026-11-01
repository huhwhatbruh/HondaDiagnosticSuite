import QtQuick
import QtQuick.Controls.Basic as C
import com.hondascan.app

// A line of text to type: the app's face, the keyboard focus ring when it has the focus, the selection in the app's red.
C.TextField {
    id: root
    height: 32
    leftPadding: 10
    color: Theme.foreground
    placeholderTextColor: Theme.mutedFg
    selectionColor: Theme.red
    selectedTextColor: Theme.redText
    font { family: Theme.sans; pixelSize: 13 }
    background: Rectangle {
        radius: Theme.rControl
        color: Theme.secondary
        border.color: root.activeFocus ? Theme.focus : Theme.outline
        border.width: root.activeFocus ? 2 : 1
    }
}
