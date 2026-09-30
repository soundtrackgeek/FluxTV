import SwiftUI

/// Add or edit an account. Details are checked with the provider before they are saved to
/// the Keychain.
struct AccountFormView: View {
    /// Existing account to edit, or nil to add one.
    var editing: StoredAccount?
    var onDone: () -> Void = {}

    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss
    @State private var kind: SourceKind = .xtream
    @State private var name = ""
    @State private var server = ""
    @State private var username = ""
    @State private var password = ""
    @State private var playlistURL = ""
    @State private var guideURL = ""
    @State private var saving = false
    @State private var error: String?

    private var canSave: Bool {
        switch kind {
        case .xtream:
            !server.trimmed.isEmpty && !username.trimmed.isEmpty && (!password.isEmpty || editing != nil)
        case .m3u:
            !playlistURL.trimmed.isEmpty
        }
    }

    var body: some View {
        Form {
            Section {
                Picker("Type", selection: $kind) {
                    Text("Xtream Codes").tag(SourceKind.xtream)
                    Text("M3U Playlist").tag(SourceKind.m3u)
                }
                #if os(iOS)
                .pickerStyle(.segmented)
                #endif
            } footer: {
                Text(kind == .xtream
                    ? "Server address, username and password from your provider. You can also paste a get.php link as an M3U playlist."
                    : "A playlist URL (M3U or M3U8). Xtream get.php links are upgraded automatically.")
            }

            Section("Account") {
                TextField("Name (optional)", text: $name)
                if kind == .xtream {
                    TextField("Server, e.g. http://provider.example:8080", text: $server)
                        .keyboardType(.URL)
                        .textContentType(.URL)
                    TextField("Username", text: $username)
                        .textContentType(.username)
                    SecureField(editing == nil ? "Password" : "Password (unchanged)", text: $password)
                        .textContentType(.password)
                } else {
                    TextField("Playlist URL", text: $playlistURL)
                        .keyboardType(.URL)
                    TextField("TV guide (XMLTV) URL, optional", text: $guideURL)
                        .keyboardType(.URL)
                }
            }
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()

            if let error {
                Section {
                    Label(error, systemImage: "exclamationmark.triangle.fill").foregroundStyle(.red)
                }
            }

            Section {
                Button {
                    Task { await save() }
                } label: {
                    HStack {
                        Text(editing == nil ? "Add Account" : "Save Changes")
                        if saving {
                            Spacer()
                            ProgressView()
                        }
                    }
                }
                .disabled(!canSave || saving)
            } footer: {
                Text("Accounts are stored in your Keychain. With iCloud Keychain they also appear on your other devices, such as Apple TV.")
            }
        }
        .navigationTitle(editing == nil ? "Add Account" : "Edit Account")
        .onAppear(perform: fill)
    }

    private func fill() {
        guard let a = editing else { return }
        kind = a.sourceKind
        name = a.name
        if kind == .xtream {
            server = a.url
            username = a.username ?? ""
        } else {
            playlistURL = a.url
            guideURL = a.epgUrl ?? ""
        }
    }

    private func save() async {
        saving = true
        error = nil
        let input: AccountInput
        switch kind {
        case .xtream:
            input = AccountInput(
                name: name.trimmed, kind: .xtream, url: server.trimmed, username: username.trimmed,
                password: password.isEmpty ? editing?.password : password, epgUrl: editing?.epgUrl)
        case .m3u:
            input = AccountInput(
                name: name.trimmed, kind: .m3u, url: playlistURL.trimmed, username: nil, password: nil,
                epgUrl: guideURL.trimmed.isEmpty ? nil : guideURL.trimmed)
        }
        do {
            try await app.saveAccount(input, replacing: editing?.key)
            saving = false
            onDone()
            dismiss()
        } catch {
            saving = false
            self.error = message(for: error)
        }
    }
}

extension String {
    var trimmed: String { trimmingCharacters(in: .whitespacesAndNewlines) }
}

/// First launch: no accounts yet.
struct WelcomeView: View {
    @State private var adding = false
    @Environment(AppModel.self) private var app

    var body: some View {
        NavigationStack {
            VStack(spacing: 28) {
                Spacer()
                Image(systemName: "play.tv.fill")
                    .font(.system(size: 72))
                    .foregroundStyle(.tint)
                Text("Welcome to FluxTV").font(.largeTitle.bold())
                Text("Add your IPTV provider to watch live TV, movies and the TV guide.")
                    .multilineTextAlignment(.center)
                    .foregroundStyle(.secondary)
                NavigationLink("Add Account") { AccountFormView() }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                #if os(tvOS)
                Text("Added an account on your iPhone? With iCloud Keychain it shows up here automatically.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Button("Check Again") { Task { try? await app.reloadAccounts() } }
                #endif
                Spacer()
            }
            .padding(40)
            .frame(maxWidth: 700)
        }
    }
}
