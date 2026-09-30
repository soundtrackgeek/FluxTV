import SwiftUI

/// Movies in a category (or favorites / recently watched) as a poster grid.
struct MovieGridView: View {
    let route: CatalogRoute
    @Environment(AppModel.self) private var app
    @State private var model = ChannelListModel()
    @State private var filter = ""

    #if os(tvOS)
    private let columns = [GridItem(.adaptive(minimum: 250, maximum: 300), spacing: 48)]
    #else
    private let columns = [GridItem(.adaptive(minimum: 110, maximum: 160), spacing: 14)]
    #endif

    private var visible: [Channel] {
        let f = filter.trimmingCharacters(in: .whitespaces)
        guard !f.isEmpty else { return model.channels }
        return model.channels.filter { $0.name.localizedCaseInsensitiveContains(f) }
    }

    var body: some View {
        ScrollView {
            LazyVGrid(columns: columns, spacing: 24) {
                ForEach(visible) { movie in
                    MovieCard(movie: movie, list: visible)
                }
            }
            .padding()
        }
        .navigationTitle(route.title)
        .listFilter(text: $filter, prompt: "Filter movies")
        .overlay {
            if model.loaded && model.channels.isEmpty {
                ContentUnavailableView(route.title, systemImage: "film", description: Text("No movies here yet."))
            }
        }
        .task(id: "\(app.catalogVersion)-\(app.favoritesVersion)") { await model.load(route, app: app) }
    }
}

struct MovieCard: View {
    let movie: Channel
    let list: [Channel]
    @Environment(AppModel.self) private var app

    var body: some View {
        Button {
            guard let id = app.source?.id else { return }
            app.player.play(movie, in: list, sourceId: id)
        } label: {
            VStack(alignment: .leading, spacing: 6) {
                poster
                #if os(iOS)
                Text(movie.displayName).font(.caption).lineLimit(2).foregroundStyle(.primary)
                #endif
            }
        }
        #if os(tvOS)
        .buttonStyle(.borderless)
        #else
        .buttonStyle(.plain)
        #endif
        .contextMenu { FavoriteButton(channel: movie) }
    }

    /// Fixed 2:3 frame; artwork of any shape is cropped to it.
    private var poster: some View {
        Color.clear
            .aspectRatio(2 / 3, contentMode: .fit)
            .overlay {
                RemoteImage(url: movie.logo, maxPixel: 600, contentMode: .fill) {
                    ZStack {
                        LinearGradient(
                            colors: [
                                Color(hue: Double(abs(movie.name.hashValueStable) % 360) / 360, saturation: 0.4, brightness: 0.45),
                                .black.opacity(0.8),
                            ],
                            startPoint: .top, endPoint: .bottom)
                        Text(movie.displayName)
                            .font(.headline)
                            .multilineTextAlignment(.center)
                            .foregroundStyle(.white)
                            .padding(8)
                    }
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .overlay(alignment: .topTrailing) {
                if movie.favorite {
                    Image(systemName: "star.fill")
                        .font(.caption)
                        .padding(6)
                        .background(.black.opacity(0.5), in: Circle())
                        .foregroundStyle(.yellow)
                        .padding(6)
                }
            }
    }
}
