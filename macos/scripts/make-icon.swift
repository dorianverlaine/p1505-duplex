// Renders the app icon (a white printer with a flip arrow on a blue
// squircle) into an .icns. Usage: swift make-icon.swift OUT.icns
import AppKit

let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "AppIcon.icns"
let iconset = URL(fileURLWithPath: NSTemporaryDirectory()).appending(path: "AppIcon.iconset")
try? FileManager.default.removeItem(at: iconset)
try! FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

func render(_ px: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let s = CGFloat(px)
    // Standard macOS icon grid: 824/1024 body with rounded corners.
    let inset = s * 100 / 1024
    let body = NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
    let path = NSBezierPath(roundedRect: body, xRadius: s * 185 / 1024, yRadius: s * 185 / 1024)
    NSGradient(colors: [NSColor(srgbRed: 0.20, green: 0.55, blue: 1.0, alpha: 1),
                        NSColor(srgbRed: 0.33, green: 0.30, blue: 0.90, alpha: 1)])!
        .draw(in: path, angle: -90)

    func symbol(_ name: String, points: CGFloat, at center: NSPoint) {
        let config = NSImage.SymbolConfiguration(pointSize: points, weight: .semibold)
            .applying(.init(paletteColors: [.white]))
        guard let img = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(config) else { return }
        let size = img.size
        img.draw(in: NSRect(x: center.x - size.width / 2, y: center.y - size.height / 2,
                            width: size.width, height: size.height))
    }
    symbol("printer.fill", points: s * 0.36, at: NSPoint(x: s * 0.5, y: s * 0.54))
    symbol("arrow.2.squarepath", points: s * 0.16, at: NSPoint(x: s * 0.5, y: s * 0.26))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for base in [16, 32, 128, 256, 512] {
    try! render(base).write(to: iconset.appending(path: "icon_\(base)x\(base).png"))
    try! render(base * 2).write(to: iconset.appending(path: "icon_\(base)x\(base)@2x.png"))
}
let p = Process()
p.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
p.arguments = ["-c", "icns", iconset.path, "-o", out]
try! p.run()
p.waitUntilExit()
exit(p.terminationStatus)
