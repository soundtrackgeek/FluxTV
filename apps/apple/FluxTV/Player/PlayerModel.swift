import AVFoundation
import AVKit
import Observation
import SwiftUI

/// Playback state shared by the iOS and tvOS player screens.
///
/// One `AVPlayer` and one `AVPlayerViewController` live for the whole session, so Picture in
/// Picture keeps running when the player screen is dismissed.
@MainActor
@Observable
final class PlayerModel: NSObject {
    let player = AVPlayer()
    @ObservationIgnored private(set) lazy var controller: AVPlayerViewController = makeController()

    private(set) var channel: Channel?
    private(set) var sourceId: Int64 = 0
    /// Channels to zap through (the list the channel was started from).
    private(set) var playlist: [Channel] = []
    private(set) var nowNext: NowNext?
    private(set) var status: PlaybackStatus?
    private(set) var errorMessage: String?
    private(set) var isLoading = false
    var isPresented = false
    private(set) var isPictureInPicture = false

    @ObservationIgnored weak var app: AppModel?
    @ObservationIgnored private var statusTask: Task<Void, Never>?
    @ObservationIgnored private var itemObservation: NSKeyValueObservation?
    @ObservationIgnored private var playToken = 0

    var isLive: Bool { channel?.kind == .live }

    private func makeController() -> AVPlayerViewController {
        let vc = AVPlayerViewController()
        vc.player = player
        vc.delegate = self
        #if os(iOS)
        vc.allowsPictureInPicturePlayback = true
        vc.canStartPictureInPictureAutomaticallyFromInline = true
        vc.entersFullScreenWhenPlaybackBegins = false
        #else
        let strip = UIHostingController(rootView: TVChannelStrip(player: self))
        strip.title = "Channels"
        strip.preferredContentSize = CGSize(width: 0, height: 300)
        vc.customInfoViewControllers = [strip]
        #endif
        return vc
    }

    #if os(tvOS)
    /// Channel actions in the transport bar's menu.
    private func updateTransportBar() {
        guard let channel else { return }
        var items: [UIMenuElement] = []
        if playlist.count > 1 {
            items.append(UIAction(title: "Previous Channel", image: UIImage(systemName: "chevron.up")) { [weak self] _ in
                self?.zap(-1)
            })
            items.append(UIAction(title: "Next Channel", image: UIImage(systemName: "chevron.down")) { [weak self] _ in
                self?.zap(1)
            })
        }
        let favorite = channel.favorite
        items.append(UIAction(
            title: favorite ? "Remove from Favorites" : "Add to Favorites",
            image: UIImage(systemName: favorite ? "star.slash" : "star")
        ) { [weak self] _ in
            guard let self, let app = self.app, let channel = self.channel else { return }
            Task { await app.toggleFavorite(channel) }
        })
        controller.transportBarCustomMenuItems = items
    }
    #endif

    // ----- control -------------------------------------------------------------------------

    func play(_ channel: Channel, in list: [Channel], sourceId: Int64) {
        playlist = list.contains(where: { $0.id == channel.id }) ? list : [channel]
        self.sourceId = sourceId
        isPresented = true
        start(channel)
    }

    /// Next (+1) or previous (-1) channel in the current list.
    func zap(_ delta: Int) {
        guard let channel, playlist.count > 1,
              let index = playlist.firstIndex(where: { $0.id == channel.id })
        else { return }
        let next = playlist[(index + delta + playlist.count) % playlist.count]
        start(next)
    }

    /// Switch to a specific channel from the current list.
    func switchTo(_ channel: Channel) {
        guard channel.id != self.channel?.id else { return }
        start(channel)
    }

    func retry() {
        if let channel { start(channel) }
    }

    /// The player screen was dismissed: stop unless Picture in Picture is showing.
    func screenDismissed() {
        if !isPictureInPicture { stop() }
    }

    func stop() {
        playToken += 1
        statusTask?.cancel()
        statusTask = nil
        itemObservation = nil
        player.pause()
        player.replaceCurrentItem(with: nil)
        channel = nil
        nowNext = nil
        status = nil
        errorMessage = nil
        isLoading = false
        isPresented = false
        if let app { Task { _ = try? await app.run { $0.stopPlayback() } } }
    }

    func favoriteChanged(_ id: String, _ on: Bool) {
        if channel?.id == id { channel?.favorite = on }
        if let i = playlist.firstIndex(where: { $0.id == id }) { playlist[i].favorite = on }
        #if os(tvOS)
        updateTransportBar()
        #endif
    }

