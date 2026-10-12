// usage: swift decode-qr.swift <image.png>   Prints the text of the first QR code found.
// Evaluator tooling only (not application code): decodes with the system's own CoreImage detector.
import CoreImage
import Foundation

guard CommandLine.arguments.count == 2,
      let image = CIImage(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])),
      let detector = CIDetector(ofType: CIDetectorTypeQRCode, context: nil, options: [CIDetectorAccuracy: CIDetectorAccuracyHigh]),
      let feature = detector.features(in: image).first as? CIQRCodeFeature,
      let message = feature.messageString
else {
    FileHandle.standardError.write(Data("no QR code found\n".utf8))
    exit(1)
}
print(message)
