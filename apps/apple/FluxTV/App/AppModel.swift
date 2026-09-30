import AVFoundation
import Foundation
import Observation
import SwiftUI

/// User-facing text for errors from the Rust core or the Keychain.
func message(for error: Error) -> String {
    if let e = error as? FluxError {
        switch e {
        case .Network(let m), .Auth(let m), .NotFound(let m), .Invalid(let m), .Data(let m), .Internal(let m):
            return m.prefix(1).uppercased() + m.dropFirst()
        }
    }
    return error.localizedDescription
}

enum AppError: LocalizedError {
    case notReady

    var errorDescription: String? { "FluxTV is still starting." }
}

/// App-wide state: the Rust engine, saved accounts and the catalog refresh cycle.
@MainActor
@Observable
final class AppModel {
    private(set) var engine: FluxEngine?
    private(set) var accounts: [StoredAccount] = []
    private(set) var sources: [SourceInfo] = []
    private(set) var startupError: String?
    private(set) var isStarted = false

    /// Bumped when the catalog or guide changes so views reload.
    private(set) var catalogVersion = 0
    /// Bumped when favorites change.
    private(set) var favoritesVersion = 0

    private(set) var isRefreshing = false
    private(set) var refreshProgress: RefreshProgress?
    private(set) var refreshError: String?

    let player = PlayerModel()

    private var currentAccountKey = UserDefaults.standard.string(forKey: "currentAccountKey") ?? "" {
        didSet { UserDefaults.standard.set(currentAccountKey, forKey: "currentAccountKey") }
    }

    var source: SourceInfo? {
        sources.first { $0.key == currentAccountKey } ?? sources.first
    }

    var currentAccount: StoredAccount? {
        guard let key = source?.key else { return nil }
        return accounts.first { $0.key == key }
    }

    init() {
        player.app = self
    }

    // ----- startup -------------------------------------------------------------------------

    func start() async {
        guard engine == nil else { return }
        do {
            try? AVAudioSession.sharedInstance().setCategory(.playback, mode: .moviePlayback)
            let path = try Self.databaseURL().path
            engine = try await Task.detached(priority: .userInitiated) { try FluxEngine(databasePath: path) }.value
            try await reloadAccounts()
            isStarted = true
            #if DEBUG
            await seedAccountFromEnvironment()
            autoplayFromEnvironment()
            #endif
            await refreshIfNeeded()
        } catch {
            startupError = message(for: error)
            isStarted = true
        }
    }

    #if DEBUG
    /// Development only: on a fresh install, `FLUXTV_SEED_SERVER`, `FLUXTV_SEED_USERNAME` and
    /// `FLUXTV_SEED_PASSWORD` in the launch environment add an Xtream account, so simulators can
    /// be tested without typing credentials (`SIMCTL_CHILD_FLUXTV_SEED_…` with `simctl launch`).
    private func seedAccountFromEnvironment() async {
        let env = ProcessInfo.processInfo.environment
        guard accounts.isEmpty, let server = env["FLUXTV_SEED_SERVER"], let user = env["FLUXTV_SEED_USERNAME"],
              let password = env["FLUXTV_SEED_PASSWORD"]
        else { return }
        do {
            try await saveAccount(AccountInput(
                name: "", kind: .xtream, url: server, username: user, password: password, epgUrl: nil))
        } catch {
            refreshError = message(for: error)
        }
    }

    /// Development only: `FLUXTV_DEBUG_AUTOPLAY=<search>` plays the best live match at launch
    /// (for checking playback on simulators that can't be driven by touch, like Apple TV).
    private func autoplayFromEnvironment() {
        guard let query = ProcessInfo.processInfo.environment["FLUXTV_DEBUG_AUTOPLAY"], let id = source?.id else { return }
        Task {
            let hits = (try? await run { try $0.search(sourceId: id, kind: .live, query: query, limit: 10) }) ?? []
            if let first = hits.first { player.play(first, in: hits, sourceId: id) }
        }
    }
    #endif

    /// Where the catalog database lives. It is a cache: on tvOS only the Caches directory is
    /// writable, and the system may purge it (accounts live in the Keychain, favorites are
    /// backed up to user defaults).
    static func databaseURL() throws -> URL {
        let fm = FileManager.default
        #if os(tvOS)
        let base = try fm.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        #else
        let base = try fm.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        #endif
        var dir = base.appendingPathComponent("FluxTV", isDirectory: true)
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? dir.setResourceValues(values)
        return dir.appendingPathComponent("fluxtv.db")
    }

    /// Run an engine call off the main thread.
    nonisolated func run<T: Sendable>(_ body: @escaping @Sendable (FluxEngine) throws -> T) async throws -> T {
        guard let engine = await engine else { throw AppError.notReady }
        return try await Task.detached(priority: .userInitiated) { try body(engine) }.value
    }

    // ----- accounts ------------------------------------------------------------------------

    /// Load accounts from the Keychain (including ones synced from other devices) and make
    /// the database match.
    func reloadAccounts() async throws {
        let stored = try AccountStore.loadAll()
        let saved = stored.map(\.saved)
        let synced = try await run { try $0.syncAccounts(accounts: saved) }
        let changed = synced.map(\.id) != sources.map(\.id)
        accounts = stored
        sources = synced
        if source?.key != currentAccountKey, let first = sources.first {
            currentAccountKey = first.key
        }
        if changed {
            await restoreFavorites()
            catalogVersion += 1
        }
    }

