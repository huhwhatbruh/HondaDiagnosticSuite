pragma Singleton
import QtQuick

// The look (owner, 2026-10-24: black, not grey): solid black, matte panes a shade above it with a quiet outline, square inside,
// one red accent (Honda racing red, as on the Type R: the selection, the primary button, the dial's indicator). Greys are neutral.
QtObject {
    readonly property color background: "#000000"
    readonly property color foreground: "#ebebeb"
    readonly property color panel: "#0c0c0c"      // the top bar and the sidebar
    readonly property color card: "#101010"       // panes and cards, a step above the bars
    readonly property color popover: "#1a1a1a"    // menus, popups, toasts
    readonly property color secondary: "#202020"  // a control's face, an idle title bar
    readonly property color hover: "#2e2e2e"
    readonly property color muted: "#161616"
    readonly property color mutedFg: "#969696"
    readonly property color accent: "#2a0d11"    // the selected row: black with a trace of the red
    readonly property color red: "#c8102e"        // the brand accent
    readonly property color redHot: "#e4283f"
    readonly property color blue: "#2e62aa"       // only the maps card's "differs from history" outline
    readonly property color outline: "#2e2e2e"    // the edge of a control
    readonly property color edge: "#1c1c1c"       // the quiet edge of a pane: visible, not drawn on
    readonly property color border: "#242424"     // a separator inside a pane
    readonly property color ok: "#4bc680"
    readonly property color warn: "#edb345"
    readonly property color fault: "#f54748"

    readonly property color focus: "#e8e8e8"      // the keyboard focus ring
    readonly property color redText: "#ffffff"      // text and marks on the red
    readonly property color scrim: "#8c000000"    // what dims the app behind a dialog

    // A control's face (ControlFace): its top and foot, resting, under the pointer and pressed, and its rim; the red one likewise.
    readonly property color faceTop: "#383838"
    readonly property color faceTopHot: "#444444"
    readonly property color faceFoot: "#262626"
    readonly property color faceFootHot: "#323232"
    readonly property color facePressed: "#1c1c1c"
    readonly property color faceRim: "#4a4a4a"
    readonly property color faceRimHot: "#5e5e5e"
    readonly property color redTop: "#d01632"
    readonly property color redFoot: "#b00d27"
    readonly property color redFootHot: red
    readonly property color redPressed: "#a30c24"
    readonly property color redRim: "#e0233c"
    readonly property color redRimHot: "#f04a5e"
    readonly property color lightZoom: "#28c840"   // the window's three lights
    readonly property color lightMinimise: "#febc2e"
    readonly property color lightClose: "#ff5f57"
    readonly property color lightGlyph: "#4d0000"
    readonly property color tick: "#4a4a4a"       // the dial's minor ticks
    readonly property color disc: "#212121"       // a raised disc on the car (top, under the pointer, foot)
    readonly property color discHot: "#2a2a2a"

    // the one inset of everything inside a pane, in line with its title
    readonly property int inset: 18

    // Depth: a pane is lit from above, a shade lighter at its top edge than at its foot (averaging `card`), so it sits on the
    // black instead of being cut out of it. Matte: no highlight line, no sheen. One instance, shared by every pane.
    readonly property Gradient cardFill: Gradient {
        GradientStop { position: 0; color: "#141414" }
        GradientStop { position: 1; color: "#0d0d0d" }
    }

    // Shape and motion: corners only just softened (owner, 2026-10-30: "wayyy too rounded"), short eased transitions that
    // answer the pointer or show something entering, never decoration
    readonly property int rCard: 6
    readonly property int rControl: 4
    readonly property int rRow: 3
    readonly property int rPopup: 6
    readonly property int fast: 120
    readonly property int normal: 220

    // Set by Main: the app's content (what a dialog blurs behind it), and the moment the car has just come on (the dials sweep
    // once, as an instrument cluster does at key-on; only in this window, so revisiting a screen does not sweep again).
    property Item backdrop: null
    property bool ignition: false

    readonly property string sans: "Noto Sans"
    readonly property string display: "Barlow Semi Condensed"   // the dials' big figures, like an instrument cluster's numerals
    readonly property string mono: "Noto Sans Mono"

    // a figure's tone from the backend: 0 normal, 1 warning, 2 fault
    function tone(t) { return t === 2 ? fault : t === 1 ? warn : foreground }
}
