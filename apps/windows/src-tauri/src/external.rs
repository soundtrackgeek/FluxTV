//! Launching an external player (mpv or VLC) for streams the built-in player can't decode
//! (e.g. AC-3/E-AC-3 audio, which WebView2's Media Source Extensions don't support).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalPlayer {
    pub name: String,
    pub path: String,
}

fn which(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(';')
        .map(|dir| Path::new(dir.trim_matches('"')).join(exe))
        .find(|p| p.is_file())
}

fn describe(path: PathBuf) -> ExternalPlayer {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let name = if lower.contains("mpv") {
        "mpv"
    } else if lower.contains("vlc") {
        "VLC"
    } else {
        "External player"
    };
    ExternalPlayer { name: name.into(), path: path.to_string_lossy().into_owned() }
}

/// Find a player: the configured path first, then mpv, then VLC in common locations.
pub fn detect(configured: Option<&str>) -> Option<ExternalPlayer> {
    if let Some(p) = configured.map(str::trim).filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(describe(p));
        }
    }
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(which("mpv.exe"));
    for base in [env("ProgramFiles"), env("LOCALAPPDATA").map(|p| p.join("Programs")), Some(PathBuf::from("C:\\"))]
        .into_iter()
        .flatten()
    {
        candidates.push(base.join("mpv").join("mpv.exe"));
    }
    candidates.extend(which("vlc.exe"));
    for base in [env("ProgramFiles"), env("ProgramFiles(x86)")].into_iter().flatten() {
        candidates.push(base.join("VideoLAN").join("VLC").join("vlc.exe"));
    }
    candidates.into_iter().find(|p| p.is_file()).map(describe)
}

pub fn launch(player: &ExternalPlayer, url: &str, title: &str) -> std::io::Result<()> {
    let mut cmd = Command::new(&player.path);
    if player.name == "mpv" {
        cmd.arg(format!("--force-media-title={title}")).arg("--cache=yes").arg(url);
    } else if player.name == "VLC" {
        cmd.arg(format!("--meta-title={title}")).arg(url);
    } else {
        cmd.arg(url);
    }
    cmd.spawn().map(|_| ())
}
