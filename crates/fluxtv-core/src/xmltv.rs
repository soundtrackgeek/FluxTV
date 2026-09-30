//! Streaming XMLTV parser.
//!
//! Guides are large (tens of MB, hundreds of thousands of programmes), so we never build
//! a DOM: channels and programmes are handed to callbacks as soon as their closing tag is seen.

use std::io::BufRead;

use quick_xml::{Reader, XmlVersion};
use quick_xml::events::{BytesStart, Event};

use crate::error::Result;
use crate::models::{EpgChannel, Programme};
use crate::time::parse_xmltv_time;

/// Which text-bearing child element we are currently inside.
#[derive(Clone, Copy, PartialEq)]
enum Field {
    None,
    DisplayName,
    Title,
    SubTitle,
    Desc,
    Category,
    EpisodeNum,
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name).and_then(|a| {
        a.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    })
}

fn resolve_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => None,
    }
}

/// An item emitted while streaming through an XMLTV document.
#[derive(Debug)]
pub enum XmltvItem {
    Channel(EpgChannel),
    Programme(Programme),
}

/// Parse an XMLTV document, invoking `on_item` for each channel and programme.
///
/// Programmes with unparseable times are skipped. The first title/description wins when a
/// programme carries several languages.
pub fn parse<R: BufRead>(
    input: R,
    mut on_item: impl FnMut(XmltvItem),
) -> Result<()> {
    let mut reader = Reader::from_reader(input);
    reader.config_mut().check_end_names = false;

    let mut buf = Vec::with_capacity(64 * 1024);
    let mut channel: Option<EpgChannel> = None;
    let mut programme: Option<Programme> = None;
    let mut field = Field::None;
    let mut text = String::new();

    loop {
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_empty = matches!(event, Event::Empty(_));
                match e.name().as_ref() {
                    "channel" => {
                        if let Some(id) = attr(e, "id") {
                            let ch = EpgChannel { id, ..Default::default() };
                            if is_empty {
                                on_item(XmltvItem::Channel(ch));
                            } else {
                                channel = Some(ch);
                            }
                        }
                    }
                    "programme" => {
                        let start = attr(e, "start").as_deref().and_then(parse_xmltv_time);
                        let stop = attr(e, "stop").as_deref().and_then(parse_xmltv_time);
                        let ch = attr(e, "channel");
                        if let (Some(start), Some(ch)) = (start, ch) {
                            programme = Some(Programme {
                                channel: ch,
                                start,
                                // Missing stop: assume one hour; fixed up by storage if a successor exists.
                                stop: stop.unwrap_or(start + 3600),
                                title: String::new(),
                                subtitle: None,
                                description: None,
                                category: None,
                                icon: None,
                                episode: None,
                            });
                        }
                    }
                    "icon" => {
                        let src = attr(e, "src");
                        if let Some(p) = programme.as_mut() {
                            if p.icon.is_none() {
                                p.icon = src;
                            }
                        } else if let Some(c) = channel.as_mut()
                            && c.icon.is_none() {
                                c.icon = src;
                            }
                    }
                    name if !is_empty => {
                        field = match name {
                            "display-name" if channel.is_some() => Field::DisplayName,
                            "title" if programme.is_some() => Field::Title,
                            "sub-title" if programme.is_some() => Field::SubTitle,
                            "desc" if programme.is_some() => Field::Desc,
                            "category" if programme.is_some() => Field::Category,
                            "episode-num" if programme.is_some() => {
                                // Prefer the human readable `onscreen` form (S01E02) over `xmltv_ns`.
                                let system = attr(e, "system");
                                let has_episode = programme.as_ref().is_some_and(|p| p.episode.is_some());
                                if system.as_deref() == Some("onscreen") || !has_episode {
                                    Field::EpisodeNum
                                } else {
                                    Field::None
                                }
                            }
                            _ => Field::None,
                        };
                        text.clear();
                    }
                    _ => {}
                }
            }
            Event::Text(ref t) if field != Field::None => text.push_str(&t.html_content()),
            Event::CData(ref t) if field != Field::None => text.push_str(&t.html_content()),
            Event::GeneralRef(ref r) if field != Field::None => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                } else if let Some(c) = resolve_entity(r) {
                    text.push(c);
                }
            }
            Event::End(ref e) => {
                match e.name().as_ref() {
                    "channel" => {
                        if let Some(c) = channel.take() {
                            on_item(XmltvItem::Channel(c));
                        }
                    }
                    "programme" => {
                        if let Some(p) = programme.take()
                            && !p.title.is_empty() && p.stop > p.start {
                                on_item(XmltvItem::Programme(p));
                            }
                    }
                    _ if field != Field::None => {
                        let value = text.trim();
                        if !value.is_empty() {
                            if let Some(c) = channel.as_mut().filter(|_| field == Field::DisplayName) {
                                c.display_names.push(value.to_string());
                            } else if let Some(p) = programme.as_mut() {
                                let slot = match field {
                                    Field::Title if p.title.is_empty() => {
                                        p.title = value.to_string();
                                        None
                                    }
                                    Field::SubTitle => Some(&mut p.subtitle),
                                    Field::Desc => Some(&mut p.description),
                                    Field::Category => Some(&mut p.category),
                                    Field::EpisodeNum => {
                                        p.episode = Some(value.to_string());
                                        None
                                    }
                                    _ => None,
                                };
                                if let Some(slot) = slot
                                    && slot.is_none() {
                                        *slot = Some(value.to_string());
                                    }
                            }
                        }
                        field = Field::None;
                        text.clear();
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

/// Wrap a reader so gzip-compressed guides (`.xml.gz`) are transparently decompressed.
pub fn maybe_gunzip(bytes: &[u8]) -> Box<dyn BufRead + '_> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        Box::new(std::io::BufReader::new(flate2::read::MultiGzDecoder::new(bytes)))
    } else {
        Box::new(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<tv generator-info-name="test">
  <channel id="nrk1.no"><display-name>NRK1</display-name><display-name>NOR| NRK1 HD</display-name><icon src="http://x/nrk1.png"/></channel>
  <channel id="empty.no"/>
  <programme start="20260922110000 +0000" stop="20260922120000 +0000" channel="nrk1.no">
    <title lang="nb">Dagsrevyen</title>
    <title lang="en">News</title>
    <sub-title>Episode &amp; more</sub-title>
    <desc lang="en">Britain&apos;s &#8220;best&#8221; news</desc>
    <category>News</category>
    <episode-num system="xmltv_ns">0.1.</episode-num>
    <episode-num system="onscreen">S01E02</episode-num>
    <icon src="http://x/p.jpg"/>
  </programme>
  <programme start="20260922120000 +0200" channel="nrk1.no"><title><![CDATA[Late <Show>]]></title></programme>
  <programme start="bad" stop="20260922120000 +0000" channel="nrk1.no"><title>Skipped</title></programme>
</tv>"#;

    #[test]
    fn parses_channels_and_programmes() {
        let mut channels = Vec::new();
        let mut progs = Vec::new();
        parse(SAMPLE.as_bytes(), |item| match item {
            XmltvItem::Channel(c) => channels.push(c),
            XmltvItem::Programme(p) => progs.push(p),
        })
        .unwrap();

        assert_eq!(channels.len(), 2);
        assert_eq!(channels[0].id, "nrk1.no");
        assert_eq!(channels[0].display_names, vec!["NRK1", "NOR| NRK1 HD"]);
        assert_eq!(channels[0].icon.as_deref(), Some("http://x/nrk1.png"));

        assert_eq!(progs.len(), 2);
        let p = &progs[0];
        assert_eq!(p.title, "Dagsrevyen");
        assert_eq!(p.subtitle.as_deref(), Some("Episode & more"));
        assert_eq!(p.description.as_deref(), Some("Britain's \u{201c}best\u{201d} news"));
        assert_eq!(p.category.as_deref(), Some("News"));
        assert_eq!(p.episode.as_deref(), Some("S01E02"));
        assert_eq!(p.icon.as_deref(), Some("http://x/p.jpg"));
        assert_eq!(p.stop - p.start, 3600);

        assert_eq!(progs[1].title, "Late <Show>");
        assert_eq!(progs[1].start, p.start + 3600 - 7200);
    }

    #[test]
    fn gunzips() {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(SAMPLE.as_bytes()).unwrap();
        let gz = enc.finish().unwrap();
        let mut n = 0;
        parse(maybe_gunzip(&gz), |item| {
            if matches!(item, XmltvItem::Programme(_)) {
                n += 1
            }
        })
        .unwrap();
        assert_eq!(n, 2);
    }
}
