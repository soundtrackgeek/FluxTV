//! MPEG-TS → HLS repackaging for players that cannot play a raw TS stream (AVPlayer).
//!
//! Many Xtream subscriptions only allow the `ts` output format for live channels. Apple's
//! media stack plays MPEG-TS only inside HLS, so [`Segmenter`] cuts the continuous stream
//! into keyframe-aligned segments and [`render_live_playlist`] publishes a sliding window
//! over them. Packets are copied as-is (no remuxing, no transcoding) with two exceptions:
//!
//! - every segment starts with the latest PAT and PMT, so it decodes on its own;
//! - DVB-style AC-3 / E-AC-3 streams (stream type `0x06` + descriptor) are relabelled with
//!   their ATSC stream types (`0x81` / `0x87`), which is what Apple's HLS demuxer expects.
//!
//! The first few segments contain one GOP each so a player can start quickly. Later segments
//! group short GOPs into roughly two seconds. The advertised target covers both phases.

use std::fmt::Write as _;

pub const TS_PACKET: usize = 188;
const SYNC: u8 = 0x47;
const PTS_HZ: f64 = 90_000.0;
const PTS_MASK: u64 = (1 << 33) - 1;
const NULL_PID: u16 = 0x1fff;
/// Preferred minimum segment length; short GOPs are grouped until they reach it.
const MIN_SEGMENT_SECS: f64 = 1.5;
const MAX_TARGET_SECS: f64 = 12.0;
/// Target used before the GOP length is known (and for audio-only streams).
const DEFAULT_TARGET_SECS: f64 = 2.0;
const STARTUP_SEGMENTS: usize = 3;
const STARTUP_AUDIO_SECS: f64 = 1.0;

/// One finished HLS media segment.
#[derive(Clone, Debug)]
pub struct Segment {
    pub sequence: u64,
    /// Seconds, from presentation timestamps.
    pub duration: f64,
    /// The segment does not continue the previous one (reconnect or timestamp jump).
    pub discontinuity: bool,
    pub data: Vec<u8>,
}

/// Codecs found in the stream's PMT, as human-readable names (`H.264`, `AAC`, ...).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamInfo {
    pub video: Option<String>,
    pub audio: Vec<String>,
}

