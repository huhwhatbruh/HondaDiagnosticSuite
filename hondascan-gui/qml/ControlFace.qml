import QtQuick
import com.hondascan.app

// The face of something to press: lit from above, an edge brighter than a pane's, and a dark lip under it, so it stands up off
// the pane. Pressed, it goes down onto the lip and darkens. Matte (no highlight line, no sheen), as the panes are. `primary` is the red one.
Item {
    id: root
    property bool hovered: false
    property bool down: false
    property bool primary: false
    property bool focusRing: false

    Rectangle {   // the lip: the face's foot in shadow
        anchors { fill: parent; topMargin: 1; bottomMargin: -1 }
        radius: Theme.rControl
        color: Theme.background
        visible: !root.down && root.enabled
    }
    Rectangle {
        id: face
        width: parent.width; height: parent.height
        y: root.down ? 1 : 0
        radius: Theme.rControl
        border.width: 1
        border.color: !root.enabled ? Theme.outline
                    : root.primary ? (root.hovered ? Theme.redRimHot : Theme.redRim)
                    : root.hovered ? Theme.faceRimHot : Theme.faceRim
        gradient: Gradient {
            GradientStop {
                position: 0
                color: !root.enabled ? Theme.secondary
                     : root.primary ? (root.down ? Theme.redPressed : root.hovered ? Theme.redHot : Theme.redTop)
                     : root.down ? Theme.facePressed : root.hovered ? Theme.faceTopHot : Theme.faceTop
                Behavior on color { ColorAnimation { duration: Theme.fast } }
            }
            GradientStop {
                position: 1
                color: !root.enabled ? Theme.secondary
                     : root.primary ? (root.down ? Theme.redPressed : root.hovered ? Theme.redFootHot : Theme.redFoot)
                     : root.down ? Theme.facePressed : root.hovered ? Theme.faceFootHot : Theme.faceFoot
                Behavior on color { ColorAnimation { duration: Theme.fast } }
            }
        }
    }
    Rectangle {   // the focus ring, for the keyboard only
        anchors { fill: parent; margins: -3 }
        radius: Theme.rControl + 2
        color: "transparent"
        border { color: Theme.focus; width: 2 }
        visible: root.focusRing
    }
}