    private func start(_ channel: Channel) {
        guard let app else { return }
        playToken += 1
        let token = playToken
        self.channel = channel
        nowNext = nil
        status = nil
        errorMessage = nil
        isLoading = true
        player.pause()
        #if os(tvOS)
        updateTransportBar()
        #endif
        let sourceId = self.sourceId
        Task {
            do {
                let playback = try await app.run {
                    try $0.play(sourceId: sourceId, kind: channel.kind, channelId: channel.id)
                }
                guard token == playToken, let url = URL(string: playback.url) else { return }
                try? AVAudioSession.sharedInstance().setActive(true)
                let item = AVPlayerItem(url: url)
                item.externalMetadata = metadata(for: channel, programme: nil)
                observe(item, token: token)
                player.replaceCurrentItem(with: item)
                player.play()
                startStatusUpdates(token: token)
            } catch {
                guard token == playToken else { return }
                isLoading = false
                errorMessage = message(for: error)
            }
        }
    }

    private func observe(_ item: AVPlayerItem, token: Int) {
        itemObservation = item.observe(\.status, options: [.new]) { [weak self] item, _ in
            let status = item.status
            let error = item.error
            Task { @MainActor [weak self] in
                guard let self, token == self.playToken else { return }
                switch status {
                case .readyToPlay:
                    self.isLoading = false
                case .failed:
                    self.isLoading = false
                    await self.reportFailure(error)
                default:
                    break
                }
            }
        }
    }

    /// Explain a failed item with what the media server saw (provider errors, codecs).
    private func reportFailure(_ error: Error?) async {
        guard let app else { return }
        let status = try? await app.run { $0.playbackStatus() }
        self.status = status
        if let unsupported = status?.unsupported, !unsupported.isEmpty {
            errorMessage = "This channel uses \(unsupported.joined(separator: " and ")), which Apple devices can't play."
        } else if let m = status?.message {
            errorMessage = "The stream could not be played: \(m)."
        } else {
            errorMessage = error.map { $0.localizedDescription } ?? "The stream could not be played."
        }
    }

    private func startStatusUpdates(token: Int) {
        statusTask?.cancel()
        statusTask = Task { [weak self] in
            var tick = 0
            while !Task.isCancelled {
                guard let self, token == self.playToken, let app = self.app, let channel = self.channel else { return }
                if tick % 15 == 0, channel.kind == .live, let epg = channel.epgId {
                    let id = self.sourceId
                    let nn = try? await app.run { try $0.nowNext(sourceId: id, epgIds: [epg]).first }
                    if token == self.playToken, let nn {
                        if nn.now?.title != self.nowNext?.now?.title {
                            self.player.currentItem?.externalMetadata = self.metadata(for: channel, programme: nn.now)
                        }
                        self.nowNext = nn
                    }
                }
                if let s = try? await app.run({ $0.playbackStatus() }), token == self.playToken {
                    self.status = s
                    if s.state == .failed, self.errorMessage == nil {
                        self.errorMessage = "The stream stopped: \(s.message ?? "the provider closed the connection")."
                    }
                }
                tick += 1
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    // ----- metadata ------------------------------------------------------------------------

    private func metadata(for channel: Channel, programme: Programme?) -> [AVMetadataItem] {
        func item(_ id: AVMetadataIdentifier, _ value: String) -> AVMetadataItem {
            let m = AVMutableMetadataItem()
            m.identifier = id
            m.value = value as NSString
            m.extendedLanguageTag = "und"
            return m
        }
        var items: [AVMetadataItem] = []
        if let p = programme {
            items.append(item(.commonIdentifierTitle, p.title))
            items.append(item(.iTunesMetadataTrackSubTitle, channel.displayName))
            if let d = p.description, !d.isEmpty { items.append(item(.commonIdentifierDescription, d)) }
        } else {
            items.append(item(.commonIdentifierTitle, channel.displayName))
        }
        return items
    }
}

extension PlayerModel: AVPlayerViewControllerDelegate {
    nonisolated func playerViewControllerWillStartPictureInPicture(_ playerViewController: AVPlayerViewController) {
        MainActor.assumeIsolated {
            isPictureInPicture = true
            isPresented = false
        }
    }

    nonisolated func playerViewControllerDidStopPictureInPicture(_ playerViewController: AVPlayerViewController) {
        MainActor.assumeIsolated {
            isPictureInPicture = false
            if !isPresented { stop() }
        }
    }

    nonisolated func playerViewController(
        _ playerViewController: AVPlayerViewController,
        restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completionHandler: @escaping (Bool) -> Void
    ) {
        MainActor.assumeIsolated {
            isPresented = true
        }
        completionHandler(true)
    }
}
