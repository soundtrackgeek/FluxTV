import SwiftUI

/// A growing timeline, with bounded queries around the horizontal viewport.
struct GuideTimeline: Equatable {
    static let slot: Int64 = 1800
    static let extensionSeconds: Int64 = 6 * 3600
    var from: Int64
    var to: Int64

    init(around time: Int64) {
        from = time - time % Self.slot - 3600
        to = from + Self.extensionSeconds
    }

    func visibleRange(offset: CGFloat, width: CGFloat, pointsPerMinute: CGFloat) -> Range<Int64> {
        let start = from + Int64(max(0, offset) / pointsPerMinute * 60)
        let stop = start + Int64(max(60, width) / pointsPerMinute * 60)
        return (start - start % Self.slot - Self.slot)..<((stop + Self.slot - 1) / Self.slot * Self.slot + Self.slot)
    }
}

struct GuideRequest: Hashable {
    let sourceId: Int64
    let epgId: String
    let version: Int
    let from: Int64
    let to: Int64
}

struct GuideView: View {
    @Environment(AppModel.self) private var app
    @State private var categories: [Category] = []
    @AppStorage("guideSelection") private var selection = GuideView.favoritesTag
    @State private var channels: [Channel] = []
    @State private var timeline = GuideTimeline(around: Int64(Date.now.timeIntervalSince1970))
    @State private var range: Range<Int64> = 0..<0
    @State private var horizontalPosition = ScrollPosition(x: 0)
    @State private var horizontalOffset: CGFloat = 0
    @State private var viewportWidth: CGFloat = 0
    @State private var loaded = false
    @State private var selected: GuideSelection?
    @State private var day = Date.now
    @State private var jumpTime = Date.now
    @State private var navigationDate: Date?

