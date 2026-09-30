#!/usr/bin/env swift
// Renders the FluxTV app icons for iOS and tvOS from the logo geometry in
// apps/windows/assets/icon.svg (1024-unit canvas).
//
//   swift apps/apple/scripts/make-icons.swift apps/apple/FluxTV/Resources/Assets.xcassets
import CoreGraphics
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers

let catalog = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "Assets.xcassets")
let space = CGColorSpace(name: CGColorSpace.sRGB)!

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
        blue: CGFloat(hex & 0xff) / 255, alpha: alpha)
}

func render(_ width: Int, _ height: Int, opaque: Bool, _ draw: (CGContext) -> Void) -> CGImage {
    let info = opaque ? CGImageAlphaInfo.noneSkipLast.rawValue : CGImageAlphaInfo.premultipliedLast.rawValue
    let ctx = CGContext(
        data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: info)!
    // SVG-style coordinates: origin top-left.
    ctx.translateBy(x: 0, y: CGFloat(height))
    ctx.scaleBy(x: 1, y: -1)
    ctx.setShouldAntialias(true)
    ctx.interpolationQuality = .high
    draw(ctx)
    return ctx.makeImage()!
}

func write(_ image: CGImage, _ path: String) {
    let url = catalog.appendingPathComponent(path)
    try! FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    let dest = CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(dest, image, nil)
    precondition(CGImageDestinationFinalize(dest), "could not write \(path)")
}

func writeJSON(_ object: Any, _ path: String) {
    let url = catalog.appendingPathComponent(path)
    try! FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    let data = try! JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
    try! data.write(to: url)
}

let info: [String: Any] = ["author": "xcode", "version": 1]

