import AVKit
import SwiftUI

/// Hosts the shared `AVPlayerViewController`.
struct PlayerControllerView: UIViewControllerRepresentable {
    let controller: AVPlayerViewController

    func makeUIViewController(context: Context) -> AVPlayerViewController { controller }
    func updateUIViewController(_ controller: AVPlayerViewController, context: Context) {}
}

/// Loading spinner and error message on top of the video.
struct PlayerStatusOverlay: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        let player = app.player
        ZStack {
            if let error = player.errorMessage {
                VStack(spacing: 14) {
                    Image(systemName: "exclamationmark.triangle.fill").font(.largeTitle).foregroundStyle(.yellow)
                    Text(error).multilineTextAlignment(.center)
                    HStack {
                        Button("Try Again") { player.retry() }
                            .buttonStyle(.borderedProminent)
                        if player.playlist.count > 1 {
                            Button("Next Channel") { player.zap(1) }
                                .buttonStyle(.bordered)
                        }
                    }
                }
                .padding(24)
                .frame(maxWidth: 560)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 18))
                .padding()
            } else if player.isLoading {
                VStack(spacing: 12) {
                    ProgressView().controlSize(.large).tint(.white)
                    if let channel = player.channel {
                        Text(channel.displayName).font(.headline).foregroundStyle(.white)
                    }
                    if player.status?.state == .reconnecting {
                        Text("Reconnecting…").font(.caption).foregroundStyle(.white.opacity(0.7))
                    }
                }
                .padding(20)
                .background(.black.opacity(0.45), in: RoundedRectangle(cornerRadius: 16))
                .allowsHitTesting(false)
            }
        }
    }
}

#if os(iOS)

struct PlayerScreen: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        GeometryReader { geo in
            let landscape = geo.size.width > geo.size.height
            VStack(spacing: 0) {
                PlayerControllerView(controller: app.player.controller)
                    .frame(height: landscape ? geo.size.height : geo.size.width * 9 / 16)
                    .overlay { PlayerStatusOverlay() }
                    .overlay(alignment: .topLeading) {
                        if landscape { CloseButton().padding(.leading, 8).padding(.top, 4) }
                    }
                if !landscape {
                    PlayerInfoPanel()
                        .frame(maxHeight: .infinity)
                        .background(Color(.systemBackground))
                }
            }
            .ignoresSafeArea(edges: landscape ? .all : [])
        }
        .background(Color.black.ignoresSafeArea())
        .statusBarHidden(false)
    }
}

private struct CloseButton: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        Button {
            app.player.isPresented = false
        } label: {
            Image(systemName: "chevron.down")
                .font(.headline)
                .padding(10)
                .background(.black.opacity(0.45), in: Circle())
                .foregroundStyle(.white)
        }
        .accessibilityLabel("Close player")
    }
}

/// Channel, programme and switching controls below the video (portrait).
private struct PlayerInfoPanel: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        let player = app.player
        VStack(alignment: .leading, spacing: 0) {
            if let channel = player.channel {
                VStack(alignment: .leading, spacing: 14) {
                    HStack(spacing: 12) {
                        ChannelLogo(channel: channel, size: 48)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(channel.displayName).font(.title3.bold()).lineLimit(1)
                            if let s = player.status, s.state != .idle, player.isLive {
                                Text(streamSummary(s)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                        }
                        Spacer()
                        Button {
                            Task { await app.toggleFavorite(channel) }
                        } label: {
                            Image(systemName: channel.favorite ? "star.fill" : "star")
                                .font(.title2)
                                .foregroundStyle(channel.favorite ? .yellow : .secondary)
                        }
                        .accessibilityLabel(channel.favorite ? "Remove from favorites" : "Add to favorites")
                        Button {
                            player.isPresented = false
                        } label: {
                            Image(systemName: "chevron.down.circle.fill").font(.title2).foregroundStyle(.secondary)
                        }
                        .accessibilityLabel("Close player")
                    }

                    if player.isLive { ProgrammeSummary(nowNext: player.nowNext) }

                    if player.playlist.count > 1 {
                        HStack(spacing: 12) {
                            Button { player.zap(-1) } label: {
                                Label("Previous", systemImage: "chevron.up").frame(maxWidth: .infinity)
                            }
                            Button { player.zap(1) } label: {
                                Label("Next", systemImage: "chevron.down").frame(maxWidth: .infinity)
                            }
                        }
                        .buttonStyle(.bordered)
                    }
                }
                .padding()

                if player.playlist.count > 1 {
                    Divider()
                    ChannelSwitcher()
                }
            }
            Spacer(minLength: 0)
        }
    }

    private func streamSummary(_ s: PlaybackStatus) -> String {
        var parts: [String] = []
        if let v = s.videoCodec { parts.append(v) }
        if let a = s.audioCodecs.first { parts.append(a) }
        if s.bitrate > 0 { parts.append(formatBitrate(s.bitrate)) }
        if s.state == .reconnecting { parts.append("reconnecting") }
        return parts.joined(separator: " · ")
    }
}

