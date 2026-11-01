import QtQuick
import com.hondascan.app

// A small label in capitals over a group of things (a pane's title, a list's heading): one look everywhere.
Text {
    color: Theme.mutedFg
    font { family: Theme.sans; pixelSize: 11; weight: Font.DemiBold; letterSpacing: 1.1; capitalization: Font.AllUppercase }
}
