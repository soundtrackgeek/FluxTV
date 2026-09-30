//! Xtream Codes API client (`player_api.php`).
//!
//! Xtream panels are notoriously loose with JSON types (`"stream_id": 5654` on one server,
//! `"stream_id": "5654"` on another, `null` for empty strings), so all fields go through
//! lenient deserializers.

use serde::de::Deserializer;
use serde::Deserialize;
use url::Url;

use crate::error::{Error, Result};
use crate::models::{AccountInfo, Category, Channel, ContentKind};

/// Accept string, number, bool or null and return a trimmed string.
fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<String, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::String(s) => s.trim().to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        _ => String::new(),
    })
}

/// Accept number, numeric string, bool or null and return an integer (0 when missing/invalid).
fn lenient_i64<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<i64, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        serde_json::Value::String(s) => s.trim().parse::<f64>().map(|f| f as i64).unwrap_or(0),
        serde_json::Value::Bool(b) => b as i64,
        _ => 0,
    })
}

fn lenient_strings<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(a) => a
            .into_iter()
            .filter_map(|x| match x {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

fn non_empty(s: String) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

#[derive(Deserialize, Default)]
struct RawUserInfo {
    #[serde(default, deserialize_with = "lenient_i64")]
    auth: i64,
    #[serde(default, deserialize_with = "lenient_string")]
    status: String,
    #[serde(default, deserialize_with = "lenient_i64")]
    exp_date: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    is_trial: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    active_cons: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    max_connections: i64,
    #[serde(default, deserialize_with = "lenient_strings")]
    allowed_output_formats: Vec<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    message: String,
}

#[derive(Deserialize, Default)]
struct RawServerInfo {
    #[serde(default, deserialize_with = "lenient_string")]
    timezone: String,
}

#[derive(Deserialize)]
struct RawAccount {
    #[serde(default)]
    user_info: Option<RawUserInfo>,
    #[serde(default)]
    server_info: Option<RawServerInfo>,
}

#[derive(Deserialize)]
struct RawCategory {
    #[serde(deserialize_with = "lenient_string")]
    category_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    category_name: String,
}

#[derive(Deserialize)]
struct RawStream {
    #[serde(default, deserialize_with = "lenient_i64")]
    num: i64,
    #[serde(default, deserialize_with = "lenient_string")]
    name: String,
    /// Live/VOD use `stream_id`; series use `series_id`.
    #[serde(default, deserialize_with = "lenient_string")]
    stream_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    series_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    stream_icon: String,
    #[serde(default, deserialize_with = "lenient_string")]
    cover: String,
    #[serde(default, deserialize_with = "lenient_string")]
    epg_channel_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    category_id: String,
    #[serde(default, deserialize_with = "lenient_i64")]
    tv_archive: i64,
    #[serde(default, deserialize_with = "lenient_i64")]
    tv_archive_duration: i64,
    #[serde(default, deserialize_with = "lenient_string")]
    container_extension: String,
}

/// Credentials and base URL for an Xtream panel.
#[derive(Clone, Debug, PartialEq)]
pub struct XtreamCredentials {
    /// `scheme://host[:port]`, no trailing slash.
    pub base: String,
    pub username: String,
    pub password: String,
}

impl XtreamCredentials {
    pub fn new(base: &str, username: &str, password: &str) -> Result<Self> {
        Ok(Self {
            base: normalize_base(base)?,
            username: username.trim().to_string(),
            password: password.trim().to_string(),
        })
    }

    /// Recognize a pasted Xtream M3U link (`.../get.php?username=u&password=p&type=m3u_plus`)
    /// or API link (`player_api.php?...`) and extract credentials from it.
    pub fn from_url(link: &str) -> Option<Self> {
        let url = Url::parse(link.trim()).ok()?;
        let path = url.path().to_ascii_lowercase();
        if !(path.ends_with("/get.php") || path.ends_with("/player_api.php") || path.ends_with("/xmltv.php")) {
            return None;
        }
        let mut user = None;
        let mut pass = None;
        for (k, v) in url.query_pairs() {
            match k.as_ref() {
                "username" => user = Some(v.to_string()),
                // Tolerate a common copy/paste typo: `password=secret=m3u_plus` (missing `&type`).
                "password" => pass = Some(v.split('=').next().unwrap_or_default().to_string()),
                _ => {}
            }
        }
        let base = format!("{}://{}", url.scheme(), url.host_str()?);
        let base = match url.port() {
            Some(p) => format!("{base}:{p}"),
            None => base,
        };
        Some(Self { base, username: user.filter(|u| !u.is_empty())?, password: pass.filter(|p| !p.is_empty())? })
    }

    fn api_url(&self) -> Result<Url> {
        let mut url = Url::parse(&format!("{}/player_api.php", self.base)).map_err(|e| Error::Invalid(e.to_string()))?;
        url.query_pairs_mut().append_pair("username", &self.username).append_pair("password", &self.password);
        Ok(url)
    }

    /// Full XMLTV guide URL.
    pub fn epg_url(&self) -> String {
        let mut url = Url::parse(&format!("{}/xmltv.php", self.base)).expect("base validated");
        url.query_pairs_mut().append_pair("username", &self.username).append_pair("password", &self.password);
        url.to_string()
    }

    /// Playback URL for a stream. Live streams use MPEG-TS (the one format every panel allows).
    pub fn stream_url(&self, kind: ContentKind, stream_id: &str, container: Option<&str>) -> String {
        let (segment, ext) = match kind {
            ContentKind::Live => ("live", "ts"),
            ContentKind::Movie => ("movie", container.unwrap_or("mp4")),
            ContentKind::Series => ("series", container.unwrap_or("mp4")),
        };
        format!(
            "{}/{}/{}/{}/{}.{}",
            self.base,
            segment,
            urlencode_path(&self.username),
            urlencode_path(&self.password),
            stream_id,
            ext
        )
    }
}

fn urlencode_path(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

/// Normalize user input like `example.com:8080/`, `http://example.com/get.php?...` to `http://example.com:8080`.
pub fn normalize_base(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(Error::Invalid("server URL is empty".into()));
    }
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("http://{input}") };
    let url = Url::parse(&with_scheme).map_err(|e| Error::Invalid(format!("server URL: {e}")))?;
    let host = url.host_str().ok_or_else(|| Error::Invalid("server URL has no host".into()))?;
    Ok(match url.port() {
        Some(p) => format!("{}://{}:{}", url.scheme(), host, p),
        None => format!("{}://{}", url.scheme(), host),
    })
}

/// Async client for one Xtream account.
#[derive(Clone)]
pub struct XtreamClient {
    http: reqwest::Client,
    creds: XtreamCredentials,
}

impl XtreamClient {
    pub fn new(http: reqwest::Client, creds: XtreamCredentials) -> Self {
        Self { http, creds }
    }

    pub fn credentials(&self) -> &XtreamCredentials {
        &self.creds
    }

    async fn get_json(&self, action: Option<&str>) -> Result<serde_json::Value> {
        let mut url = self.creds.api_url()?;
        if let Some(a) = action {
            url.query_pairs_mut().append_pair("action", a);
        }
        let resp = self.http.get(url).send().await?.error_for_status()?;
        let bytes = resp.bytes().await?;
        if bytes.is_empty() {
            return Ok(serde_json::Value::Array(vec![]));
        }
        serde_json::from_slice(&bytes).map_err(Error::from)
    }

    /// Validate credentials and fetch subscription details.
    pub async fn account(&self) -> Result<AccountInfo> {
        let v = self.get_json(None).await?;
        let raw: RawAccount = serde_json::from_value(v).map_err(|_| Error::Auth("unexpected response from server".into()))?;
        let user = raw.user_info.ok_or_else(|| Error::Auth("invalid username or password".into()))?;
        if user.auth != 1 {
            return Err(Error::Auth("invalid username or password".into()));
        }
        Ok(AccountInfo {
            status: user.status,
            expires_at: (user.exp_date > 0).then_some(user.exp_date),
            is_trial: user.is_trial != 0,
            active_connections: user.active_cons,
            max_connections: user.max_connections,
            allowed_formats: user.allowed_output_formats,
            server_timezone: raw.server_info.and_then(|s| non_empty(s.timezone)),
            message: non_empty(fix_mojibake(&user.message)),
        })
    }

    pub async fn categories(&self, kind: ContentKind) -> Result<Vec<Category>> {
        let action = match kind {
            ContentKind::Live => "get_live_categories",
            ContentKind::Movie => "get_vod_categories",
            ContentKind::Series => "get_series_categories",
        };
        let v = self.get_json(Some(action)).await?;
        let raw: Vec<RawCategory> = if v.is_array() { serde_json::from_value(v)? } else { Vec::new() };
        Ok(raw
            .into_iter()
            .map(|c| Category { id: c.category_id, name: c.category_name, kind, channel_count: 0 })
            .collect())
    }

    pub async fn streams(&self, kind: ContentKind) -> Result<Vec<Channel>> {
        let action = match kind {
            ContentKind::Live => "get_live_streams",
            ContentKind::Movie => "get_vod_streams",
            ContentKind::Series => "get_series",
        };
        let v = self.get_json(Some(action)).await?;
        let raw: Vec<RawStream> = if v.is_array() { serde_json::from_value(v)? } else { Vec::new() };
        Ok(raw.into_iter().filter_map(|s| to_channel(s, kind)).collect())
    }
}

fn to_channel(s: RawStream, kind: ContentKind) -> Option<Channel> {
    let id = if kind == ContentKind::Series { s.series_id } else { s.stream_id };
    if id.is_empty() {
        return None;
    }
    let logo = non_empty(s.stream_icon).or_else(|| non_empty(s.cover));
    Some(Channel {
        id,
        kind,
        num: s.num,
        name: s.name,
        logo,
        category_id: s.category_id,
        epg_id: non_empty(s.epg_channel_id),
        catchup_days: if s.tv_archive != 0 { s.tv_archive_duration.max(1) } else { 0 },
        container: non_empty(s.container_extension),
        favorite: false,
        url: None,
    })
}

/// Some panels double-encode UTF-8 (`â„¢` instead of `™`). Undo it when the result is valid UTF-8.
fn fix_mojibake(s: &str) -> String {
    if s.chars().all(|c| (c as u32) < 0x100 || "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ".contains(c)) {
        let bytes: Option<Vec<u8>> = s
            .chars()
            .map(|c| {
                if (c as u32) < 0x100 {
                    Some(c as u8)
                } else {
                    // Map Windows-1252 specials back to their byte values.
                    let table = "€\u{81}‚ƒ„…†‡ˆ‰Š‹Œ\u{8d}Ž\u{8f}\u{90}‘’“”•–—˜™š›œ\u{9d}žŸ";
                    table.chars().position(|t| t == c).map(|i| 0x80 + i as u8)
                }
            })
            .collect();
        if let Some(Ok(fixed)) = bytes.map(String::from_utf8) {
            return fixed;
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pasted_m3u_links() {
        let c = XtreamCredentials::from_url(
            "http://ourxtream.com/get.php?username=alice&password=s3cret=m3u_plus",
        )
        .unwrap();
        assert_eq!(c.base, "http://ourxtream.com");
        assert_eq!(c.username, "alice");
        assert_eq!(c.password, "s3cret");

        let c = XtreamCredentials::from_url("https://x.tv:8443/player_api.php?username=a&password=b").unwrap();
        assert_eq!(c.base, "https://x.tv:8443");
        assert!(XtreamCredentials::from_url("http://example.com/list.m3u").is_none());
    }

    #[test]
    fn normalizes_base() {
        assert_eq!(normalize_base("example.com:8080/").unwrap(), "http://example.com:8080");
        assert_eq!(normalize_base("https://example.com/get.php?x=1").unwrap(), "https://example.com");
        assert!(normalize_base("  ").is_err());
    }

    #[test]
    fn builds_stream_urls() {
        let c = XtreamCredentials::new("http://h:80", "u", "p w").unwrap();
        assert_eq!(c.stream_url(ContentKind::Live, "42", None), "http://h/live/u/p%20w/42.ts");
        assert_eq!(c.stream_url(ContentKind::Movie, "7", Some("mkv")), "http://h/movie/u/p%20w/7.mkv");
        assert!(c.epg_url().starts_with("http://h/xmltv.php?username=u&password=p+w"));
    }

    #[test]
    fn lenient_stream_json() {
        let json = r#"[
          {"num":1,"name":"A","stream_id":5654,"stream_icon":"","epg_channel_id":null,"category_id":"50","tv_archive":"1","tv_archive_duration":"7"},
          {"num":"2","name":"B","stream_id":"77","category_id":3,"tv_archive":0,"tv_archive_duration":0},
          {"name":"no id"}
        ]"#;
        let raw: Vec<RawStream> = serde_json::from_str(json).unwrap();
        let chans: Vec<Channel> = raw.into_iter().filter_map(|s| to_channel(s, ContentKind::Live)).collect();
        assert_eq!(chans.len(), 2);
        assert_eq!(chans[0].id, "5654");
        assert_eq!(chans[0].logo, None);
        assert_eq!(chans[0].epg_id, None);
        assert_eq!(chans[0].catchup_days, 7);
        assert_eq!(chans[1].num, 2);
        assert_eq!(chans[1].category_id, "3");
    }

    #[test]
    fn fixes_double_encoded_utf8() {
        assert_eq!(fix_mojibake("Powered by Valhalla\u{00e2}\u{201e}\u{00a2}"), "Powered by Valhalla™");
        assert_eq!(fix_mojibake("Plain"), "Plain");
        assert_eq!(fix_mojibake("Ærlig"), "Ærlig");
    }
}