private struct ProgrammeSummary: View {
    let nowNext: NowNext?

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            VStack(alignment: .leading, spacing: 6) {
                if let now = nowNext?.now, now.isOn(at: context.date) {
                    HStack {
                        Text("NOW").font(.caption2.bold()).foregroundStyle(.tint)
                        Text(now.timeRange).font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        Text("\(now.minutesLeft(at: context.date)) min left").font(.caption).foregroundStyle(.secondary)
                    }
                    Text(now.title).font(.headline)
                    ProgressView(value: now.progress(at: context.date))
                    if let d = now.description, !d.isEmpty {
                        Text(d).font(.subheadline).foregroundStyle(.secondary).lineLimit(3)
                    }
                }
                if let next = nowNext?.next {
                    Text("Next at \(formatTime(next.start)): \(next.title)")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                if nowNext?.now == nil && nowNext?.next == nil {
                    Text("No guide information").font(.subheadline).foregroundStyle(.tertiary)
                }
            }
        }
    }
}

/// The list the channel was started from, for quick switching.
private struct ChannelSwitcher: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        let player = app.player
        ScrollViewReader { proxy in
            List(player.playlist) { ch in
                Button {
                    player.switchTo(ch)
                } label: {
                    HStack(spacing: 12) {
                        ChannelLogo(channel: ch, size: 34)
                        Text(ch.displayName).lineLimit(1)
                        Spacer()
                        if ch.id == player.channel?.id {
                            Image(systemName: "speaker.wave.2.fill").foregroundStyle(.tint)
                        }
                    }
                }
                .foregroundStyle(.primary)
                .id(ch.id)
            }
            .listStyle(.plain)
            .onAppear { if let id = player.channel?.id { proxy.scrollTo(id, anchor: .center) } }
        }
    }
}

#else

/// tvOS: the system player full screen. Channel switching lives in the transport bar menu and
/// the "Channels" tab of the info panel (swipe down).
struct PlayerScreen: View {
    @Environment(AppModel.self) private var app

    var body: some View {
        PlayerControllerView(controller: app.player.controller)
            .ignoresSafeArea()
            .overlay { PlayerStatusOverlay() }
    }
}

/// Horizontal channel strip shown in the player's info panel.
struct TVChannelStrip: View {
    let player: PlayerModel

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal) {
                LazyHStack(spacing: 40) {
                    ForEach(player.playlist) { ch in
                        Button {
                            player.switchTo(ch)
                        } label: {
                            VStack(spacing: 10) {
                                ChannelLogo(channel: ch, size: 120)
                                Text(ch.displayName).font(.caption).lineLimit(1).frame(width: 200)
                            }
                            .padding(12)
                        }
                        .buttonStyle(.card)
                        .id(ch.id)
                    }
                }
                .padding(.horizontal, 60)
                .padding(.vertical, 30)
            }
            .onAppear { if let id = player.channel?.id { proxy.scrollTo(id, anchor: .center) } }
        }
    }
}

#endif
