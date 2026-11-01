import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Effects
import com.hondascan.app

// The base of every dialog: modal, centred, the app behind it blurred and dimmed (fading in), the sheet itself settling in from a
// touch smaller. The blur is the GPU's (MultiEffect over a live picture of the app, Theme.backdrop), so the car's numbers keep
// moving, softly, behind the question; the sheet itself stays solid and matte (materials.md: material on the layer above the
// content, never in it). A dialog built on it only sets its width, padding and contents. Costs nothing while no dialog is open: the modal layer exists only then.
Popup {
    id: root
    modal: true
    anchors.centerIn: Overlay.overlay
    transformOrigin: Item.Center
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    background: Rectangle { radius: Theme.rPopup; color: Theme.popover; border.color: Theme.outline; border.width: 1 }

    Overlay.modal: Item {
        Behavior on opacity { NumberAnimation { duration: Theme.normal } }
        ShaderEffectSource {
            id: picture
            anchors.fill: parent
            sourceItem: Theme.backdrop
            visible: false
        }
        MultiEffect {
            anchors.fill: parent
            source: picture
            autoPaddingEnabled: false
            blurEnabled: true
            blur: 1
            blurMax: 40
            saturation: -0.3
        }
        Rectangle { anchors.fill: parent; color: Theme.scrim }
    }
    enter: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.normal; easing.type: Easing.OutCubic }
            NumberAnimation { property: "scale"; from: 0.96; to: 1; duration: Theme.normal; easing.type: Easing.OutCubic }
        }
    }
    exit: Transition {
        ParallelAnimation {
            NumberAnimation { property: "opacity"; to: 0; duration: Theme.fast }
            NumberAnimation { property: "scale"; to: 0.98; duration: Theme.fast }
        }
    }
}