    nonisolated static let favoritesTag = "\u{1}favorites"

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
            VStack(spacing: 0) {
                navigation
                if channels.isEmpty {
                    if loaded {
                        ContentUnavailableView("No Guide", systemImage: "calendar",
                            description: Text(selection == Self.favoritesTag
                                ? "Add channels to your favorites, or pick a category."
                                : "No channels in this category have guide data."))
                    } else {
                        ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                } else {
                    grid
                }
            }
            .navigationTitle("TV Guide")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar { ToolbarItem(placement: .principal) { categoryPicker } }
            .sheet(item: $selected) { s in
                ProgrammeDetailView(channel: s.channel, programme: s.programme, channels: channels)
            }
        }
        .task(id: "\(app.source?.id ?? 0)|\(app.catalogVersion)") { await loadCategories() }
        .task(id: "\(app.source?.id ?? 0)|\(selection)|\(app.catalogVersion)|\(app.favoritesVersion)") { await loadChannels() }
    }

    private var categoryPicker: some View {
        Picker("Channels", selection: $selection) {
            Label("Favorites", systemImage: "star.fill").tag(Self.favoritesTag)
            ForEach(categories) { c in Text(c.displayName).tag(c.id) }
        }
        .pickerStyle(.menu)
    }

    private var navigation: some View {
        ViewThatFits(in: .horizontal) {
            HStack { dayControls; timeControls; legend }
            VStack(spacing: 8) { HStack { dayControls }; HStack { timeControls; legend } }
        }
        .font(.caption)
        .padding(8)
        .background(.background)
    }

    private var dayControls: some View {
        Group {
            Button { changeDay(-1) } label: { Image(systemName: "chevron.left") }
                .accessibilityLabel("Previous day")
            #if os(tvOS)
            Menu(day.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day())) {
                ForEach(-7...7, id: \.self) { offset in
                    let date = Calendar.current.date(byAdding: .day, value: offset, to: .now) ?? .now
                    Button(date.formatted(.dateTime.weekday(.wide).month().day())) { chooseDay(date) }
                }
            }
            .accessibilityLabel("Guide day")
            #else
            DatePicker("Guide day", selection: Binding(get: { day }, set: { chooseDay($0) }), displayedComponents: .date)
                .labelsHidden().accessibilityLabel("Guide day")
            #endif
            Button { changeDay(1) } label: { Image(systemName: "chevron.right") }
                .accessibilityLabel("Next day")
            Button("Now") { jump(to: .now) }
        }
    }

    private var timeControls: some View {
        Group {
            #if os(tvOS)
            Menu("Jump to time") {
                ForEach(0..<24, id: \.self) { hour in
                    Menu(String(format: "%02d:00", hour)) {
                        ForEach([0, 15, 30, 45], id: \.self) { minute in
                            Button(String(format: "%02d:%02d", hour, minute)) {
                                if let date = Calendar.current.date(bySettingHour: hour, minute: minute, second: 0, of: day) {
                                    jumpTime = date
                                    jump(to: date)
                                }
                            }
                        }
                    }
                }
            }
            #else
            DatePicker("Jump to time", selection: $jumpTime, displayedComponents: .hourAndMinute)
                .labelsHidden().accessibilityLabel("Jump to time")
            Button("Jump") { chooseDay(day) }
            #endif
        }
    }

    private var legend: some View {
        HStack(spacing: 8) {
            Text("● Sports").foregroundStyle(.green)
            Text("● News").foregroundStyle(.blue)
            Text("● Film").foregroundStyle(.purple)
        }
        .font(.caption2)
    }

    private var grid: some View {
        GeometryReader { geometry in
            ScrollView(.horizontal) {
                ScrollView(.vertical) {
                    LazyVStack(alignment: .leading, spacing: 0, pinnedViews: [.sectionHeaders]) {
                        Section {
                            ForEach(channels) { channel in
                                GuideProgrammeRow(channel: channel,
                                    request: GuideRequest(sourceId: app.source?.id ?? 0, epgId: channel.epgId ?? "",
                                        version: app.catalogVersion, from: range.lowerBound, to: range.upperBound),
                                    timeline: timeline, horizontalOffset: horizontalOffset, channelWidth: channelWidth,
                                    rowHeight: rowHeight, pointsPerMinute: pointsPerMinute,
                                    watch: { watch(channel) }, select: { selected = GuideSelection(channel: channel, programme: $0) },
                                    focusTime: { reveal($0) })
                            }
                        } header: { timeHeader }
                    }
                }
                .frame(width: channelWidth + x(timeline.to), height: geometry.size.height)
            }
            .scrollPosition($horizontalPosition)
            .onScrollGeometryChange(for: CGFloat.self) { max(0, $0.contentOffset.x) } action: { _, offset in
                horizontalOffset = offset
                updateRange(width: geometry.size.width)
            }
            .onChange(of: geometry.size.width, initial: true) { _, width in
                viewportWidth = width
                updateRange(width: width)
            }
            .onChange(of: navigationDate) { _, date in
                if let date { horizontalPosition.scrollTo(x: max(0, x(Int64(date.timeIntervalSince1970)) - 30 * pointsPerMinute)) }
            }
            .onAppear { jump(to: navigationDate ?? .now) }
        }
    }

    private var timeHeader: some View {
        ZStack(alignment: .leading) {
            ForEach(Array(stride(from: max(timeline.from, range.lowerBound), to: min(timeline.to, range.upperBound), by: Int(GuideTimeline.slot))), id: \.self) { slot in
                VStack(alignment: .leading, spacing: 0) {
                    Text(formatTime(slot)).font(.caption.monospacedDigit())
                    if Calendar.current.component(.hour, from: Date(timeIntervalSince1970: Double(slot))) == 0 {
                        Text(Date(timeIntervalSince1970: Double(slot)).formatted(.dateTime.month(.abbreviated).day())).font(.caption2)
                    }
                }
                .offset(x: channelWidth + x(slot))
            }
            Text(day.formatted(.dateTime.weekday(.abbreviated).day()))
                .font(.caption.weight(.semibold))
                .frame(width: channelWidth, height: 40, alignment: .leading)
                .background(.background)
                .offset(x: horizontalOffset)
                .zIndex(1)
        }
        .frame(width: channelWidth + x(timeline.to), height: 40, alignment: .leading)
        .background(.background)
    }

    private func x(_ time: Int64) -> CGFloat { CGFloat(time - timeline.from) / 60 * pointsPerMinute }

    private func updateRange(width: CGFloat) {
        let next = timeline.visibleRange(offset: horizontalOffset, width: width - channelWidth, pointsPerMinute: pointsPerMinute)
        if range != next { range = next }
        let date = Date(timeIntervalSince1970: Double(timeline.from) + Double(horizontalOffset / pointsPerMinute * 60) + Double(GuideTimeline.slot))
        if !Calendar.current.isDate(day, inSameDayAs: date) { day = date }
        if horizontalOffset < 30 * pointsPerMinute {
            timeline.from -= GuideTimeline.extensionSeconds
            let offset = horizontalOffset + CGFloat(GuideTimeline.extensionSeconds) / 60 * pointsPerMinute
            horizontalOffset = offset
            horizontalPosition.scrollTo(x: offset)
        } else if horizontalOffset + width > channelWidth + x(timeline.to) - 30 * pointsPerMinute {
            timeline.to += GuideTimeline.extensionSeconds
        }
    }

    private func reveal(_ programme: Programme) {
        let left = x(programme.start)
        let right = min(x(programme.stop), left + 30 * pointsPerMinute)
        if right < horizontalOffset || left > horizontalOffset + viewportWidth - channelWidth {
            horizontalPosition.scrollTo(x: max(0, left - 15 * pointsPerMinute))
        }
    }

    private func jump(to date: Date) {
        timeline = GuideTimeline(around: Int64(date.timeIntervalSince1970))
        horizontalOffset = max(0, x(Int64(date.timeIntervalSince1970)) - 30 * pointsPerMinute)
        range = timeline.visibleRange(offset: horizontalOffset, width: viewportWidth - channelWidth, pointsPerMinute: pointsPerMinute)
        day = date
        navigationDate = date
        horizontalPosition.scrollTo(x: horizontalOffset)
    }

    private func chooseDay(_ date: Date) {
        let parts = Calendar.current.dateComponents([.hour, .minute], from: jumpTime)
        if let value = Calendar.current.date(bySettingHour: parts.hour ?? 0, minute: parts.minute ?? 0, second: 0, of: date) { jump(to: value) }
    }

    private func changeDay(_ delta: Int) {
        if let date = Calendar.current.date(byAdding: .day, value: delta, to: day) { chooseDay(date) }
    }

    private func watch(_ channel: Channel) {
        if let id = app.source?.id { app.player.play(channel, in: channels, sourceId: id) }
    }

    private func loadCategories() async {
        guard let id = app.source?.id else { return }
        let list = ((try? await app.run { try $0.categories(sourceId: id, kind: .live) }) ?? []).filter { $0.channelCount > 0 }
        guard !Task.isCancelled else { return }
        categories = list
        let favorites = (try? await app.run { try $0.favoriteIds(sourceId: id, kind: .live) }) ?? []
        guard !Task.isCancelled else { return }
        let known = selection == Self.favoritesTag ? !favorites.isEmpty : categories.contains { $0.id == selection }
        guard !known else { return }
        let candidates = categories.prefix(60).map(\.id)
        let pick = try? await app.run { engine in
            try candidates.first { cat in try engine.channels(sourceId: id, kind: .live, categoryId: cat).filter { $0.epgId != nil }.count >= 3 }
        }
        guard !Task.isCancelled else { return }
        if let pick = pick ?? categories.first?.id { selection = pick }
    }

    private func loadChannels() async {
        loaded = false
        channels = []
        guard let id = app.source?.id else { loaded = true; return }
        let selection = selection
        let list = (try? await app.run { engine in
            selection == Self.favoritesTag ? try engine.favorites(sourceId: id, kind: .live)
                : try engine.channels(sourceId: id, kind: .live, categoryId: selection)
        }) ?? []
        guard !Task.isCancelled else { return }
        channels = list.filter { $0.epgId != nil }
        loaded = true
    }
}

