import QtQuick
import com.hondascan.app

// A honeycomb of hairlines, as on the Type R's dash trim: matte (no highlight, no gloss), drawn once per size, and faded
// toward the left so it sits behind the content instead of under it. `cell` is a hexagon's radius in pixels.
Canvas {
    id: root
    property real cell: 15
    property real inset: Theme.rCard      // keeps the lines off the pane's rounded corners
    renderStrategy: Canvas.Cooperative
    antialiasing: true
    onWidthChanged: requestPaint()
    onHeightChanged: requestPaint()
    onPaint: {
        const ctx = getContext("2d")
        ctx.reset()
        const g = ctx.createLinearGradient(0, 0, width, 0)
        g.addColorStop(0, "rgba(255,255,255,0)")
        g.addColorStop(0.45, "rgba(255,255,255,0.02)")
        g.addColorStop(1, "rgba(255,255,255,0.075)")
        ctx.strokeStyle = g
        ctx.lineWidth = 1
        const r = cell, w = Math.sqrt(3) * r, rowH = r * 1.5
        ctx.beginPath()
        for (let row = 0, y = inset; y < height - inset; row++, y += rowH) {
            for (let x = inset + (row % 2 ? w / 2 : 0); x < width - inset; x += w) {
                for (let k = 0; k < 6; k++) {
                    const a = Math.PI / 6 + k * Math.PI / 3
                    const px = x + r * Math.cos(a), py = y + r * Math.sin(a)
                    if (k === 0) ctx.moveTo(px, py); else ctx.lineTo(px, py)
                }
                ctx.closePath()
            }
        }
        ctx.stroke()
    }
}
