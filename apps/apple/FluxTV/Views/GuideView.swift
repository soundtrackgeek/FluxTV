import SwiftUI

/// TV guide grid: channels down the side, a 12-hour timeline across.
struct GuideView: View {
    @Environment(AppModel.self) private var app
    @State private var categories: [Category] = []
    @AppStorage("guideSelection") private var selection = GuideView.favoritesTag
    @State private var channels: [Channel] = []
    @State private var rows: [String: [Programme]] = [:]
    @State private var window: ClosedRange<Int64> = 0...0
    @State private var loaded = false
    @State private var selected: GuideSelection?

    nonisolated static let favoritesTag = "\u{1}favorites"
    private let maxChannels = 250

    #if os(tvOS)
    private let rowHeight: CGFloat = 96
    private let pointsPerMinute: CGFloat = 9
    private let channelWidth: CGFloat = 340
    #else
    private let rowHeight: CGFloat = 60
    private let pointsPerMinute: CGFloat = 4.5
    private let channelWidth: CGFloat = 132
    #endif

    var body: some View {
        NavigationStack {
            Group {
                if channels.isEmpty {
                    if loaded {
                        ContentUnavailableView(
                            "No Guide", systemImage: "calendar",
                            description: Text(selection == Self.favoritesTag
                                ? "Add channels to your favorites, or pick a category."
                                : "No channels in this category have guide data."))
                    } else {
                        ProgressView()
                    }
                } else {
                    grid
                }
            }
            .navigationTitle("TV Guide")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .principal) { categoryPicker }
            }
            .sheet(item: $selected) { s in
                ProgrammeDetailView(channel: s.channel, programme: s.programme, channels: channels)
            }
        }
        .task(id: app.catalogVersion) { await loadCategories() }
        .task(id: "\(selection)|\(app.catalogVersion)|\(app.favoritesVersion)") { await loadGuide() }
    }

    private var categoryPicker: some View {
        Picker("Channels", selection: $selection) {
            Label("Favorites", systemImage: "star.fill").tag(Self.favoritesTag)
            ForEach(categories) { c in Text(c.displayName).tag(c.id) }
        }
        .pickerStyle(.menu)
    }

    private var grid: some View {
        ScrollView(.vertical) {
            HStack(alignment: .top, spacing: 0) {
                VStack(spacing: 0) {
                    Color.clear.frame(height: 32)
                    ForEach(channels) { ch in
                        Button {
                            if let id = app.source?.id { app.player.play(ch, in: channels, sourceId: id) }
                        } label: {
                            HStack(spacing: 8) {
                                ChannelLogo(channel: ch, size: rowHeight * 0.6)
                                Text(ch.displayName).font(.caption).lineLimit(2)
                                Spacer(minLength: 0)
                            }
                            .padding(.horizontal, 6)
                            .frame(width: channelWidth, height: rowHeight)
                        }
                        .buttonStyle(.plain)
                    }
                }
                ScrollViewReader { proxy in
                    ScrollView(.horizontal) {
                        VStack(alignment: .leading, spacing: 0) {
                            timeHeader
                            ForEach(channels) { ch in
                                programmeRow(ch)
                            }
                        }
                        .overlay(alignment: .topLeading) { nowLine }
                    }
                    .onAppear { proxy.scrollTo(nowSlot, anchor: .leading) }
                    .onChange(of: window) { proxy.scrollTo(nowSlot, anchor: .leading) }
                }
            }
        }
    }

    private var slots: [Int64] { Array(stride(from: window.lowerBound, to: window.upperBound, by: 1800)) }

    private var nowSlot: Int64 {
        let now = Int64(Date.now.timeIntervalSince1970)
        return slots.last(where: { $0 <= now - 900 }) ?? window.lowerBound
    }

    private func x(_ t: Int64) -> CGFloat {
        CGFloat(t - window.lowerBound) / 60 * pointsPerMinute
    }

    private var timeHeader: some View {
        HStack(spacing: 0) {
            ForEach(slots, id: \.self) { slot in
                Text(formatTime(slot))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(width: 30 * pointsPerMinute, height: 32, alignment: .leading)
                    .padding(.leading, 4)
                    .id(slot)
            }
        }
    }

    private var nowLine: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            let t = Int64(context.date.timeIntervalSince1970)
            if window.contains(t) {
                Rectangle()
                    .fill(.red)
                    .frame(width: 2, height: 32 + rowHeight * CGFloat(channels.count))
                    .offset(x: x(t))
                    .allowsHitTesting(false)
            }
        }
    }

    /// Cells for one row: each programme clipped to the window, with the gap before it.
    private func layout(_ programmes: [Programme]) -> [(programme: Programme, gap: CGFloat, width: CGFloat)] {
        var cursor = window.lowerBound
        var out: [(Programme, CGFloat, CGFloat)] = []
        for p in programmes {
            let start = max(p.start, cursor)
            let stop = min(p.stop, window.upperBound)
            guard stop > start else { continue }
            out.append((p, x(start) - x(cursor), x(stop) - x(start)))
            cursor = stop
        }
        return out
    }

    private func programmeRow(_ ch: Channel) -> some View {
        let cells = layout(ch.epgId.flatMap { rows[$0] } ?? [])
        return HStack(spacing: 0) {
            ForEach(cells, id: \.programme.id) { cell in
                if cell.gap > 0 { Color.clear.frame(width: cell.gap) }
                ProgrammeCell(programme: cell.programme, width: cell.width, height: rowHeight) {
                    selected = GuideSelection(channel: ch, programme: cell.programme)
                }
            }
            if cells.isEmpty {
                Text("No guide data")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .padding(.leading, 8)
            }
        }
        .frame(width: x(window.upperBound), height: rowHeight, alignment: .leading)
    }

    private func loadCategories() async {
        guard let id = app.source?.id else { return }
        categories = ((try? await app.run { try $0.categories(sourceId: id, kind: .live) }) ?? []).filter { $0.channelCount > 0 }
        let favorites = (try? await app.run { try $0.favoriteIds(sourceId: id, kind: .live) }) ?? []
        let known = selection == Self.favoritesTag ? !favorites.isEmpty : categories.contains { $0.id == selection }
        guard !known else { return }
        // Start with the first category that actually has guide data.
        let candidates = categories.prefix(60).map(\.id)
        let pick = try? await app.run { engine in
            try candidates.first { cat in
                try engine.channels(sourceId: id, kind: .live, categoryId: cat).filter { $0.epgId != nil }.count >= 3
            }
        }
        if let pick = pick ?? categories.first?.id { selection = pick }
    }

    private func loadGuide() async {
        guard let id = app.source?.id else { return }
        let now = Int64(Date.now.timeIntervalSince1970)
        let from = now - now % 1800 - 1800
        let to = from + 12 * 3600
        let selection = selection
        let list: [Channel] = (try? await app.run { engine in
            selection == Self.favoritesTag
                ? try engine.favorites(sourceId: id, kind: .live)
                : try engine.channels(sourceId: id, kind: .live, categoryId: selection)
        }) ?? []
        let withGuide = Array(list.filter { $0.epgId != nil }.prefix(maxChannels))
        let ids = Array(Set(withGuide.compactMap(\.epgId)))
        let guide = (try? await app.run { try $0.guide(sourceId: id, epgIds: ids, from: from, to: to) }) ?? []
        guard !Task.isCancelled else { return }
        rows = Dictionary(guide.map { ($0.epgId, $0.programmes) }, uniquingKeysWith: { a, _ in a })
        channels = withGuide
        window = from...to
        loaded = true
    }
}