impl StreamInfo {
    /// Codecs Apple's HLS stack cannot decode, if any.
    pub fn unsupported(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(v) = &self.video
            && !matches!(v.as_str(), "H.264" | "HEVC")
        {
            out.push(v.clone());
        }
        let playable_audio = self.audio.iter().any(|a| matches!(a.as_str(), "AAC" | "MP3" | "AC-3" | "E-AC-3"));
        if !self.audio.is_empty() && !playable_audio {
            out.extend(self.audio.iter().cloned());
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EsKind {
    H264,
    Hevc,
    OtherVideo,
    Audio,
}

#[derive(Clone, Debug)]
struct Es {
    pid: u16,
    kind: EsKind,
}

/// Incremental TS → segments splitter. Feed bytes with [`push`](Self::push), collect
/// finished segments with [`take_segments`](Self::take_segments).
pub struct Segmenter {
    carry: Vec<u8>,
    pat: Option<[u8; TS_PACKET]>,
    pmt_pid: Option<u16>,
    /// PMT section being reassembled (multi-packet PMTs are rare but legal).
    pmt_buf: Vec<u8>,
    pmt_packets: Vec<[u8; TS_PACKET]>,
    /// Latest PMT packets, patched if a stream type was relabelled.
    pmt_out: Vec<[u8; TS_PACKET]>,
    /// Single-packet PMTs we patched: the rewritten section bytes to substitute in-stream.
    pmt_patch: Option<[u8; TS_PACKET]>,
    info: StreamInfo,
    timing: Option<Es>,

    current: Vec<u8>,
    current_start: Option<u64>,
    current_last: Option<u64>,
    startup_remaining: usize,
    target: Option<f64>,
    min_cut: f64,
    pending_discontinuity: bool,
    next_sequence: u64,
    done: Vec<Segment>,
}

impl Default for Segmenter {
    fn default() -> Self {
        Self::new()
    }
}

impl Segmenter {
    pub fn new() -> Self {
        Self {
            carry: Vec::with_capacity(TS_PACKET * 2),
            pat: None,
            pmt_pid: None,
            pmt_buf: Vec::new(),
            pmt_packets: Vec::new(),
            pmt_out: Vec::new(),
            pmt_patch: None,
            info: StreamInfo::default(),
            timing: None,
            current: Vec::new(),
            current_start: None,
            current_last: None,
            startup_remaining: STARTUP_SEGMENTS,
            target: None,
            min_cut: MIN_SEGMENT_SECS,
            pending_discontinuity: false,
            next_sequence: 0,
            done: Vec::new(),
        }
    }

    /// Codecs announced by the stream, once its PMT has been seen.
    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    /// `EXT-X-TARGETDURATION` in whole seconds, once known.
    pub fn target_duration(&self) -> Option<u32> {
        self.target.map(|t| t.ceil() as u32)
    }

    /// Finished segments since the last call.
    pub fn take_segments(&mut self) -> Vec<Segment> {
        std::mem::take(&mut self.done)
    }

    /// The upstream connection was replaced: drop the partial segment and program state,
    /// and flag the next segment as a discontinuity.
    pub fn discontinuity(&mut self) {
        self.carry.clear();
        self.current.clear();
        self.current_start = None;
        self.current_last = None;
        self.pat = None;
        self.pmt_pid = None;
        self.pmt_buf.clear();
        self.pmt_packets.clear();
        self.pmt_out.clear();
        self.pmt_patch = None;
        self.timing = None;
        self.startup_remaining = STARTUP_SEGMENTS;
        self.pending_discontinuity = true;
    }

    /// Feed raw bytes from the upstream connection.
    pub fn push(&mut self, mut data: &[u8]) {
        if !self.carry.is_empty() {
            let need = TS_PACKET - self.carry.len();
            let take = need.min(data.len());
            self.carry.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.carry.len() < TS_PACKET {
                return;
            }
            let pkt = std::mem::take(&mut self.carry);
            if pkt[0] == SYNC {
                self.packet(&pkt);
            } else {
                // Lost sync: rescan the carried bytes together with the new data.
                let mut joined = pkt[1..].to_vec();
                joined.extend_from_slice(data);
                return self.push_aligned(&joined);
            }
        }
        self.push_aligned(data);
    }

    fn push_aligned(&mut self, data: &[u8]) {
        let mut i = 0;
        while i < data.len() {
            if data[i] != SYNC || (i + TS_PACKET < data.len() && data[i + TS_PACKET] != SYNC) {
                i += 1;
                continue;
            }
            if i + TS_PACKET > data.len() {
                break;
            }
            let pkt: &[u8] = &data[i..i + TS_PACKET];
            self.packet(pkt);
            i += TS_PACKET;
        }
        if i < data.len() {
            self.carry.extend_from_slice(&data[i..]);
        }
    }

    fn packet(&mut self, pkt: &[u8]) {
        let pusi = pkt[1] & 0x40 != 0;
        let pid = (u16::from(pkt[1] & 0x1f) << 8) | u16::from(pkt[2]);
        if pid == NULL_PID {
            return;
        }
        let afc = (pkt[3] >> 4) & 0x3;
        let mut off = 4;
        let mut random_access = false;
        if afc & 0x2 != 0 {
            let len = pkt[4] as usize;
            if len > 0 && len <= 183 {
                random_access = pkt[5] & 0x40 != 0;
            }
            off = 5 + len;
        }
        let payload: &[u8] = if afc & 0x1 != 0 && off < TS_PACKET { &pkt[off..] } else { &[] };

        if pid == 0 {
            if pusi && !payload.is_empty() {
                self.parse_pat(payload);
                let mut copy = [0u8; TS_PACKET];
                copy.copy_from_slice(pkt);
                self.pat = Some(copy);
            }
            self.append(pkt);
            return;
        }
        if Some(pid) == self.pmt_pid {
            self.collect_pmt(pkt, pusi, payload);
            match self.pmt_patch {
                Some(mut patched) if pusi => {
                    patched[3] = (patched[3] & 0xf0) | (pkt[3] & 0x0f); // keep the continuity counter
                    self.append(&patched);
                }
                _ => self.append(pkt),
            }
            return;
        }

        if pusi
            && let Some(timing) = self.timing.clone()
            && timing.pid == pid
            && let Some((pts, es_start)) = pes_pts(payload)
        {
            let key = match timing.kind {
                EsKind::Audio => true,
                EsKind::H264 | EsKind::Hevc => random_access || has_keyframe_nal(&payload[es_start..], timing.kind),
                EsKind::OtherVideo => random_access,
            };
            self.on_timing_pes(pts, key, timing.kind == EsKind::Audio);
        }
        self.append(pkt);
    }

    fn append(&mut self, pkt: &[u8]) {
        if self.current_start.is_some() {
            self.current.extend_from_slice(pkt);
        }
    }

    fn start_segment(&mut self, pts: u64) {
        self.current.clear();
        if let Some(pat) = &self.pat {
            self.current.extend_from_slice(pat);
        }
        for p in &self.pmt_out {
            self.current.extend_from_slice(p);
        }
        self.current_start = Some(pts);
        self.current_last = Some(pts);
    }

    fn finish_segment(&mut self, duration: f64) {
        let psi_len = TS_PACKET * (usize::from(self.pat.is_some()) + self.pmt_out.len());
        if self.current.len() > psi_len && duration > 0.0 {
            self.done.push(Segment {
                sequence: self.next_sequence,
                duration,
                discontinuity: std::mem::take(&mut self.pending_discontinuity),
                data: std::mem::take(&mut self.current),
            });
            self.next_sequence += 1;
            self.startup_remaining = self.startup_remaining.saturating_sub(1);
        }
        self.current.clear();
        self.current_start = None;
    }

    fn on_timing_pes(&mut self, pts: u64, key: bool, audio_only: bool) {
        if self.pat.is_none() || self.pmt_out.is_empty() {
            return;
        }
        let Some(start) = self.current_start else {
            if key {
                self.start_segment(pts);
            }
            return;
        };
        let elapsed = pts.wrapping_sub(start) & PTS_MASK;
        let secs = elapsed as f64 / PTS_HZ;
        let limit = self.target.unwrap_or(MAX_TARGET_SECS);

        // A jump backwards (huge masked delta) or far forwards: timestamps were reset.
        if secs > (limit * 3.0).max(30.0) {
            let last = self.current_last.map(|l| (l.wrapping_sub(start) & PTS_MASK) as f64 / PTS_HZ).unwrap_or(0.0);
            let estimate = if last > 0.0 && last <= limit { last } else { limit.min(DEFAULT_TARGET_SECS) };
            self.finish_segment(estimate);
            self.pending_discontinuity = true;
            if key {
                self.start_segment(pts);
            }
            return;
        }

        if self.target.is_none() {
            if audio_only {
                self.set_target(DEFAULT_TARGET_SECS, DEFAULT_TARGET_SECS);
            } else if key && secs > 0.0 {
                // Advertise the steady-state target even during short startup segments.
                // The target must not grow just because startup ends.
                let gops = (MIN_SEGMENT_SECS / secs).ceil().max(1.0);
                let segment = gops * secs;
                self.set_target((segment - 0.05).ceil().clamp(DEFAULT_TARGET_SECS, MAX_TARGET_SECS), segment * 0.9);
            }
        }

        let startup_cut = self.startup_remaining > 0
            && key
            && secs > 0.0
            && (!audio_only || secs >= STARTUP_AUDIO_SECS);
        let cut = startup_cut || match self.target {
            Some(target) => (key && secs >= self.min_cut) || secs >= target + 0.45,
            None => secs >= MAX_TARGET_SECS,
        };
        if cut {
            if self.target.is_none() {
                self.set_target(MAX_TARGET_SECS, MAX_TARGET_SECS);
            }
            self.finish_segment(secs);
            self.start_segment(pts);
        } else {
            self.current_last = Some(pts);
        }
    }

    fn set_target(&mut self, target: f64, min_cut: f64) {
        self.target = Some(target);
        self.min_cut = min_cut.min(target);
    }

    fn parse_pat(&mut self, payload: &[u8]) {
        let Some(section) = section_after_pointer(payload) else { return };
        if section.len() < 12 || section[0] != 0x00 {
            return;
        }
        let len = (usize::from(section[1] & 0x0f) << 8) | usize::from(section[2]);
        let end = (3 + len).saturating_sub(4).min(section.len());
        let mut i = 8;
        while i + 4 <= end {
            let program = (u16::from(section[i]) << 8) | u16::from(section[i + 1]);
            let pid = (u16::from(section[i + 2] & 0x1f) << 8) | u16::from(section[i + 3]);
            if program != 0 {
                if self.pmt_pid != Some(pid) {
                    self.pmt_pid = Some(pid);
                    self.pmt_out.clear();
                    self.pmt_patch = None;
                }
                return;
            }
            i += 4;
        }
    }

    fn collect_pmt(&mut self, pkt: &[u8], pusi: bool, payload: &[u8]) {
        let mut copy = [0u8; TS_PACKET];
        copy.copy_from_slice(pkt);
        if pusi {
            let Some(section) = section_after_pointer(payload) else { return };
            self.pmt_buf.clear();
            self.pmt_buf.extend_from_slice(section);
            self.pmt_packets.clear();
        } else if self.pmt_packets.is_empty() || self.pmt_packets.len() >= 8 {
            return;
        } else {
            self.pmt_buf.extend_from_slice(payload);
        }
        self.pmt_packets.push(copy);
        if self.pmt_buf.len() < 3 {
            return;
        }
        let total = 3 + ((usize::from(self.pmt_buf[1] & 0x0f) << 8) | usize::from(self.pmt_buf[2]));
        if self.pmt_buf.len() < total {
            return;
        }
        let mut section = self.pmt_buf[..total].to_vec();
        let Some((streams, patched)) = parse_pmt(&mut section) else { return };
        let packets = std::mem::take(&mut self.pmt_packets);

        self.pmt_patch = None;
        self.pmt_out = packets.clone();
        if patched && packets.len() == 1 {
            // Rewrite the section in place inside a copy of the packet (same length, new CRC).
            let mut pkt = packets[0];
            let afc = (pkt[3] >> 4) & 0x3;
            let mut off = 4;
            if afc & 0x2 != 0 {
                off = 5 + pkt[4] as usize;
            }
            let start = off + 1 + pkt[off] as usize;
            if start + section.len() <= TS_PACKET {
                pkt[start..start + section.len()].copy_from_slice(&section);
                self.pmt_out = vec![pkt];
                self.pmt_patch = Some(pkt);
            }
        }

        let timing = streams
            .iter()
            .find(|s| s.kind != EsKind::Audio)
            .or_else(|| streams.iter().find(|s| s.kind == EsKind::Audio))
            .map(|s| Es { pid: s.pid, kind: s.kind });
        if self.timing.as_ref().map(|t| t.pid) != timing.as_ref().map(|t| t.pid) {
            // Different program layout: restart at the next keyframe of the new timing stream.
            self.current.clear();
            self.current_start = None;
        }
        self.timing = timing;
        self.info = StreamInfo {
            video: streams.iter().find(|s| s.kind != EsKind::Audio).map(|s| s.codec.clone()),
            audio: streams.iter().filter(|s| s.kind == EsKind::Audio).map(|s| s.codec.clone()).collect(),
        };
    }
}

#[derive(Clone)]
struct PmtStream {
    pid: u16,
    kind: EsKind,
    codec: String,
}

/// Parse a PMT section. Relabels DVB AC-3/E-AC-3 streams in place and fixes the CRC.
/// Returns the elementary streams and whether the section was modified.
fn parse_pmt(section: &mut [u8]) -> Option<(Vec<PmtStream>, bool)> {
    if section.len() < 16 || section[0] != 0x02 {
        return None;
    }
    let end = section.len() - 4;
    let program_info = (usize::from(section[10] & 0x0f) << 8) | usize::from(section[11]);
    let mut i = 12 + program_info;
    let mut streams = Vec::new();
    let mut patched = false;
    while i + 5 <= end {
        let stream_type = section[i];
        let pid = (u16::from(section[i + 1] & 0x1f) << 8) | u16::from(section[i + 2]);
        let info_len = (usize::from(section[i + 3] & 0x0f) << 8) | usize::from(section[i + 4]);
        let desc = &section[(i + 5).min(end)..(i + 5 + info_len).min(end)];
        let (kind, codec, relabel) = classify(stream_type, desc);
        if let Some(t) = relabel {
            section[i] = t;
            patched = true;
        }
        if let (Some(kind), Some(codec)) = (kind, codec) {
            streams.push(PmtStream { pid, kind, codec: codec.to_string() });
        }
        i += 5 + info_len;
    }
    if patched {
        let crc = crc32_mpeg2(&section[..end]);
        section[end..end + 4].copy_from_slice(&crc.to_be_bytes());
    }
    Some((streams, patched))
}

/// (kind, codec name, new stream type if it should be relabelled)
fn classify(stream_type: u8, descriptors: &[u8]) -> (Option<EsKind>, Option<&'static str>, Option<u8>) {
    match stream_type {
        0x1b => (Some(EsKind::H264), Some("H.264"), None),
        0x24 => (Some(EsKind::Hevc), Some("HEVC"), None),
        0x01 | 0x02 => (Some(EsKind::OtherVideo), Some("MPEG-2 Video"), None),
        0x10 => (Some(EsKind::OtherVideo), Some("MPEG-4 Video"), None),
        0x03 | 0x04 => (Some(EsKind::Audio), Some("MP3"), None),
        0x0f => (Some(EsKind::Audio), Some("AAC"), None),
        0x11 => (Some(EsKind::Audio), Some("AAC-LATM"), None),
        0x81 => (Some(EsKind::Audio), Some("AC-3"), None),
        0x87 => (Some(EsKind::Audio), Some("E-AC-3"), None),
        0x06 => {
            let mut j = 0;
            while j + 2 <= descriptors.len() {
                let tag = descriptors[j];
                let len = descriptors[j + 1] as usize;
                let body = &descriptors[(j + 2).min(descriptors.len())..(j + 2 + len).min(descriptors.len())];
                match tag {
                    0x6a => return (Some(EsKind::Audio), Some("AC-3"), Some(0x81)),
                    0x7a => return (Some(EsKind::Audio), Some("E-AC-3"), Some(0x87)),
                    0x05 if body == b"AC-3" => return (Some(EsKind::Audio), Some("AC-3"), Some(0x81)),
                    0x05 if body == b"EAC3" => return (Some(EsKind::Audio), Some("E-AC-3"), Some(0x87)),
                    _ => {}
                }
                j += 2 + len;
            }
            (None, None, None) // teletext, subtitles and other private data
        }
        _ => (None, None, None),
    }
}

fn section_after_pointer(payload: &[u8]) -> Option<&[u8]> {
    let pointer = *payload.first()? as usize;
    payload.get(1 + pointer..)
}

/// PTS of a PES packet starting in `payload`, plus the offset of its elementary stream data.
fn pes_pts(payload: &[u8]) -> Option<(u64, usize)> {
    if payload.len() < 14 || payload[0..3] != [0, 0, 1] || payload[7] & 0x80 == 0 {
        return None;
    }
    let p = &payload[9..14];
    let pts = (u64::from((p[0] >> 1) & 0x07) << 30)
        | (u64::from(p[1]) << 22)
        | (u64::from(p[2] >> 1) << 15)
        | (u64::from(p[3]) << 7)
        | u64::from(p[4] >> 1);
    let es_start = (9 + payload[8] as usize).min(payload.len());
    Some((pts, es_start))
}

/// Whether the start of an access unit contains an IDR/IRAP picture or a parameter set.
/// Fallback for encoders that do not set the random-access indicator.
fn has_keyframe_nal(es: &[u8], kind: EsKind) -> bool {
    let mut i = 0;
    while i + 3 < es.len() {
        if es[i] == 0 && es[i + 1] == 0 && es[i + 2] == 1 {
            let header = es[i + 3];
            let key = match kind {
                EsKind::H264 => matches!(header & 0x1f, 5 | 7),
                EsKind::Hevc => matches!((header >> 1) & 0x3f, 16..=21 | 32 | 33),
                _ => false,
            };
            if key {
                return true;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    false
}

/// CRC-32/MPEG-2 as used by PSI sections.
pub fn crc32_mpeg2(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04c1_1db7 } else { crc << 1 };
        }
    }
    crc
}

/// What a TS file's first bytes reveal (for movies served as MPEG-TS files).
#[derive(Clone, Debug)]
pub struct TsProbe {
    /// Offset of the first packet (files sometimes start with junk).
    pub sync_offset: usize,
    /// PAT + PMT packets (PMT relabelled if needed) to put in front of each segment.
    pub psi: Vec<u8>,
    pub pmt_pid: u16,
    /// Relabelled PMT packet to substitute for in-stream PMTs, if one was needed.
    pub patched_pmt: Option<[u8; TS_PACKET]>,
    /// PID used for timing (video, else audio).
    pub timing_pid: u16,
    pub first_pts: u64,
    pub info: StreamInfo,
}

/// Whether `data` looks like MPEG-TS (three consecutive sync bytes).
pub fn looks_like_ts(data: &[u8]) -> bool {
    find_sync(data).is_some()
}

fn find_sync(data: &[u8]) -> Option<usize> {
    (0..data.len().min(TS_PACKET * 4))
        .find(|&i| i + 2 * TS_PACKET < data.len() && data[i] == SYNC && data[i + TS_PACKET] == SYNC && data[i + 2 * TS_PACKET] == SYNC)
}

/// Parse program tables and the first timestamp from the start of a TS file.
pub fn probe_ts(data: &[u8]) -> Option<TsProbe> {
    let sync = find_sync(data)?;
    let mut seg = Segmenter::new();
    let mut first: Option<(u16, u64)> = None;
    let mut i = sync;
    while i + TS_PACKET <= data.len() && first.is_none() {
        let pkt = &data[i..i + TS_PACKET];
        seg.push(pkt);
        if let Some(t) = &seg.timing
            && pkt[1] & 0x40 != 0
            && ((u16::from(pkt[1] & 0x1f) << 8) | u16::from(pkt[2])) == t.pid
            && let Some((pts, _)) = pes_pts(payload_of(pkt))
        {
            first = Some((t.pid, pts));
        }
        i += TS_PACKET;
    }
    let (timing_pid, first_pts) = first?;
    let mut psi = Vec::new();
    psi.extend_from_slice(&seg.pat?);
    for p in &seg.pmt_out {
        psi.extend_from_slice(p);
    }
    Some(TsProbe {
        sync_offset: sync,
        psi,
        pmt_pid: seg.pmt_pid?,
        patched_pmt: seg.pmt_patch,
        timing_pid,
        first_pts,
        info: seg.info.clone(),
    })
}

fn payload_of(pkt: &[u8]) -> &[u8] {
    let afc = (pkt[3] >> 4) & 0x3;
    let off = if afc & 0x2 != 0 { 5 + pkt[4] as usize } else { 4 };
    if afc & 0x1 != 0 && off < TS_PACKET { &pkt[off..] } else { &[] }
}

/// The latest PTS of `pid` in a chunk of TS data (e.g. the end of a file).
pub fn last_pts(data: &[u8], pid: u16) -> Option<u64> {
    let sync = find_sync(data)?;
    let mut best: Option<u64> = None;
    let mut i = sync;
    while i + TS_PACKET <= data.len() {
        let pkt = &data[i..i + TS_PACKET];
        if pkt[0] != SYNC {
            break;
        }
        if pkt[1] & 0x40 != 0
            && ((u16::from(pkt[1] & 0x1f) << 8) | u16::from(pkt[2])) == pid
            && let Some((pts, _)) = pes_pts(payload_of(pkt))
        {
            // Frames are reordered (B-frames): keep the maximum, relative to the first seen.
            best = Some(match best {
                Some(b) if (pts.wrapping_sub(b) & PTS_MASK) > PTS_MASK / 2 => b,
                _ => pts,
            });
        }
        i += TS_PACKET;
    }
    best
}

/// Seconds from `first` to `last` PTS (33-bit wrap aware).
pub fn pts_span(first: u64, last: u64) -> f64 {
    (last.wrapping_sub(first) & PTS_MASK) as f64 / PTS_HZ
}

/// Replace in-stream PMT packets with a relabelled copy (keeping continuity counters).
pub fn patch_pmt_packets(data: &mut [u8], pmt_pid: u16, patched: &[u8; TS_PACKET]) {
    for pkt in data.chunks_exact_mut(TS_PACKET) {
        if pkt[0] == SYNC && pkt[1] & 0x40 != 0 && ((u16::from(pkt[1] & 0x1f) << 8) | u16::from(pkt[2])) == pmt_pid {
            let cc = pkt[3] & 0x0f;
            pkt.copy_from_slice(patched);
            pkt[3] = (pkt[3] & 0xf0) | cc;
        }
    }
}

/// Render a complete (VOD) media playlist with segment URIs `<n>.ts`.
pub fn render_vod_playlist(durations: &[f64]) -> String {
    let target = durations.iter().map(|d| d.ceil() as u32).max().unwrap_or(1).max(1);
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:0\n"
    );
    for (i, d) in durations.iter().enumerate() {
        let _ = write!(out, "#EXTINF:{d:.3},\n{i}.ts\n");
    }
    out.push_str("#EXT-X-ENDLIST\n");
    out
}

/// Minimal description of a segment for playlist rendering.
#[derive(Clone, Copy, Debug)]
pub struct PlaylistEntry {
    pub sequence: u64,
    pub duration: f64,
    pub discontinuity: bool,
}

/// Render a live (sliding window) media playlist. Segment URIs are `<sequence>.ts`,
/// relative to the playlist.
pub fn render_live_playlist(entries: &[PlaylistEntry], target_duration: u32, discontinuity_sequence: u64) -> String {
    let longest = entries.iter().map(|e| e.duration.round() as u32).max().unwrap_or(0);
    let target = target_duration.max(longest).max(1);
    let first = entries.first().map_or(0, |e| e.sequence);
    let mut out = String::with_capacity(96 + entries.len() * 40);
    let _ = write!(
        out,
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:{first}\n"
    );
    if discontinuity_sequence > 0 {
        let _ = writeln!(out, "#EXT-X-DISCONTINUITY-SEQUENCE:{discontinuity_sequence}");
    }
    for (i, e) in entries.iter().enumerate() {
        // A discontinuity on the first entry is expressed by the discontinuity sequence.
        if e.discontinuity && i > 0 {
            out.push_str("#EXT-X-DISCONTINUITY\n");
        }
        let _ = write!(out, "#EXTINF:{:.3},\n{}.ts\n", e.duration, e.sequence);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO: u16 = 0x100;
    const AUDIO: u16 = 0x101;
    const PMT: u16 = 0x1000;

    struct Mux {
        cc: std::collections::HashMap<u16, u8>,
        out: Vec<u8>,
    }

    impl Mux {
        fn new() -> Self {
            Self { cc: Default::default(), out: Vec::new() }
        }

        fn packet(&mut self, pid: u16, pusi: bool, rai: bool, payload: &[u8]) {
            let cc = self.cc.entry(pid).or_insert(0);
            let mut p = vec![SYNC, (if pusi { 0x40 } else { 0 }) | (pid >> 8) as u8, pid as u8];
            let body = &payload[..payload.len().min(if rai { 182 } else { 184 })];
            if rai || body.len() < 184 {
                // Adaptation field carrying the random-access flag and/or stuffing.
                let af_len = 183 - body.len();
                p.push(0x30 | *cc);
                p.push(af_len as u8);
                if af_len > 0 {
                    p.push(if rai { 0x40 } else { 0x00 });
                    p.extend(std::iter::repeat_n(0xff, af_len - 1));
                }
            } else {
                p.push(0x10 | *cc);
            }
            p.extend_from_slice(body);
            assert_eq!(p.len(), TS_PACKET, "packet size");
            *cc = (*cc + 1) & 0x0f;
            self.out.extend_from_slice(&p);
        }

        fn section(table: &[u8]) -> Vec<u8> {
            let mut s = table.to_vec();
            let crc = crc32_mpeg2(&s);
            s.extend_from_slice(&crc.to_be_bytes());
            let mut payload = vec![0u8]; // pointer field
            payload.extend(s);
            payload
        }

        fn pat(&mut self) {
            let t = [0x00, 0xb0, 13, 0x00, 0x01, 0xc1, 0, 0, 0x00, 0x01, 0xe0 | (PMT >> 8) as u8, PMT as u8];
            let payload = Self::section(&t);
            self.packet(0, true, false, &payload);
        }

        fn pmt(&mut self, audio_type: u8, audio_desc: &[u8]) {
            let mut es = vec![0x1b, 0xe0 | (VIDEO >> 8) as u8, VIDEO as u8, 0xf0, 0x00];
            es.extend([audio_type, 0xe0 | (AUDIO >> 8) as u8, AUDIO as u8, 0xf0, audio_desc.len() as u8]);
            es.extend_from_slice(audio_desc);
            let len = 9 + es.len() + 4;
            let mut t = vec![0x02, 0xb0 | (len >> 8) as u8, len as u8, 0x00, 0x01, 0xc1, 0, 0];
            t.extend([0xe0 | (VIDEO >> 8) as u8, VIDEO as u8, 0xf0, 0x00]);
            t.extend(es);
            let payload = Self::section(&t);
            self.packet(PMT, true, false, &payload);
        }

        fn pes(&mut self, pid: u16, pts: u64, key: bool, rai: bool) {
            let mut p = vec![0, 0, 1, if pid == VIDEO { 0xe0 } else { 0xc0 }, 0, 0, 0x80, 0x80, 5];
            p.push(0x21 | (((pts >> 30) & 0x07) as u8) << 1);
            p.push((pts >> 22) as u8);
            p.push(0x01 | (((pts >> 15) & 0x7f) as u8) << 1);
            p.push((pts >> 7) as u8);
            p.push(0x01 | ((pts & 0x7f) as u8) << 1);
            // Access unit delimiter, then an IDR or non-IDR slice.
            p.extend([0, 0, 0, 1, 0x09, 0xf0, 0, 0, 1, if key { 0x65 } else { 0x41 }]);
            p.resize(170, 0xaa);
            self.packet(pid, true, rai, &p);
            self.packet(pid, false, false, &[0xbb; 184]);
        }
    }

    /// 25 fps video with a keyframe every `gop` frames, AAC audio, PSI every second.
    fn stream(seconds: u32, gop: u32, rai: bool, start_pts: u64) -> Vec<u8> {
        let mut m = Mux::new();
        for f in 0..seconds * 25 {
            if f % 25 == 0 {
                m.pat();
                m.pmt(0x06, &[0x6a, 0x01, 0x00]); // DVB AC-3
            }
            let pts = start_pts + u64::from(f) * 3600;
            m.pes(VIDEO, pts & PTS_MASK, f % gop == 0, rai && f % gop == 0);
            if f % 2 == 0 {
                m.pes(AUDIO, pts & PTS_MASK, false, false);
            }
        }
        m.out
    }

    fn segment_all(data: &[u8], chunk: usize) -> (Segmenter, Vec<Segment>) {
        let mut s = Segmenter::new();
        let mut out = Vec::new();
        for c in data.chunks(chunk) {
            s.push(c);
            out.extend(s.take_segments());
        }
        (s, out)
    }

    #[test]
    fn cuts_two_second_segments_at_keyframes() {
        let data = stream(12, 50, true, 900_000);
        let (s, segs) = segment_all(&data, 1316);
        assert_eq!(s.target_duration(), Some(2));
        assert_eq!(segs.len(), 5, "12 s → five closed 2 s segments plus one open");
        for (i, seg) in segs.iter().enumerate() {
            assert_eq!(seg.sequence, i as u64);
            assert!((seg.duration - 2.0).abs() < 1e-9, "duration {}", seg.duration);
            assert_eq!(seg.data.len() % TS_PACKET, 0);
            // PAT then PMT first.
            assert_eq!(seg.data[1] & 0x1f, 0);
            assert_eq!(seg.data[2], 0);
            assert_eq!(((u16::from(seg.data[TS_PACKET + 1] & 0x1f)) << 8) | u16::from(seg.data[TS_PACKET + 2]), PMT);
        }
        assert_eq!(s.info().video.as_deref(), Some("H.264"));
        assert_eq!(s.info().audio, vec!["AC-3".to_string()]);
        assert!(s.info().unsupported().is_empty());
    }

    #[test]
    fn relabels_dvb_ac3_with_valid_crc() {
        let data = stream(4, 50, true, 0);
        let (_, segs) = segment_all(&data, 4096);
        let pmt = &segs[0].data[TS_PACKET..2 * TS_PACKET];
        let payload = if pmt[3] & 0x20 != 0 { &pmt[5 + pmt[4] as usize..] } else { &pmt[4..] };
        let section = section_after_pointer(payload).unwrap();
        let len = 3 + ((usize::from(section[1] & 0x0f) << 8) | usize::from(section[2]));
        assert_eq!(crc32_mpeg2(&section[..len]), 0, "CRC over section incl. CRC is zero");
        // Second ES entry (after 12-byte header + 5-byte video entry) is now ATSC AC-3.
        assert_eq!(section[12 + 5], 0x81);
    }

    #[test]
    fn groups_short_gops_and_detects_idr_without_rai() {
        // Startup uses individual 0.4 s GOPs, then groups them to 1.6 s. No RAI:
        // keyframes must be found by NAL type, including across input chunk boundaries.
        let data = stream(10, 10, false, 0);
        let (s, segs) = segment_all(&data, 188 * 3 + 17);
        assert_eq!(s.target_duration(), Some(2));
        assert!(segs.len() >= 5);
        for seg in &segs[..STARTUP_SEGMENTS] {
            assert!(
                (seg.duration - 0.4).abs() < 1e-9,
                "duration {}",
                seg.duration
            );
        }
        for seg in &segs[STARTUP_SEGMENTS..] {
            assert!(
                (seg.duration - 1.6).abs() < 1e-9,
                "duration {}",
                seg.duration
            );
        }
    }

    #[test]
    fn one_second_startup_then_two_second_segments_with_stable_target() {
        let mut s = Segmenter::new();
        let mut segments = Vec::new();
        for chunk in stream(10, 25, true, 0).chunks(1316) {
            s.push(chunk);
            let finished = s.take_segments();
            if !finished.is_empty() {
                assert_eq!(
                    s.target_duration(),
                    Some(2),
                    "target covers startup and steady playback"
                );
                segments.extend(finished);
            }
        }
        let durations: Vec<_> = segments.iter().map(|s| s.duration).collect();
        assert_eq!(durations, [1.0, 1.0, 1.0, 2.0, 2.0, 2.0]);
        for (i, segment) in segments.iter().enumerate() {
            assert_eq!(segment.sequence, i as u64);
            assert!(!segment.discontinuity);
            // Each segment has fresh PSI and starts with a keyframe PES.
            let payload = payload_of(&segment.data[2 * TS_PACKET..3 * TS_PACKET]);
            let (_, es_start) = pes_pts(payload).unwrap();
            assert!(has_keyframe_nal(&payload[es_start..], EsKind::H264));
        }
    }

    #[test]
    fn long_gops_stay_decodable_and_reconnect_restarts_short_segments() {
        let (s, segments) = segment_all(&stream(13, 100, true, 0), 4096);
        assert_eq!(s.target_duration(), Some(4));
        assert!(segments.iter().all(|s| s.duration == 4.0));

        let mut s = Segmenter::new();
        s.push(&stream(8, 25, true, PTS_MASK - 90_000));
        let before = s.take_segments();
        s.discontinuity();
        s.push(&stream(8, 25, true, 900_000));
        let after = s.take_segments();
        assert_eq!(after[0].sequence, before.last().unwrap().sequence + 1);
        assert!(after[0].discontinuity);
        assert_eq!(
            after[..3].iter().map(|s| s.duration).collect::<Vec<_>>(),
            [1.0; 3]
        );
        assert!(
            after[3..]
                .iter()
                .all(|s| s.duration == 2.0 && !s.discontinuity)
        );
    }

    #[test]
    fn audio_only_startup_is_one_second_then_normal_buffering() {
        let mut m = Mux::new();
        m.pat();
        let pmt = [
            0x02, 0xb0, 18, 0, 1, 0xc1, 0, 0, 0xe1, 1, 0xf0, 0, 0x0f, 0xe1, 1, 0xf0, 0,
        ];
        m.packet(PMT, true, false, &Mux::section(&pmt));
        for frame in 0..201 {
            m.pes(AUDIO, frame * 3600, false, false);
        }
        let (s, segments) = segment_all(&m.out, 999);
        assert_eq!(s.target_duration(), Some(2));
        assert_eq!(
            segments.iter().map(|s| s.duration).collect::<Vec<_>>(),
            [1.0, 1.0, 1.0, 2.0, 2.0]
        );
    }

    #[test]
    fn handles_pts_wrap_and_garbage_prefix() {
        let mut data = vec![0x12, 0x47, 0x00, 0x99]; // junk, including a false sync byte
        data.extend(stream(8, 50, true, PTS_MASK - 90_000));
        let (_, segs) = segment_all(&data, 999);
        assert!(segs.len() >= 3);
        assert!(segs.iter().all(|s| (s.duration - 2.0).abs() < 1e-9 && !s.discontinuity));
    }

    #[test]
    fn reconnect_flags_discontinuity() {
        let mut s = Segmenter::new();
        s.push(&stream(5, 50, true, 0));
        let before = s.take_segments();
        s.discontinuity();
        s.push(&stream(5, 50, true, 5_000_000));
        let after = s.take_segments();
        assert!(!before.is_empty() && !after.is_empty());
        assert!(after[0].discontinuity);
        assert!(!after[1..].iter().any(|x| x.discontinuity));
        assert_eq!(after[0].sequence, before.last().unwrap().sequence + 1);
    }

    #[test]
    fn playlist_rendering() {
        let entries = [
            PlaylistEntry { sequence: 7, duration: 2.0, discontinuity: false },
            PlaylistEntry { sequence: 8, duration: 2.04, discontinuity: true },
        ];
        let p = render_live_playlist(&entries, 2, 1);
        assert_eq!(
            p,
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:7\n\
             #EXT-X-DISCONTINUITY-SEQUENCE:1\n#EXTINF:2.000,\n7.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:2.040,\n8.ts\n"
        );
    }

    #[test]
    fn probes_ts_files() {
        let mut data = vec![0u8; 5]; // junk before the first packet
        data.extend(stream(6, 50, true, 1_000));
        let probe = probe_ts(&data).unwrap();
        assert_eq!(probe.sync_offset, 5);
        assert_eq!(probe.timing_pid, VIDEO);
        assert_eq!(probe.first_pts, 1_000);
        assert_eq!(probe.psi.len(), 2 * TS_PACKET);
        assert!(probe.patched_pmt.is_some(), "DVB AC-3 gets relabelled");
        let last = last_pts(&data[data.len() - 200 * TS_PACKET..], VIDEO).unwrap();
        assert!((pts_span(probe.first_pts, last) - (6.0 - 0.04)).abs() < 1e-9);

        let mut chunk = data[5..].to_vec();
        patch_pmt_packets(&mut chunk, probe.pmt_pid, probe.patched_pmt.as_ref().unwrap());
        let (_, segs) = segment_all(&chunk, 4096);
        assert!(!segs.is_empty());
        assert!(!looks_like_ts(b"\x1aE\xdf\xa3 not a transport stream"));
    }

    #[test]
    fn vod_playlist() {
        let p = render_vod_playlist(&[6.0, 6.0, 2.5]);
        assert!(p.contains("#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:6\n"));
        assert!(p.ends_with("#EXTINF:2.500,\n2.ts\n#EXT-X-ENDLIST\n"));
    }

    /// Optional check against a real capture: `FLUXTV_TS_SAMPLE=/path/sample.ts cargo test -p fluxtv-core real_sample -- --nocapture`
    #[test]
    fn real_sample() {
        let Ok(path) = std::env::var("FLUXTV_TS_SAMPLE") else { return };
        let data = std::fs::read(path).unwrap();
        let (s, segs) = segment_all(&data, 65_536);
        println!("info {:?} target {:?}", s.info(), s.target_duration());
        for seg in &segs {
            println!("#{} {:.3}s {} bytes", seg.sequence, seg.duration, seg.data.len());
        }
        assert!(!segs.is_empty());
    }
}
