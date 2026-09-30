import SwiftUI
import UIKit

/// In-memory cache of decoded, downsampled images. Channel logos are tiny but numerous, and
/// many providers' logo links are dead, so failures are remembered too.
final class ImageCache: @unchecked Sendable {
    static let shared = ImageCache()

    private let images = NSCache<NSString, UIImage>()
    private let lock = NSLock()
    private var failed = Set<String>()
    private let session: URLSession = {
        let config = URLSessionConfiguration.default
        config.timeoutIntervalForRequest = 15
        config.urlCache = URLCache(memoryCapacity: 16 << 20, diskCapacity: 200 << 20)
        config.requestCachePolicy = .returnCacheDataElseLoad
        config.httpMaximumConnectionsPerHost = 6
        return URLSession(configuration: config)
    }()

    init() {
        images.totalCostLimit = 64 << 20
    }

    func cached(_ url: String) -> UIImage? {
        images.object(forKey: url as NSString)
    }

    func hasFailed(_ url: String) -> Bool {
        lock.withLock { failed.contains(url) }
    }

    func load(_ url: String, maxPixel: CGFloat) async -> UIImage? {
        if let image = cached(url) { return image }
        guard !hasFailed(url), let u = URL(string: url), u.scheme?.hasPrefix("http") == true else { return nil }
        do {
            let (data, response) = try await session.data(from: u)
            guard (response as? HTTPURLResponse)?.statusCode ?? 200 < 400,
                  let image = UIImage(data: data)
            else { throw URLError(.cannotDecodeContentData) }
            let scaled = await image.byPreparingThumbnail(ofSize: fit(image.size, maxPixel)) ?? image
            images.setObject(scaled, forKey: url as NSString, cost: Int(scaled.size.width * scaled.size.height * 4))
            return scaled
        } catch {
            if !(error is CancellationError), (error as? URLError)?.code != .cancelled {
                _ = lock.withLock { failed.insert(url) }
            }
            return nil
        }
    }

    private func fit(_ size: CGSize, _ maxPixel: CGFloat) -> CGSize {
        let scale = min(1, maxPixel / max(size.width, size.height, 1))
        return CGSize(width: max(1, size.width * scale), height: max(1, size.height * scale))
    }
}

/// A remote image with a placeholder, loaded through `ImageCache`.
struct RemoteImage<Placeholder: View>: View {
    let url: String?
    var maxPixel: CGFloat = 256
    var contentMode: ContentMode = .fit
    @ViewBuilder var placeholder: () -> Placeholder

    @State private var image: UIImage?

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image).resizable().aspectRatio(contentMode: contentMode)
            } else {
                placeholder()
            }
        }
        .task(id: url) {
            guard let url else { image = nil; return }
            image = ImageCache.shared.cached(url)
            if image == nil { image = await ImageCache.shared.load(url, maxPixel: maxPixel) }
        }
    }
}

/// Channel logo on a neutral tile, falling back to coloured initials.
struct ChannelLogo: View {
    let channel: Channel
    var size: CGFloat = 44

    var body: some View {
        RemoteImage(url: channel.logo, maxPixel: size * 3) {
            let hue = Double(abs(channel.name.hashValueStable) % 360) / 360
            ZStack {
                RoundedRectangle(cornerRadius: size * 0.2).fill(Color(hue: hue, saturation: 0.45, brightness: 0.55))
                Text(initials(channel.name))
                    .font(.system(size: size * 0.36, weight: .semibold, design: .rounded))
                    .foregroundStyle(.white)
            }
        }
        .padding(channel.logo == nil ? 0 : size * 0.08)
        .frame(width: size, height: size)
        .background(RoundedRectangle(cornerRadius: size * 0.2).fill(.quaternary.opacity(0.6)))
        .clipShape(RoundedRectangle(cornerRadius: size * 0.2))
    }
}

extension String {
    /// Deterministic hash (Swift's `hashValue` changes between launches).
    var hashValueStable: Int {
        var h: Int32 = 0
        for u in unicodeScalars { h = h &* 31 &+ Int32(truncatingIfNeeded: u.value) }
        return Int(h)
    }
}
