#!/usr/bin/env swift
//
// Render the placeholder app icon: a dark glass diamond in a rounded square.
// Produces a 1024x1024 PNG; build-iconset.sh turns it into Dragon Glass.icns.
//
// usage: swift make-icon.swift <out.png>

import AppKit
import Foundation

let side = 1024.0
let output = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon-1024.png"

let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: Int(side),
    pixelsHigh: Int(side),
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
)!

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

let rect = NSRect(x: 0, y: 0, width: side, height: side)
let inset = 32.0
let tile = rect.insetBy(dx: inset, dy: inset)
let tilePath = NSBezierPath(roundedRect: tile, xRadius: 210, yRadius: 210)

// Background: deep blue-black, lit from the top left.
let background = NSGradient(
    starting: NSColor(red: 0.10, green: 0.15, blue: 0.22, alpha: 1),
    ending: NSColor(red: 0.02, green: 0.03, blue: 0.05, alpha: 1)
)!
background.draw(in: tilePath, angle: -60)

// Soft inner border.
NSColor(white: 1, alpha: 0.10).setStroke()
tilePath.lineWidth = 6
tilePath.stroke()

// The glass: a faceted diamond.
let center = NSPoint(x: side / 2, y: side / 2)
let top = NSPoint(x: center.x, y: center.y + 330)
let right = NSPoint(x: center.x + 300, y: center.y)
let bottom = NSPoint(x: center.x, y: center.y - 330)
let left = NSPoint(x: center.x - 300, y: center.y)

func facet(_ points: [NSPoint]) -> NSBezierPath {
    let path = NSBezierPath()
    path.move(to: points[0])
    for point in points.dropFirst() {
        path.line(to: point)
    }
    path.close()
    return path
}

let light = NSColor(red: 0.45, green: 0.90, blue: 1.0, alpha: 1)
let deep = NSColor(red: 0.42, green: 0.30, blue: 0.95, alpha: 1)

let facets: [([NSPoint], NSColor)] = [
    ([top, right, center], light.withAlphaComponent(0.55)),
    ([top, left, center], light.withAlphaComponent(0.25)),
    ([bottom, right, center], deep.withAlphaComponent(0.45)),
    ([bottom, left, center], deep.withAlphaComponent(0.70)),
]
for (points, color) in facets {
    color.setFill()
    facet(points).fill()
}

let outline = NSBezierPath()
outline.move(to: top)
outline.line(to: right)
outline.line(to: bottom)
outline.line(to: left)
outline.close()
NSColor(white: 1, alpha: 0.85).setStroke()
outline.lineWidth = 14
outline.stroke()

NSGraphicsContext.restoreGraphicsState()

let png = rep.representation(using: .png, properties: [:])!
try png.write(to: URL(fileURLWithPath: output))
print("wrote \(output)")
