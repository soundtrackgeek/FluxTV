import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var app
    @State private var info: AccountInfo?
    @State private var infoError: String?
    @State private var confirmDelete: StoredAccount?

    var body: some View {
        NavigationStack {
            Form {
                Section("Accounts") {
                    ForEach(app.accounts) { account in
                        HStack {
                            Button {
                                app.select(account.key)
                            } label: {
                                VStack(alignment: .leading) {
                                    Text(account.name)
                                    Text(account.sourceKind == .xtream ? "Xtream · \(account.username ?? "")" : "M3U playlist")
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            Spacer()
                            if account.key == app.source?.key {
                                Image(systemName: "checkmark").foregroundStyle(.tint)
                            }
                        }
                        .contextMenu {
                            NavigationLink("Edit") { AccountFormView(editing: account) }
                            Button("Delete", role: .destructive) { confirmDelete = account }
                        }
                        #if os(iOS)
                        .swipeActions {
                            Button("Delete", role: .destructive) { confirmDelete = account }
                        }
                        #endif
                    }
                    NavigationLink {
                        AccountFormView()
                    } label: {
                        Label("Add Account", systemImage: "plus")
                    }
                    if let current = app.currentAccount {
                        NavigationLink {
                            AccountFormView(editing: current)
                        } label: {
                            Label("Edit \(current.name)", systemImage: "pencil")
                        }
                    }
                }

                if let source = app.source {
                    Section("Subscription") {
                        if let info {
                            LabeledContent("Status", value: info.status.isEmpty ? "Unknown" : info.status)
                            if let exp = info.expiresAt {
                                LabeledContent("Expires", value: Date(timeIntervalSince1970: TimeInterval(exp)).formatted(date: .abbreviated, time: .omitted))
                            }
                            LabeledContent("Connections", value: "\(info.activeConnections) of \(info.maxConnections) in use")
                            if info.isTrial { LabeledContent("Trial", value: "Yes") }
                            if let m = info.message, !m.isEmpty { Text(m).font(.footnote).foregroundStyle(.secondary) }
                        } else if let infoError {
                            Label(infoError, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary)
                        } else if source.kind == .m3u {
                            Text("M3U playlists don't report subscription details.").foregroundStyle(.secondary)
                        } else {
                            ProgressView()
                        }
                    }

                    Section {
                        LabeledContent("Live channels", value: source.liveCount.formatted())
                        LabeledContent("Movies", value: source.movieCount.formatted())
                        LabeledContent("Channels updated", value: formatAgo(source.catalogUpdatedAt))
                        LabeledContent("Guide updated", value: formatAgo(source.guideUpdatedAt))
                        Button {
                            Task { await app.refresh(force: true) }
                        } label: {
                            if app.isRefreshing {
                                HStack {
                                    ProgressView()
                                    Text(app.refreshProgress?.message ?? "Refreshing…").padding(.leading, 6)
                                }
                            } else {
                                Label("Refresh Channels and Guide", systemImage: "arrow.clockwise")
                            }
                        }
                        .disabled(app.isRefreshing)
                        if let e = app.refreshError {
                            Text(e).font(.footnote).foregroundStyle(.red)
                        }
                    } header: {
                        Text("Library")
                    } footer: {
                        Text("Channels refresh automatically once a day and the guide every 12 hours.")
                    }

                    Section {
                        Button("Clear Watch History", role: .destructive) { Task { await app.clearHistory() } }
                    }
                }

                Section("About") {
                    LabeledContent("Version", value: appVersion)
                    if let core = app.engine?.version() { LabeledContent("Core", value: core) }
                }
            }
            .navigationTitle("Settings")
            .task(id: "\(app.source?.id ?? 0)") { await loadInfo() }
            .confirmationDialog(
                "Delete \(confirmDelete?.name ?? "account")?", isPresented: .init(
                    get: { confirmDelete != nil }, set: { if !$0 { confirmDelete = nil } }),
                titleVisibility: .visible
            ) {
                Button("Delete", role: .destructive) {
                    if let key = confirmDelete?.key { Task { await app.deleteAccount(key) } }
                }
            } message: {
                Text("This removes the account from this device and from iCloud Keychain, along with its favorites and history.")
            }
        }
    }

    private var appVersion: String {
        let v = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "?"
        let b = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "?"
        return "\(v) (\(b))"
    }

    private func loadInfo() async {
        info = nil
        infoError = nil
        switch await app.accountInfo() {
        case .success(let i): info = i
        case .failure(let e): infoError = message(for: e)
        }
    }
}
