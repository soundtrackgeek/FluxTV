import SwiftUI

@main
struct FluxTVApp: App {
    @State private var app = AppModel()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(app)
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var app
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        @Bindable var player = app.player
        Group {
            if !app.isStarted {
                ProgressView("Starting…")
            } else if let error = app.startupError {
                ContentUnavailableView("FluxTV Couldn't Start", systemImage: "exclamationmark.triangle", description: Text(error))
            } else if app.sources.isEmpty {
                WelcomeView()
            } else {
                MainTabs()
            }
        }
        .fullScreenCover(isPresented: $player.isPresented, onDismiss: { app.player.screenDismissed() }) {
            PlayerScreen()
                .environment(app)
        }
        .task { await app.start() }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active, app.isStarted else { return }
            Task {
                // Pick up accounts added on another device through iCloud Keychain.
                try? await app.reloadAccounts()
                await app.refreshIfNeeded()
            }
        }
    }
}

enum AppTab: String, Hashable {
    case live, movies, guide, search, settings
}

struct MainTabs: View {
    @Environment(AppModel.self) private var app
    @SceneStorage("selectedTab") private var tab: AppTab = .live

    var body: some View {
        TabView(selection: $tab) {
            Tab("Live TV", systemImage: "tv", value: AppTab.live) {
                NavigationStack { CategoryListView(kind: .live).navigationTitle("Live TV") }
            }
            Tab("Movies", systemImage: "film", value: AppTab.movies) {
                NavigationStack { CategoryListView(kind: .movie).navigationTitle("Movies") }
            }
            Tab("Guide", systemImage: "calendar", value: AppTab.guide) {
                GuideView()
            }
            Tab(value: AppTab.search, role: .search) {
                SearchView()
            }
            Tab("Settings", systemImage: "gearshape", value: AppTab.settings) {
                SettingsView()
            }
        }
        #if os(iOS)
        .tabViewStyle(.sidebarAdaptable)
        #endif
        .overlay(alignment: .bottom) {
            if app.isRefreshing, tab != .settings {
                RefreshBanner()
                    .padding(.bottom, bannerInset)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.default, value: app.isRefreshing)
        #if DEBUG
        .task {
            if let value = ProcessInfo.processInfo.environment["FLUXTV_DEBUG_TAB"], let initialTab = AppTab(rawValue: value) {
                tab = initialTab
            }
        }
        #endif
    }

    private var bannerInset: CGFloat {
        #if os(tvOS)
        40
        #else
        60
        #endif
    }
}

private struct RefreshBanner: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        HStack(spacing: 10) {
            ProgressView()
            VStack(alignment: .leading, spacing: 2) {
                Text(app.refreshProgress?.stage == "epg" ? "Updating TV guide" : "Updating channels")
                    .font(.caption.bold())
                Text(app.refreshProgress?.message ?? "")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: Capsule())
        .allowsHitTesting(false)
    }
}
