import AppKit
import Foundation
import ImageIO

let source = URL(fileURLWithPath: CommandLine.arguments[1])
let destination = URL(fileURLWithPath: CommandLine.arguments[2])
let imageSource = CGImageSourceCreateWithURL(source as CFURL, nil)!
let image = CGImageSourceCreateImageAtIndex(imageSource, 0, nil)!
precondition(image.width == 36 && image.height == 36)

var rgba = [UInt8](repeating: 0, count: 36 * 36 * 4)
let context = CGContext(
    data: &rgba,
    width: 36,
    height: 36,
    bitsPerComponent: 8,
    bytesPerRow: 36 * 4,
    space: CGColorSpaceCreateDeviceRGB(),
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
)!
context.draw(image, in: CGRect(x: 0, y: 0, width: 36, height: 36))
try Data(rgba).write(to: destination)
