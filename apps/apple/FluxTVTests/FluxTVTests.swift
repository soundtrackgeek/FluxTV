import XCTest

@testable import FluxTV

final class FormattingTests: XCTestCase {
    func testSplitPrefix() {
        XCTAssertEqual(splitPrefix("NOR| NRK1 HD").prefix, "NOR")
        XCTAssertEqual(splitPrefix("NOR| NRK1 HD").rest, "NRK1 HD")
        XCTAssertEqual(splitPrefix("UK: BBC One").rest, "BBC One")
        XCTAssertNil(splitPrefix("20:00 Match").prefix)
        XCTAssertNil(splitPrefix("Plain Name").prefix)
    }

    func testInitials() {
        XCTAssertEqual(initials("NOR| NRK1 HD"), "NH")
        XCTAssertEqual(initials("GOLD| TV 2"), "T2")
    }
}

final class EngineTests: XCTestCase {
    private func tempDatabase() -> String {
        FileManager.default.temporaryDirectory.appendingPathComponent("fluxtv-test-\(UUID().uuidString).db").path
    }

    func testAccountsSyncWithoutStoringPasswords() throws {
        let engine = try FluxEngine(databasePath: tempDatabase())
        XCTAssertTrue(engine.version().hasPrefix("FluxTV/"))
        let account = SavedAccount(
            key: "k1", name: "Test", kind: .xtream, url: "http://provider.invalid", username: "user",
            password: "secret", epgUrl: nil)
        let sources = try engine.syncAccounts(accounts: [account])
        XCTAssertEqual(sources.count, 1)
        XCTAssertEqual(sources[0].key, "k1")
        XCTAssertTrue(try engine.staleness(sourceId: sources[0].id).catalog)
        XCTAssertEqual(try engine.categories(sourceId: sources[0].id, kind: .live), [])

        // Same key again keeps the source; an empty list removes it.
        XCTAssertEqual(try engine.syncAccounts(accounts: [account]).first?.id, sources[0].id)
        XCTAssertEqual(try engine.syncAccounts(accounts: []), [])
    }

    func testFavoritesRestore() throws {
        let engine = try FluxEngine(databasePath: tempDatabase())
        let id = try engine.syncAccounts(accounts: [
            SavedAccount(key: "k", name: "T", kind: .m3u, url: "http://x.invalid/list.m3u", username: nil, password: nil, epgUrl: nil)
        ])[0].id
        try engine.restoreFavorites(sourceId: id, kind: .live, channelIds: ["a", "b"])
        XCTAssertEqual(try engine.favoriteIds(sourceId: id, kind: .live), ["a", "b"])
    }

    func testPlaybackStatusIsIdleWithoutStream() throws {
        let engine = try FluxEngine(databasePath: tempDatabase())
        XCTAssertEqual(engine.playbackStatus().state, .idle)
    }
}

final class AccountStoreTests: XCTestCase {
    func testKeychainRoundTrip() throws {
        let validated = ValidatedAccount(
            name: "Keychain Test", kind: .xtream, url: "http://provider.invalid", username: "u", password: "p",
            epgUrl: nil, info: nil)
        let account = StoredAccount(validated: validated, key: "test-\(UUID().uuidString)")
        try AccountStore.save(account)
        defer { try? AccountStore.delete(key: account.key) }
        let loaded = try AccountStore.loadAll().first { $0.key == account.key }
        XCTAssertEqual(loaded?.password, "p")
        XCTAssertEqual(loaded?.name, "Keychain Test")

        var changed = account
        changed.password = "p2"
        try AccountStore.save(changed)
        XCTAssertEqual(try AccountStore.loadAll().first { $0.key == account.key }?.password, "p2")

        try AccountStore.delete(key: account.key)
        XCTAssertNil(try AccountStore.loadAll().first { $0.key == account.key })
    }
}

final class GuideTimelineTests: XCTestCase {
    func testViewportQueriesHaveBoundedOverscan() {
        let timeline = GuideTimeline(around: 40_000)
        let range = timeline.visibleRange(offset: 151, width: 900, pointsPerMinute: 5)
        XCTAssertLessThanOrEqual(range.count, 6 * 3600)
        XCTAssertEqual(range, timeline.visibleRange(offset: 152, width: 900, pointsPerMinute: 5))
        let later = timeline.visibleRange(offset: 1950, width: 900, pointsPerMinute: 5)
        XCTAssertGreaterThan(later.lowerBound, range.lowerBound)
    }

    func testExtendingEarlierPreservesVisibleTimes() {
        var timeline = GuideTimeline(around: 40_000)
        let before = timeline.visibleRange(offset: 100, width: 700, pointsPerMinute: 4.5)
        timeline.from -= GuideTimeline.extensionSeconds
        let after = timeline.visibleRange(offset: 100 + CGFloat(GuideTimeline.extensionSeconds) / 60 * 4.5,
            width: 700, pointsPerMinute: 4.5)
        XCTAssertEqual(before, after)
    }
}
