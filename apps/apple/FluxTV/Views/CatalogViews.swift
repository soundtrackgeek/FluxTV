import SwiftUI

/// Destinations inside the Live TV and Movies tabs.
enum CatalogRoute: Hashable {
    case favorites(ContentKind)
    case recent(ContentKind)
    case category(ContentKind, id: String, name: String)

    var kind: ContentKind {
        switch self {
        case .favorites(let k), .recent(let k), .category(let k, _, _): k
        }
    }

    var title: String {
        switch self {
        case .favorites: "Favorites"
        case .recent: "Recently Watched"
        case .category(_, _, let name): name
        }
    }
}

/// Categories of one kind, plus Favorites and Recently Watched.
struct CategoryListView: View {
    let kind: ContentKind
    @Environment(AppModel.self) private var app
    @State private var categories: [Category] = []
    @State private var favoriteCount = 0
    @State private var loaded = false
    @State private var filter = ""

    private var filtered: [Category] {
        let f = filter.trimmingCharacters(in: .whitespaces)
        guard !f.isEmpty else { return categories }
        return categories.filter { $0.name.localizedCaseInsensitiveContains(f) }
    }

    var body: some View {
        List {
            if filter.isEmpty {
                Section {
                    NavigationLink(value: CatalogRoute.favorites(kind)) {
                        LabeledContent {
                            Text(favoriteCount, format: .number)
                        } label: {
                            Label("Favorites", systemImage: "star.fill")
                        }
                    }
                    NavigationLink(value: CatalogRoute.recent(kind)) {
                        Label("Recently Watched", systemImage: "clock.arrow.circlepath")
                    }
                }
            }
            Section(filter.isEmpty ? "Categories" : "Matching Categories") {
                ForEach(filtered) { cat in
                    NavigationLink(value: CatalogRoute.category(kind, id: cat.id, name: cat.displayName)) {
                        LabeledContent {
                            Text(cat.channelCount, format: .number)
                        } label: {
                            Text(cat.displayName).lineLimit(1)
                            if let prefix = splitPrefix(cat.name).prefix { Text(prefix) }
                        }
                    }
                }
            }
        }
        .listFilter(text: $filter, prompt: "Filter categories")
        .overlay {
            if loaded && categories.isEmpty {
                if app.isRefreshing {
                    ProgressView(app.refreshProgress?.message ?? "Loading channels…")
                } else {
                    ContentUnavailableView(
                        kind == .movie ? "No Movies" : "No Channels", systemImage: kind == .movie ? "film" : "tv",
                        description: Text(app.refreshError ?? "Refresh the account in Settings."))
                }
            }
        }
        .navigationDestination(for: CatalogRoute.self) { route in
            if route.kind == .movie {
                MovieGridView(route: route)
            } else {
                ChannelListView(route: route)
            }
        }
        .task(id: "\(app.catalogVersion)-\(app.favoritesVersion)") { await load() }
    }

    private func load() async {
        guard let id = app.source?.id else { return }
        let kind = kind
        // Providers list plenty of empty categories (off-season events and the like).
        categories = ((try? await app.run { try $0.categories(sourceId: id, kind: kind) }) ?? []).filter { $0.channelCount > 0 }
        favoriteCount = (try? await app.run { try $0.favoriteIds(sourceId: id, kind: kind).count }) ?? 0
        loaded = true
    }
}

/// Loads the channels for a route and the programme guide for them.
@MainActor
@Observable
final class ChannelListModel {
    var channels: [Channel] = []
    var nowNext: [String: NowNext] = [:]
    var loaded = false

    func load(_ route: CatalogRoute, app: AppModel) async {
        guard let id = app.source?.id else { return }
        let result: [Channel]? = try? await app.run { engine in
            switch route {
            case .favorites(let k): try engine.favorites(sourceId: id, kind: k)
            case .recent(let k): try engine.recent(sourceId: id, kind: k, limit: 60)
            case .category(let k, let cat, _): try engine.channels(sourceId: id, kind: k, categoryId: cat)
            }
        }
        channels = result ?? []
        loaded = true
        await loadGuide(app: app)
    }

    func loadGuide(app: AppModel) async {
        guard let id = app.source?.id else { return }
        let ids = Array(Set(channels.compactMap(\.epgId)))
        guard !ids.isEmpty else { return }
        let rows = (try? await app.run { try $0.nowNext(sourceId: id, epgIds: ids) }) ?? []
        nowNext = Dictionary(rows.map { ($0.epgId, $0) }, uniquingKeysWith: { a, _ in a })
    }
}