    /// Validate with the provider, then save to the Keychain and select it.
    func saveAccount(_ input: AccountInput, replacing key: String? = nil) async throws {
        let engine = try await run { $0 }
        let validated = try await engine.validateAccount(input: input)
        let existing = key.flatMap { k in accounts.first { $0.key == k } }
        var account = StoredAccount(validated: validated, createdAt: existing?.createdAt ?? .now)
        if let existing { account.key = existing.key }
        try AccountStore.save(account)
        try await reloadAccounts()
        select(account.key)
        // The first refresh can take a minute or two; the banner shows its progress.
        Task { await refreshIfNeeded() }
    }

    func deleteAccount(_ key: String) async {
        do {
            try AccountStore.delete(key: key)
            FavoritesBackup.remove(accountKey: key)
            if player.sourceId == sources.first(where: { $0.key == key })?.id { player.stop() }
            try await reloadAccounts()
            catalogVersion += 1
        } catch {
            refreshError = message(for: error)
        }
    }

    func select(_ key: String) {
        guard key != currentAccountKey || source?.key != key else { return }
        player.stop()
        currentAccountKey = key
        catalogVersion += 1
        Task { await refreshIfNeeded() }
    }

    func accountInfo() async -> Result<AccountInfo?, Error> {
        guard let id = source?.id, let engine else { return .success(nil) }
        do { return .success(try await engine.accountInfo(sourceId: id)) } catch { return .failure(error) }
    }

    // ----- refresh -------------------------------------------------------------------------

    func refreshIfNeeded() async {
        guard let id = source?.id,
              let stale = try? await run({ try $0.staleness(sourceId: id) }),
              stale.catalog || stale.guide
        else { return }
        await refresh(force: false)
    }

    func refresh(force: Bool) async {
        guard let id = source?.id, let engine, !isRefreshing else { return }
        isRefreshing = true
        refreshError = nil
        let stale = try? await run { try $0.staleness(sourceId: id) }
        let stage = force || stale?.catalog != false ? "catalog" : "epg"
        refreshProgress = RefreshProgress(stage: stage, message: "Starting", fraction: nil, done: false)
        let poller = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(400))
                guard let self, let p = try? await self.run({ $0.refreshProgress(sourceId: id) }) else { continue }
                // Channels are usable while the (slower) guide import runs.
                if p.stage == "epg", self.refreshProgress?.stage != "epg" { self.catalogVersion += 1 }
                self.refreshProgress = p
            }
        }
        do {
            let outcome = try await engine.refresh(sourceId: id, force: force)
            refreshError = outcome.guideError.map { "The TV guide could not be updated: \($0)" }
            if outcome.catalogRefreshed { await restoreFavorites() }
        } catch {
            refreshError = message(for: error)
        }
        poller.cancel()
        refreshProgress = nil
        isRefreshing = false
        if let s = try? await run({ try $0.source(sourceId: id) }), let i = sources.firstIndex(where: { $0.id == id }) {
            sources[i] = s
        }
        catalogVersion += 1
    }

    // ----- favorites -----------------------------------------------------------------------

    @discardableResult
    func toggleFavorite(_ channel: Channel) async -> Bool {
        guard let source else { return channel.favorite }
        let id = source.id
        let key = source.key
        do {
            let on = try await run { try $0.toggleFavorite(sourceId: id, kind: channel.kind, channelId: channel.id) }
            let ids = try await run { try $0.favoriteIds(sourceId: id, kind: channel.kind) }
            FavoritesBackup.save(ids, accountKey: key, kind: channel.kind)
            favoritesVersion += 1
            player.favoriteChanged(channel.id, on)
            return on
        } catch {
            return channel.favorite
        }
    }

    /// Re-apply backed-up favorites (after a purged or recreated database).
    private func restoreFavorites() async {
        for s in sources {
            for kind in [ContentKind.live, .movie] {
                let ids = FavoritesBackup.load(accountKey: s.key, kind: kind)
                guard !ids.isEmpty else { continue }
                let id = s.id
                _ = try? await run { try $0.restoreFavorites(sourceId: id, kind: kind, channelIds: ids) }
            }
        }
    }

    func clearHistory() async {
        guard let id = source?.id else { return }
        _ = try? await run { try $0.clearHistory(sourceId: id) }
        catalogVersion += 1
    }
}

/// Favorite ids per account in user defaults, so they survive a purged cache database.
enum FavoritesBackup {
    private static func key(_ account: String, _ kind: ContentKind) -> String {
        "favorites.\(account).\(kind == .movie ? "movie" : "live")"
    }

    static func save(_ ids: [String], accountKey: String, kind: ContentKind) {
        UserDefaults.standard.set(ids, forKey: key(accountKey, kind))
    }

    static func load(accountKey: String, kind: ContentKind) -> [String] {
        UserDefaults.standard.stringArray(forKey: key(accountKey, kind)) ?? []
    }

    static func remove(accountKey: String) {
        for kind in [ContentKind.live, .movie] {
            UserDefaults.standard.removeObject(forKey: key(accountKey, kind))
        }
    }
}