struct GuideSelection: Identifiable {
    let channel: Channel
    let programme: Programme
    var id: String { programme.id }
}

private struct ProgrammeCell: View {
    let programme: Programme
    let width: CGFloat
    let height: CGFloat
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            TimelineView(.periodic(from: .now, by: 60)) { context in
                let on = programme.isOn(at: context.date)
                VStack(alignment: .leading, spacing: 2) {
                    Text(programme.title).font(.caption.weight(.semibold)).lineLimit(1)
                    Text(programme.timeRange).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
                .padding(.horizontal, 6)
                .frame(width: max(0, width - 2), height: height - 4, alignment: .leading)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .fill(on ? AnyShapeStyle(.tint.opacity(0.28)) : AnyShapeStyle(.quaternary.opacity(0.7))))
            }
        }
        .buttonStyle(.plain)
        .frame(width: width, height: height)
    }
}

struct ProgrammeDetailView: View {
    let channel: Channel
    let programme: Programme
    let channels: [Channel]
    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    HStack(spacing: 12) {
                        ChannelLogo(channel: channel, size: 52)
                        VStack(alignment: .leading) {
                            Text(channel.displayName).font(.headline)
                            Text("\(programme.startDate.formatted(.dateTime.weekday(.wide))) \(programme.timeRange)")
                                .foregroundStyle(.secondary)
                        }
                    }
                    Text(programme.title).font(.title2.bold())
                    if let sub = programme.subtitle, !sub.isEmpty { Text(sub).font(.headline).foregroundStyle(.secondary) }
                    HStack {
                        if let c = programme.category, !c.isEmpty { Text(c) }
                        if let e = programme.episode, !e.isEmpty { Text(e) }
                    }
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    if programme.isOn() {
                        ProgressView(value: programme.progress()) {
                            Text("\(programme.minutesLeft()) min left").font(.caption)
                        }
                    }
                    if let d = programme.description, !d.isEmpty { Text(d) }
                    Button {
                        dismiss()
                        if let id = app.source?.id { app.player.play(channel, in: channels, sourceId: id) }
                    } label: {
                        Label("Watch \(channel.displayName)", systemImage: "play.fill")
                    }
                    .buttonStyle(.borderedProminent)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
            }
            #if os(iOS)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            #endif
        }
        #if os(iOS)
        .presentationDetents([.medium, .large])
        #endif
    }
}