struct GuideSelection: Identifiable {
    let channel: Channel
    let programme: Programme
    var id: String { "\(channel.id)|\(programme.id)" }
}

/// State belongs to a lazy row and is discarded when the row leaves the viewport.
private struct GuideProgrammeRow: View {
    @Environment(AppModel.self) private var app
    let channel: Channel
    let request: GuideRequest
    let timeline: GuideTimeline
    let horizontalOffset: CGFloat
    let channelWidth: CGFloat
    let rowHeight: CGFloat
    let pointsPerMinute: CGFloat
    let watch: () -> Void
    let select: (Programme) -> Void
    let focusTime: (Programme) -> Void
    @State private var programmes: [Programme] = []
    @State private var loading = true
    @State private var failed = false
    @State private var retry = 0
    @FocusState private var focused: String?

    private func x(_ time: Int64) -> CGFloat { CGFloat(time - timeline.from) / 60 * pointsPerMinute }

    var body: some View {
        HStack(spacing: 0) {
            Button(action: watch) {
                HStack(spacing: 8) {
                    ChannelLogo(channel: channel, size: rowHeight * 0.6)
                    Text(channel.displayName).font(.caption).lineLimit(2)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 6)
                .frame(width: channelWidth, height: rowHeight)
                .background(.background)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Watch \(channel.displayName)")
            .offset(x: horizontalOffset)
            .zIndex(2)
            ZStack(alignment: .leading) {
                if programmes.isEmpty {
                    HStack {
                        Text(loading ? "Loading…" : failed ? "Guide unavailable" : "No programmes in this time range").font(.caption).foregroundStyle(.secondary)
                        if failed { Button("Retry") { retry += 1 } }
                    }
                    .offset(x: horizontalOffset + 8)
                }
                ForEach(programmes.filter { $0.stop > max(request.from, timeline.from) && $0.start < min(request.to, timeline.to) }) { programme in
                    let start = max(programme.start, timeline.from)
                    let stop = min(programme.stop, timeline.to)
                    ProgrammeCell(programme: programme, width: max(2, x(stop) - x(start)), height: rowHeight) { select(programme) }
                        .focused($focused, equals: programme.id)
                        .offset(x: x(start))
                }
                TimelineView(.periodic(from: .now, by: 60)) { context in
                    let time = Int64(context.date.timeIntervalSince1970)
                    if time >= timeline.from && time < timeline.to {
                        Rectangle().fill(.red).frame(width: 2, height: rowHeight).offset(x: x(time)).allowsHitTesting(false)
                    }
                }
            }
            .frame(width: x(timeline.to), height: rowHeight, alignment: .leading)
        }
        .frame(height: rowHeight)
        #if os(tvOS)
        .focusSection()
        #endif
        .onChange(of: focused) { _, id in
            if let programme = programmes.first(where: { $0.id == id }) { focusTime(programme) }
        }
        .task(id: "\(request)|\(retry)") { await load() }
        .onDisappear { programmes = [] }
    }

    private func load() async {
        loading = true
        failed = false
        do {
            try await Task.sleep(for: .milliseconds(80))
            let result = try await app.run { try $0.guide(sourceId: request.sourceId, epgIds: [request.epgId], from: request.from, to: request.to) }
            guard !Task.isCancelled else { return }
            programmes = result.first?.programmes ?? []
            loading = false
        } catch {
            guard !Task.isCancelled else { return }
            loading = false
            failed = true
        }
    }
}

func programmeCategoryColor(_ category: String?) -> Color {
    let value = category?.lowercased() ?? ""
    if ["sport", "football", "soccer", "tennis", "basketball"].contains(where: value.contains) { return .green }
    if ["news", "current affairs", "nyhet"].contains(where: value.contains) { return .blue }
    if ["film", "movie", "cinema"].contains(where: value.contains) { return .purple }
    return .gray
}

private struct ProgrammeCell: View {
    @Environment(\.isFocused) private var isFocused
    let programme: Programme
    let width: CGFloat
    let height: CGFloat
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            TimelineView(.periodic(from: .now, by: 60)) { context in
                let color = programmeCategoryColor(programme.category)
                VStack(alignment: .leading, spacing: 2) {
                    Text(programme.title).font(.caption.weight(.semibold)).lineLimit(1)
                    Text(programme.timeRange).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                }
                .padding(.horizontal, 8)
                .frame(width: max(0, width - 4), height: height - 4, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 6).fill(color.opacity(programme.isOn(at: context.date) ? 0.32 : 0.16)))
                .overlay(alignment: .leading) { RoundedRectangle(cornerRadius: 2).fill(color).frame(width: 3) }
                .overlay { RoundedRectangle(cornerRadius: 6).stroke(isFocused ? Color.accentColor : .clear, lineWidth: 3) }
                .opacity(programme.stopDate <= context.date ? 0.6 : 1)
            }
        }
        .buttonStyle(.plain)
        .frame(width: width, height: height)
        .accessibilityLabel("\(programme.title), \(programme.timeRange)\(programme.category.map { ", \($0)" } ?? "")")
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
