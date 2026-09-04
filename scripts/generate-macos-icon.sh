#!/usr/bin/env bash
# Usage: scripts/generate-macos-icon.sh
#
# Rebuilds assets/icons/macos/ from assets/icons/serialx-icon.svg.
#
# The master artwork fills its canvas — a rounded plate inset 32 of 1024 — which
# is what the Windows executable icon and the Linux hicolor PNGs want. macOS
# instead lays every app icon out on a shared grid: an 824x824 plate inset 100
# on a 1024 canvas. An icon drawn edge to edge therefore renders noticeably
# larger than its Dock neighbours, so the macOS assets are rescaled onto that
# grid rather than copied from the master as-is.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

master="assets/icons/serialx-icon.svg"
iconset="assets/icons/macos/SerialX.iconset"
icns="assets/icons/macos/SerialX.icns"

swift - "$master" "$iconset" <<'SWIFT'
import AppKit
import Foundation

let canvas = 1024.0
let masterInset = 32.0 // Rounded plate inset in the master artwork.
let macOSInset = 100.0 // Rounded plate inset on the macOS icon grid.

let svgPath = CommandLine.arguments[1]
let outputDirectory = CommandLine.arguments[2]

guard let artwork = NSImage(contentsOfFile: svgPath) else {
    FileHandle.standardError.write(Data("unable to read \(svgPath)\n".utf8))
    exit(1)
}
var proposed = CGRect(x: 0, y: 0, width: canvas, height: canvas)
guard let master = artwork.cgImage(forProposedRect: &proposed, context: nil, hints: nil) else {
    FileHandle.standardError.write(Data("unable to rasterise \(svgPath)\n".utf8))
    exit(1)
}

// Map the master plate onto the macOS plate, then read off where the full
// canvas lands so the artwork keeps its proportions.
let scale = (canvas - 2 * macOSInset) / (canvas - 2 * masterInset)
let offset = macOSInset - masterInset * scale

let variants: [(name: String, side: Int)] = [
    ("icon_16x16.png", 16), ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32), ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128), ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256), ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512), ("icon_512x512@2x.png", 1024),
]

for variant in variants {
    let ratio = Double(variant.side) / canvas
    guard let context = CGContext(
        data: nil, width: variant.side, height: variant.side, bitsPerComponent: 8,
        bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { exit(1) }
    context.interpolationQuality = .high
    context.draw(master, in: CGRect(
        x: offset * ratio, y: offset * ratio,
        width: canvas * scale * ratio, height: canvas * scale * ratio
    ))

    let url = URL(fileURLWithPath: outputDirectory).appendingPathComponent(variant.name)
    guard let image = context.makeImage(),
          let destination = CGImageDestinationCreateWithURL(url as CFURL, "public.png" as CFString, 1, nil)
    else { exit(1) }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else {
        FileHandle.standardError.write(Data("unable to write \(variant.name)\n".utf8))
        exit(1)
    }
    print("\(variant.name) (\(variant.side)px)")
}
SWIFT

iconutil --convert icns "$iconset" --output "$icns"
echo "${icns} rebuilt from ${master}"
