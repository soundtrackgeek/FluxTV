//! Text normalization and ranking for channel search.
//!
//! IPTV channel names are messy: `NOR| NRK1 HD`, `GOLD| NRK1 ᴿᴬᵂ`, `UK| SKY SPORTS ᵁᴴᴰ`.
//! We normalize both names and queries to lowercase ASCII-ish words so that typing
//! `nrk1 raw` or `sky sports uhd` just works.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Normalize text for matching: compatibility-decompose (turns superscript `ᴴᴰ` into `HD`),
/// drop accents, lowercase, and collapse everything that is not a letter/digit into single spaces.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.nfkd() {
        if is_combining_mark(c) {
            continue;
        }
        let mapped: &str = match c {
            'ø' | 'Ø' => "o",
            'æ' | 'Æ' => "ae",
            'ß' => "ss",
            'đ' | 'Đ' => "d",
            'ł' | 'Ł' => "l",
            'þ' | 'Þ' => "th",
            _ => "",
        };
        if !mapped.is_empty() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push_str(mapped);
        } else if c.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.extend(c.to_lowercase());
        } else {
            pending_space = true;
        }
    }
    out
}

/// Split a query into normalized tokens.
pub fn tokens(query: &str) -> Vec<String> {
    normalize(query).split(' ').filter(|t| !t.is_empty()).map(str::to_owned).collect()
}

/// Score a normalized name against query tokens. `None` means "does not match".
/// Every token must appear somewhere; whole-word and prefix hits rank higher.
pub fn score(name_norm: &str, tokens: &[String]) -> Option<i64> {
    if tokens.is_empty() {
        return None;
    }
    let words: Vec<&str> = name_norm.split(' ').collect();
    let mut compact: Option<String> = None;
    let mut total = 0i64;
    for (i, t) in tokens.iter().enumerate() {
        let mut best = 0i64;
        for (wi, w) in words.iter().enumerate() {
            let s = if *w == t {
                30
            } else if w.starts_with(t.as_str()) {
                20
            } else if w.contains(t.as_str()) {
                6
            } else {
                0
            };
            // Matching the first query token early in the name is a strong signal
            // (`nrk` should prefer `NRK1 HD` over `Something NRK Extra`).
            let s = if i == 0 && s > 0 && wi <= 1 { s + 8 } else { s };
            best = best.max(s);
        }
        if best == 0 {
            // Allow `nrk1` to match `nrk 1` and vice versa.
            let c = compact.get_or_insert_with(|| name_norm.replace(' ', ""));
            if c.contains(t.as_str()) {
                best = 4;
            } else {
                return None;
            }
        }
        total += best;
    }
    // Prefer shorter names when scores tie: `NRK1 HD` before `NRK1 HD BACKUP FEED 3`.
    total -= (name_norm.len() as i64) / 8;
    Some(total)
}

/// Longest token, used as the cheap SQL `LIKE` pre-filter before ranking in Rust.
pub fn most_selective_token(tokens: &[String]) -> Option<&str> {
    tokens.iter().max_by_key(|t| t.len()).map(String::as_str)
}

/// Words that describe quality/format rather than identity; stripped when matching
/// channel names against XMLTV display names.
const NOISE_WORDS: &[&str] = &[
    "hd", "fhd", "uhd", "sd", "4k", "8k", "hevc", "h265", "raw", "hq", "lq", "backup", "vip", "50fps",
    "60fps", "fps", "1080p", "720p", "2160p", "3840p", "ultra",
];

/// A looser key for fuzzy EPG matching: normalized, without country prefix and quality tags.
pub fn epg_match_key(name: &str) -> String {
    let n = normalize(name);
    let mut words: Vec<&str> = n.split(' ').filter(|w| !w.is_empty()).collect();
    // Drop a leading country/group prefix such as `uk`, `nor`, `us`, `de` when the original
    // name used the common `XX| Name` or `XX: Name` convention.
    if words.len() > 1 && (name.contains('|') || name.contains(':')) && words[0].len() <= 4 {
        words.remove(0);
    }
    words.retain(|w| !NOISE_WORDS.contains(w));
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_superscripts_and_accents() {
        assert_eq!(normalize("GOLD| NRK1 ᴿᴬᵂ"), "gold nrk1 raw");
        assert_eq!(normalize("WORLD CUP 2026 ᵁᴴᴰ ³⁸⁴⁰ᴾ"), "world cup 2026 uhd 3840p");
        assert_eq!(normalize("NRK1 MØRE OG ROMSDAL"), "nrk1 more og romsdal");
        assert_eq!(normalize("Tegnspråk"), "tegnsprak");
        assert_eq!(normalize("  UK|  BBC   One  "), "uk bbc one");
    }

    #[test]
    fn scoring_requires_all_tokens() {
        let t = tokens("nrk1 raw");
        assert!(score("gold nrk1 raw", &t).is_some());
        assert!(score("nor nrk1 hd", &t).is_none());
    }

    #[test]
    fn scoring_prefers_whole_words_and_short_names() {
        let t = tokens("nrk");
        let a = score("nor nrk1 hd", &t).unwrap();
        let b = score("nor something nrk1 extra long backup name", &t).unwrap();
        assert!(a > b);
        let t = tokens("tv2");
        assert!(score("nor tv2 hd", &t).unwrap() > score("nor tv2 sport premium 2 hd", &t).unwrap());
    }

    #[test]
    fn compact_matching() {
        assert!(score("nrk 1", &tokens("nrk1")).is_some());
    }

    #[test]
    fn epg_keys() {
        assert_eq!(epg_match_key("UK| ALIBI FHD"), "alibi");
        assert_eq!(epg_match_key("Alibi"), "alibi");
        assert_eq!(epg_match_key("NOR| NRK1 HD"), "nrk1");
        assert_eq!(epg_match_key("GOLD| NRK1 ᴿᴬᵂ"), "nrk1");
    }
}
