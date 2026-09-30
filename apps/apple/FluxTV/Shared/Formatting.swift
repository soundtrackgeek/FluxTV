import Foundation
import SwiftUI

extension Channel: Identifiable {}

extension Channel {
    /// Name without the provider's `XX| ` prefix.
    var displayName: String { splitPrefix(name).rest }
    /// Provider prefix such as `NOR` or `UK`, if any.
    var prefix: String? { splitPrefix(name).prefix }
}

extension Category: Identifiable {}

extension Category {
    var displayName: String { splitPrefix(name).rest }
}

extension Programme: Identifiable {
    public var id: String { "\(channel)|\(start)" }

    var startDate: Date { Date(timeIntervalSince1970: TimeInterval(start)) }
    var stopDate: Date { Date(timeIntervalSince1970: TimeInterval(stop)) }

    func progress(at now: Date = .now) -> Double {
        let t = now.timeIntervalSince1970
        return min(1, max(0, (t - Double(start)) / max(1, Double(stop - start))))
    }

    func isOn(at now: Date = .now) -> Bool {
        let t = Int64(now.timeIntervalSince1970)
        return start <= t && t < stop
    }

    var timeRange: String { "\(formatTime(start)) – \(formatTime(stop))" }

    func minutesLeft(at now: Date = .now) -> Int {
        max(0, Int((Double(stop) - now.timeIntervalSince1970) / 60))
    }
}

/// Split the `XX| ` or `XX: ` prefix providers put in front of channel and category names.
func splitPrefix(_ name: String) -> (prefix: String?, rest: String) {
    var trimmed = name.trimmingCharacters(in: .whitespaces)
    // Some providers wrap the prefix: "|SPT| SPORT BOXING".
    if trimmed.hasPrefix("|") { trimmed = String(trimmed.dropFirst()) }
    guard let sep = trimmed.firstIndex(where: { $0 == "|" || $0 == ":" }) else { return (nil, trimmed) }
    let prefix = trimmed[..<sep].trimmingCharacters(in: .whitespaces)
    let rest = trimmed[trimmed.index(after: sep)...].trimmingCharacters(in: .whitespaces)
    // Leave times like "20:00 Match" alone.
    let looksLikeTime = trimmed.prefix(8).range(of: #"\d{2}:\d{2}"#, options: .regularExpression) != nil
    guard !prefix.isEmpty, prefix.count <= 10, !rest.isEmpty, !looksLikeTime else { return (nil, trimmed) }
    return (prefix, rest)
}

func initials(_ name: String) -> String {
    let words = splitPrefix(name).rest
        .components(separatedBy: CharacterSet.alphanumerics.inverted)
        .filter { !$0.isEmpty }
    let first = words.first?.first.map(String.init) ?? "?"
    let second = words.dropFirst().first?.first.map(String.init) ?? ""
    return (first + second).uppercased()
}

private let timeFormatter: DateFormatter = {
    let f = DateFormatter()
    f.timeStyle = .short
    f.dateStyle = .none
    return f
}()

func formatTime(_ unix: Int64) -> String {
    timeFormatter.string(from: Date(timeIntervalSince1970: TimeInterval(unix)))
}

func formatAgo(_ unix: Int64?) -> String {
    guard let unix else { return "never" }
    let date = Date(timeIntervalSince1970: TimeInterval(unix))
    if Date.now.timeIntervalSince(date) < 90 { return "just now" }
    return date.formatted(.relative(presentation: .named))
}

func formatBitrate(_ bps: UInt64) -> String {
    bps >= 1_000_000 ? String(format: "%.1f Mbit/s", Double(bps) / 1e6) : "\(bps / 1000) kbit/s"
}

extension View {
    /// A search field that filters the current list. iOS only: on tvOS it would put a full
    /// keyboard above every list, and the Search tab covers it.
    @ViewBuilder
    func listFilter(text: Binding<String>, prompt: String) -> some View {
        #if os(tvOS)
        self
        #else
        searchable(text: text, prompt: prompt)
        #endif
    }
}