func background(_ ctx: CGContext, _ w: CGFloat, _ h: CGFloat) {
    let gradient = CGGradient(colorsSpace: space, colors: [color(0x6c8cff), color(0x9b6cff)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(gradient, start: .zero, end: CGPoint(x: w, y: h), options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
}

/// The TV-with-antenna glyph, centred on `center`, `size` = height of the glyph in pixels.
func glyph(_ ctx: CGContext, center: CGPoint, size: CGFloat, shadow: Bool = false) {
    // Glyph bounds in SVG units (including stroke): x 204…820, y 178…758.
    let s = size / 580
    ctx.saveGState()
    ctx.translateBy(x: center.x - 512 * s, y: center.y - 468 * s)
    ctx.scaleBy(x: s, y: s)
    if shadow {
        ctx.setShadow(offset: CGSize(width: 0, height: 18), blur: 40, color: color(0x1b1440, 0.35))
    }
    ctx.setStrokeColor(color(0xffffff))
    ctx.setFillColor(color(0xffffff))
    ctx.setLineWidth(56)
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)
    ctx.addPath(CGPath(roundedRect: CGRect(x: 232, y: 330, width: 560, height: 400), cornerWidth: 72, cornerHeight: 72, transform: nil))
    ctx.move(to: CGPoint(x: 400, y: 206))
    ctx.addLine(to: CGPoint(x: 512, y: 318))
    ctx.addLine(to: CGPoint(x: 624, y: 206))
    ctx.strokePath()
    ctx.move(to: CGPoint(x: 462, y: 440))
    ctx.addLine(to: CGPoint(x: 462, y: 620))
    ctx.addLine(to: CGPoint(x: 610, y: 530))
    ctx.closePath()
    ctx.fillPath()
    ctx.restoreGState()
}

func wordmark(_ ctx: CGContext, _ text: String, origin: CGPoint, height: CGFloat) {
    let font = CTFontCreateWithName("SFProRounded-Bold" as CFString, height, nil)
    let attrs: [NSAttributedString.Key: Any] = [
        NSAttributedString.Key(kCTFontAttributeName as String): font,
        NSAttributedString.Key(kCTForegroundColorAttributeName as String): color(0xffffff),
    ]
    let line = CTLineCreateWithAttributedString(NSAttributedString(string: text, attributes: attrs))
    ctx.saveGState()
    // Undo the flip for text so glyphs are upright.
    ctx.translateBy(x: origin.x, y: origin.y)
    ctx.scaleBy(x: 1, y: -1)
    ctx.textPosition = .zero
    CTLineDraw(line, ctx)
    ctx.restoreGState()
}

// ----- iOS -----------------------------------------------------------------------------------

write(render(1024, 1024, opaque: true) { ctx in
    background(ctx, 1024, 1024)
    glyph(ctx, center: CGPoint(x: 512, y: 520), size: 600)
}, "AppIcon.appiconset/AppIcon-1024.png")
writeJSON([
    "images": [["filename": "AppIcon-1024.png", "idiom": "universal", "platform": "ios", "size": "1024x1024"]],
    "info": info,
], "AppIcon.appiconset/Contents.json")

// ----- tvOS ----------------------------------------------------------------------------------

let brand = "App Icon & Top Shelf Image.brandassets"

func imagestack(_ name: String, width: Int, height: Int, scales: [Int]) {
    let stack = "\(brand)/\(name).imagestack"
    let layers: [(String, Bool, (CGContext, CGFloat, CGFloat) -> Void)] = [
        ("Front", false, { ctx, w, h in glyph(ctx, center: CGPoint(x: w / 2, y: h * 0.53), size: h * 0.62, shadow: true) }),
        ("Back", true, { ctx, w, h in background(ctx, w, h) }),
    ]
    for (layer, opaque, draw) in layers {
        let set = "\(stack)/\(layer).imagestacklayer/Content.imageset"
        var images: [[String: Any]] = []
        for scale in scales {
            let (w, h) = (width * scale, height * scale)
            let file = "\(layer.lowercased())-\(w)x\(h).png"
            write(render(w, h, opaque: opaque) { draw($0, CGFloat(w), CGFloat(h)) }, "\(set)/\(file)")
            images.append(["filename": file, "idiom": "tv", "scale": "\(scale)x"])
        }
        writeJSON(["images": images, "info": info], "\(set)/Contents.json")
        writeJSON(["info": info], "\(stack)/\(layer).imagestacklayer/Contents.json")
    }
    writeJSON(["info": info, "layers": layers.map { ["filename": "\($0.0).imagestacklayer"] }], "\(stack)/Contents.json")
}

func topShelf(_ name: String, width: Int, height: Int) {
    let set = "\(brand)/\(name).imageset"
    var images: [[String: Any]] = []
    for scale in [1, 2] {
        let (w, h) = (width * scale, height * scale)
        let file = "topshelf-\(w)x\(h).png"
        write(render(w, h, opaque: true) { ctx in
            let (cw, ch) = (CGFloat(w), CGFloat(h))
            background(ctx, cw, ch)
            let g = ch * 0.42
            let text = ch * 0.2
            let textWidth = text * 3.3
            let total = g * 1.06 + text * 0.4 + textWidth
            let left = (cw - total) / 2
            glyph(ctx, center: CGPoint(x: left + g * 0.53, y: ch * 0.5), size: g, shadow: true)
            wordmark(ctx, "FluxTV", origin: CGPoint(x: left + g * 1.06 + text * 0.4, y: ch * 0.5 + text * 0.36), height: text)
        }, "\(set)/\(file)")
        images.append(["filename": file, "idiom": "tv", "scale": "\(scale)x"])
    }
    writeJSON(["images": images, "info": info], "\(set)/Contents.json")
}

imagestack("App Icon", width: 400, height: 240, scales: [1, 2])
imagestack("App Icon - App Store", width: 1280, height: 768, scales: [1])
topShelf("Top Shelf Image", width: 1920, height: 720)
topShelf("Top Shelf Image Wide", width: 2320, height: 720)
writeJSON([
    "assets": [
        ["filename": "App Icon - App Store.imagestack", "idiom": "tv", "role": "primary-app-icon", "size": "1280x768"],
        ["filename": "App Icon.imagestack", "idiom": "tv", "role": "primary-app-icon", "size": "400x240"],
        ["filename": "Top Shelf Image Wide.imageset", "idiom": "tv", "role": "top-shelf-image-wide", "size": "2320x720"],
        ["filename": "Top Shelf Image.imageset", "idiom": "tv", "role": "top-shelf-image", "size": "1920x720"],
    ],
    "info": info,
], "\(brand)/Contents.json")

// ----- shared --------------------------------------------------------------------------------

writeJSON([
    "colors": [[
        "color": ["color-space": "srgb", "components": ["alpha": "1.000", "red": "0.424", "green": "0.549", "blue": "1.000"]],
        "idiom": "universal",
    ]],
    "info": info,
], "AccentColor.colorset/Contents.json")
writeJSON(["info": info], "Contents.json")
print("icons written to \(catalog.path)")