struct ChannelListView: View {
    let route: CatalogRoute
    @Environment(AppModel.self) private var app
    @State private var model = ChannelListModel()
    @State private var filter = ""

    private var visible: [Channel] {
        let f = filter.trimmingCharacters(in: .whitespaces)
        guard !f.isEmpty else { return model.channels }
        return model.channels.filter { $0.name.localizedCaseInsensitiveContains(f) }
    }

    var body: some View {
        List(visible) { channel in
            ChannelButton(channel: channel, nowNext: channel.epgId.flatMap { model.nowNext[$0] }, list: visible)
        }
        .navigationTitle(route.title)
        .listFilter(text: $filter, prompt: "Filter channels")
        .overlay {
            if model.loaded && model.channels.isEmpty {
                switch route {
                case .favorites:
                    ContentUnavailableView(
                        "No Favorites", systemImage: "star",
                        description: Text(favoriteHint))
                case .recent:
                    ContentUnavailableView("Nothing Watched Yet", systemImage: "clock")
                case .category:
                    ContentUnavailableView("Empty Category", systemImage: "tv")
                }
            }
        }
        .task(id: "\(app.catalogVersion)-\(app.favoritesVersion)") { await model.load(route, app: app) }
        .task {
            // Keep "now playing" current while the list is open.
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                await model.loadGuide(app: app)
            }
        }
    }

    private var favoriteHint: String {
        #if os(tvOS)
        "Press and hold a channel to add it to your favorites."
        #else
        "Swipe left on a channel, or touch and hold it, to add it to your favorites."
        #endif
    }
}

/// A channel row that plays the channel, with favorite actions.
struct ChannelButton: View {
    let channel: Channel
    let nowNext: NowNext?
    let list: [Channel]
    @Environment(AppModel.self) private var app

    var body: some View {
        Button {
            guard let id = app.source?.id else { return }
            app.player.play(channel, in: list, sourceId: id)
        } label: {
            ChannelRow(channel: channel, nowNext: nowNext, isPlaying: app.player.channel?.id == channel.id)
        }
        .contextMenu { FavoriteButton(channel: channel) }
        #if os(iOS)
        .foregroundStyle(.primary)
        .swipeActions(edge: .trailing) {
            FavoriteButton(channel: channel).tint(.yellow)
        }
        #endif
    }
}

struct FavoriteButton: View {
    let channel: Channel
    @Environment(AppModel.self) private var app

    var body: some View {
        Button {
            Task { await app.toggleFavorite(channel) }
        } label: {
            if channel.favorite {
                Label("Remove from Favorites", systemImage: "star.slash")
            } else {
                Label("Add to Favorites", systemImage: "star")
            }
        }
    }
}

struct ChannelRow: View {
    let channel: Channel
    let nowNext: NowNext?
    var isPlaying = false

    #if os(tvOS)
    private let logoSize: CGFloat = 80
    #else
    private let logoSize: CGFloat = 44
    #endif

    var body: some View {
        HStack(spacing: 14) {
            ChannelLogo(channel: channel, size: logoSize)
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 6) {
                    Text(channel.displayName).font(.headline).lineLimit(1)
                    if channel.favorite {
                        Image(systemName: "star.fill").font(.caption).foregroundStyle(.yellow)
                    }
                    if let prefix = channel.prefix {
                        Text(prefix)
                            .font(.caption2.weight(.semibold))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(.quaternary, in: Capsule())
                            .foregroundStyle(.secondary)
                    }
                }
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    programmeInfo(at: context.date)
                }
            }
            Spacer(minLength: 0)
            if isPlaying {
                Image(systemName: "speaker.wave.2.fill").foregroundStyle(.tint)
            }
        }
        .padding(.vertical, 2)
        .contentShape(Rectangle())
    }

    @ViewBuilder
    private func programmeInfo(at now: Date) -> some View {
        if let current = [nowNext?.now, nowNext?.next].compactMap({ $0 }).first(where: { $0.isOn(at: now) }) {
            Text(current.title).font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
            ProgressView(value: current.progress(at: now))
                .progressViewStyle(.linear)
                .frame(maxWidth: 220)
        } else if let next = nowNext?.next ?? nowNext?.now, next.start > Int64(now.timeIntervalSince1970) {
            Text("\(formatTime(next.start)) · \(next.title)").font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
        }
    }
}
