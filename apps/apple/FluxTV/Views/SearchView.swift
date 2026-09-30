import SwiftUI

/// Ranked search across all live channels or movies of the current account.
struct SearchView: View {
    @Environment(AppModel.self) private var app
    @State private var query = ""
    @State private var kind: ContentKind = .live
    @State private var results: [Channel] = []
    @State private var nowNext: [String: NowNext] = [:]
    @State private var searched = false

    var body: some View {
        NavigationStack {
            List {
                Picker("Search in", selection: $kind) {
                    Text("Live TV").tag(ContentKind.live)
                    Text("Movies").tag(ContentKind.movie)
                }
                .pickerStyle(.segmented)
                #if os(iOS)
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
                #endif

                ForEach(results) { channel in
                    ChannelButton(channel: channel, nowNext: channel.epgId.flatMap { nowNext[$0] }, list: results)
                }
            }
            .navigationTitle("Search")
            .searchable(text: $query, prompt: kind == .live ? "Channels, e.g. nrk1 hd" : "Movie title")
            .autocorrectionDisabled()
            .overlay {
                if query.isEmpty {
                    ContentUnavailableView(
                        "Search", systemImage: "magnifyingglass",
                        description: Text("Quality tags and accents are ignored: “nrk1 raw” finds “GOLD| NRK1 ᴿᴬᵂ”."))
                } else if searched && results.isEmpty {
                    ContentUnavailableView.search(text: query)
                }
            }
            .task(id: "\(query)|\(kind == .live)|\(app.catalogVersion)|\(app.favoritesVersion)") { await search() }
        }
    }

    private func search() async {
        let q = query.trimmingCharacters(in: .whitespaces)
        guard !q.isEmpty, let id = app.source?.id else {
            results = []
            searched = false
            return
        }
        try? await Task.sleep(for: .milliseconds(220))  // debounce typing
        guard !Task.isCancelled else { return }
        let kind = kind
        let found = (try? await app.run { try $0.search(sourceId: id, kind: kind, query: q, limit: 200) }) ?? []
        guard !Task.isCancelled else { return }
        results = found
        searched = true
        let ids = Array(Set(found.compactMap(\.epgId)))
        if kind == .live, !ids.isEmpty {
            let rows = (try? await app.run { try $0.nowNext(sourceId: id, epgIds: ids) }) ?? []
            nowNext = Dictionary(rows.map { ($0.epgId, $0) }, uniquingKeysWith: { a, _ in a })
        }
    }
}
