import QtQuick
import QtQuick.Controls.Basic as C
import com.hondascan.app

// Several lines of text to type: Field's look.
C.TextArea {
    id: root
    wrapMode: TextEdit.Wrap
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
