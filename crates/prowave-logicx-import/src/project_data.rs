//! Scanners for the parts of `ProjectData` the reader decodes: channel strips, Audio Unit
//! descriptors, stock plug-in records, the Sampler's instrument reference and the Audio Unit
//! class-info property lists.
//!
//! `ProjectData` has no index, so every scanner walks the bytes once and checks each read.
//! A byte that does not fit a record's shape is skipped, never an error: the layouts are
//! reverse-engineered, and a project that a scanner does not recognise is still readable.
//! Only the root header is checked strictly, because a file shorter than its own header says
//! is truncated, and reading half a project would mislead.
//!
//! The record shapes come from the real Logic Pro 11 project studied in
//! `LOGICX-FORMAT-SPEC.md` ("What the real file shows").

use crate::model::{PluginFormat, PluginSlot, Track, TrackKind};
use crate::{Error, MAX_PLIST_BYTES};
use plist::{Dictionary, Value};

/// The root header of `ProjectData`: the magic, three words, and a `u32` payload length at
/// offset 16. The payload follows the header.
const ROOT_HEADER_BYTES: usize = 24;

/// The offset of the little-endian `u32` payload length in the root header.
const PAYLOAD_LENGTH_AT: usize = 16;

/// Most channel strips a project may hold. Logic allows 1000 tracks plus busses and aux.
const MAX_STRIPS: usize = 4096;

/// Most Audio Unit descriptors read from one `ProjectData`.
const MAX_AU_DESCRIPTORS: usize = 1024;

/// Most stock plug-in records read from one `ProjectData`.
const MAX_STOCK_RECORDS: usize = 1024;

/// Most class-info plists read from one `ProjectData`.
const MAX_CLASS_INFOS: usize = 256;

/// Most plug-ins attached to one strip. Logic's insert rack has 32 slots.
const MAX_PLUGINS_PER_TRACK: usize = 64;

/// The longest file name read after a Sampler tag.
const MAX_FILE_NAME_BYTES: usize = 256;

/// The Audio Unit type codes as they are stored: the four bytes in little-endian order. So
/// `umua` is `aumu` (instrument), `xfua` is `aufx` (effect), `fmua` is `aumf` (music effect)
/// and `imua` is `aumi` (MIDI processor).
const AU_TYPE_ANCHORS: [[u8; 4]; 4] = [*b"umua", *b"xfua", *b"fmua", *b"imua"];

/// The tag of a Sampler record, stored reversed: `PMAS` is `SAMP`.
const SAMPLER_TAG: &[u8; 4] = b"PMAS";

/// The tag that follows the Sampler tag in the record, stored reversed: `1MAS` is `SAM1`.
const SAMPLER_SUBTAG: &[u8; 4] = b"1MAS";

/// The tag of a stock plug-in record. Stock plug-ins are not Audio Units, so there is no
/// type code to anchor on; the literal ASCII `GAME` marks the record.
const STOCK_TAG: &[u8; 4] = b"GAME";

/// The start of an XML property list inside `ProjectData`.
const XML_START: &[u8] = b"<?xml";

/// The end of an XML property list.
const XML_END: &[u8] = b"</plist>";

/// The first four bytes of a Logic project file.
const MAGIC: [u8; 4] = [0x23, 0x47, 0xC0, 0xAB];

/// What the scanners found: the strips, the plug-ins and the instrument file references.
pub(crate) struct Assembly {
    /// Every channel strip, in file order.
    pub(crate) tracks: Vec<Track>,
    /// The plug-ins no active strip owns.
    pub(crate) unassigned: Vec<PluginSlot>,
}

/// Reads the whole of `data` into tracks and plug-ins. Errors only for a truncated root header.
pub(crate) fn assemble(data: &[u8], path: &str) -> Result<Assembly, Error> {
    check_root_header(data, path)?;

    let strips = channel_strips(data);
    let mut tracks: Vec<Track> = strips
        .iter()
        .map(|strip| Track {
            name: strip.name.clone(),
            kind: strip.kind,
            active: strip.active,
            volume_db: None,
            pan: None,
            mute: None,
            solo: None,
            plugins: Vec::new(),
            midi_regions: Vec::new(),
            audio_regions: Vec::new(),
            instrument_files: Vec::new(),
        })
        .collect();

    let class_infos = class_infos(data);
    // Every plug-in with the offset where it starts, so that it can be attached to the
    // strip that owns that position.
    let mut pending: Vec<(usize, PluginSlot)> = Vec::new();

    for descriptor in au_descriptors(data) {
        let state = class_infos
            .iter()
            .find(|dict| matches_descriptor(dict, &descriptor))
            .and_then(class_info_bytes);
        pending.push((
            descriptor.offset,
            PluginSlot {
                slot: 0,
                name: descriptor.name.unwrap_or_else(|| "Audio Unit".to_string()),
                vendor: None,
                format: PluginFormat::AudioUnit,
                au_type: Some(descriptor.kind),
                au_subtype: Some(descriptor.subtype),
                au_manufacturer: Some(descriptor.manufacturer),
                bypass: None,
                state,
            },
        ));
    }

    for record in stock_records(data) {
        pending.push((record.offset, stock_slot(record.name)));
    }

    let mut instrument_refs: Vec<(usize, String)> = Vec::new();
    for sampler in sampler_records(data) {
        pending.push((sampler.offset, stock_slot("Sampler".to_string())));
        if let Some(file) = sampler.file {
            instrument_refs.push((sampler.offset, file));
        }
    }

    pending.sort_by_key(|(offset, _)| *offset);
    let mut unassigned: Vec<PluginSlot> = Vec::new();
    for (offset, mut slot) in pending {
        match owner(&strips, offset) {
            Some(index) if strips.get(index).is_some_and(|strip| strip.active) => {
                match tracks.get_mut(index) {
                    Some(track) if track.plugins.len() < MAX_PLUGINS_PER_TRACK => {
                        slot.slot = track.plugins.len();
                        track.plugins.push(slot);
                    }
                    _ => push_unassigned(&mut unassigned, slot),
                }
            }
            _ => push_unassigned(&mut unassigned, slot),
        }
    }
    for (offset, file) in instrument_refs {
        if let Some(index) = owner(&strips, offset) {
            if strips.get(index).is_some_and(|strip| strip.active) {
                if let Some(track) = tracks.get_mut(index) {
                    track.instrument_files.push(file);
                }
            }
        }
    }

    Ok(Assembly { tracks, unassigned })
}

fn push_unassigned(unassigned: &mut Vec<PluginSlot>, mut slot: PluginSlot) {
    slot.slot = unassigned.len();
    unassigned.push(slot);
}

fn stock_slot(name: String) -> PluginSlot {
    PluginSlot {
        slot: 0,
        name,
        vendor: None,
        format: PluginFormat::Stock,
        au_type: None,
        au_subtype: None,
        au_manufacturer: None,
        bypass: None,
        state: None,
    }
}

/// The strip that owns the record at `offset`: the last strip that starts before it.
fn owner(strips: &[Strip], offset: usize) -> Option<usize> {
    strips
        .partition_point(|strip| strip.offset < offset)
        .checked_sub(1)
}

/// Checks the root header: the magic, the header length, and that the payload it declares is
/// all there. A file shorter than its declared payload was cut short.
pub(crate) fn check_root_header(data: &[u8], path: &str) -> Result<(), Error> {
    let actual = data.len() as u64;
    if data.len() < ROOT_HEADER_BYTES || data.get(..MAGIC.len()) != Some(&MAGIC[..]) {
        return Err(Error::Truncated {
            path: path.to_string(),
            expected: ROOT_HEADER_BYTES as u64,
            actual,
        });
    }
    let declared = data
        .get(PAYLOAD_LENGTH_AT..PAYLOAD_LENGTH_AT + 4)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0);
    let expected = ROOT_HEADER_BYTES as u64 + u64::from(declared);
    if actual < expected {
        return Err(Error::Truncated {
            path: path.to_string(),
            expected,
            actual,
        });
    }
    Ok(())
}

// --- channel strips -------------------------------------------------------------------------

/// A channel strip record: the name field and the descriptor that follows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Strip {
    /// The offset of the record's leading space.
    pub(crate) offset: usize,
    pub(crate) name: String,
    pub(crate) kind: TrackKind,
    pub(crate) active: bool,
}

/// Finds the channel strips. A record is a NUL byte, then a 16-byte field that opens with a
/// space and holds a NUL-padded printable name, then an 8-byte descriptor.
pub(crate) fn channel_strips(data: &[u8]) -> Vec<Strip> {
    let mut strips = Vec::new();
    for offset in 1..data.len() {
        if strips.len() >= MAX_STRIPS {
            break;
        }
        if data.get(offset - 1) != Some(&0) {
            continue;
        }
        let Some(field) = data.get(offset..offset + 16) else {
            break;
        };
        let Some(name) = strip_name(field) else {
            continue;
        };
        let Some(descriptor) = data.get(offset + 16..offset + 24) else {
            continue;
        };
        let (kind, active) = classify(descriptor);
        strips.push(Strip {
            offset,
            name,
            kind,
            active,
        });
    }
    strips
}

/// The name of a channel strip field: a leading space, then a NUL-padded printable name.
fn strip_name(field: &[u8]) -> Option<String> {
    let (&lead, body) = field.split_first()?;
    if lead != b' ' {
        return None;
    }
    padded_name(body)
}

/// A NUL-padded printable name: the bytes up to the first NUL, then nothing but NULs. A
/// name of spaces only is refused.
pub(crate) fn padded_name(field: &[u8]) -> Option<String> {
    let length = field
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(field.len());
    let (name, padding) = field.split_at(length);
    if name.is_empty() || padding.iter().any(|&byte| byte != 0) {
        return None;
    }
    if !name.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
        return None;
    }
    if name.iter().all(|&byte| byte == b' ') {
        return None;
    }
    String::from_utf8(name.to_vec()).ok()
}

/// The kind and the in-use flag of a strip, from its 8-byte descriptor.
///
/// The head byte picks the kind. `0xAB` is an aux when byte 1 is `0xF5`, else audio; `0x29`
/// is an instrument when byte 2 is `0xF3` or `0xF7`, else an input. Byte 4 being non-zero
/// marks a strip in use: on the real project that is exactly the three tracks its metadata
/// counts. Byte 2 bit 2 is set on busses and the master too, so it does not mark use.
fn classify(descriptor: &[u8]) -> (TrackKind, bool) {
    let byte = |index: usize| descriptor.get(index).copied().unwrap_or(0);
    let head = byte(0);
    let kind = match head {
        0x89 => TrackKind::Master,
        0x49 => TrackKind::Output,
        0xE9 => TrackKind::Bus,
        0xAB if byte(1) == 0xF5 => TrackKind::Aux,
        0xAB => TrackKind::Audio,
        0x29 if byte(2) == 0xF3 || byte(2) == 0xF7 => TrackKind::Instrument,
        0x29 => TrackKind::Input,
        other => TrackKind::Unknown(other),
    };
    (kind, byte(4) != 0)
}

// --- Audio Unit descriptors ------------------------------------------------------------------

/// An Audio Unit's four-character codes and its name, as the project stores them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AuDescriptor {
    /// The offset of the type code.
    pub(crate) offset: usize,
    /// The name stored in the 12 bytes before the manufacturer code, when printable.
    pub(crate) name: Option<String>,
    pub(crate) kind: [u8; 4],
    pub(crate) subtype: [u8; 4],
    pub(crate) manufacturer: [u8; 4],
}

/// Finds the Audio Unit descriptors: a type code, with the manufacturer code just before it
/// and the subtype just after it, all four bytes in little-endian order. Each code must be
/// printable ASCII, and the name sits in the 12 bytes before the manufacturer code.
pub(crate) fn au_descriptors(data: &[u8]) -> Vec<AuDescriptor> {
    let mut found = Vec::new();
    for offset in 4..data.len() {
        if found.len() >= MAX_AU_DESCRIPTORS {
            break;
        }
        let Some(anchor) = data.get(offset..offset + 4) else {
            break;
        };
        if !AU_TYPE_ANCHORS
            .iter()
            .any(|known| known.as_slice() == anchor)
        {
            continue;
        }
        let Some(manufacturer) = data.get(offset - 4..offset).and_then(stored_fourcc) else {
            continue;
        };
        let Some(subtype) = data.get(offset + 4..offset + 8).and_then(stored_fourcc) else {
            continue;
        };
        let Some(kind) = stored_fourcc(anchor) else {
            continue;
        };
        let name = offset
            .checked_sub(16)
            .and_then(|start| data.get(start..offset - 4))
            .and_then(padded_name);
        found.push(AuDescriptor {
            offset,
            name,
            kind,
            subtype,
            manufacturer,
        });
    }
    found
}

/// Reverses four stored bytes into the code they spell. Refuses anything that is not four
/// printable bytes.
fn stored_fourcc(bytes: &[u8]) -> Option<[u8; 4]> {
    let mut code: [u8; 4] = bytes.try_into().ok()?;
    if !code.iter().all(|byte| (0x20..=0x7e).contains(byte)) {
        return None;
    }
    code.reverse();
    Some(code)
}

// --- stock plug-ins and the Sampler ----------------------------------------------------------

/// A stock plug-in record: the name in the 12 bytes before the literal `GAME` tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StockRecord {
    /// The offset of the `GAME` tag.
    pub(crate) offset: usize,
    pub(crate) name: String,
}

/// Finds the stock plug-in records. Two flag bytes sit 14 and 13 bytes before the tag: byte
/// 14 is `0x02` (an insert) or `0x00` (an instrument), and byte 13 is `0x02` (an insert or an
/// instrument) or `0x01` (the Klopfgeist metronome). Other tag occurrences are not records.
pub(crate) fn stock_records(data: &[u8]) -> Vec<StockRecord> {
    let mut found = Vec::new();
    for offset in 14..data.len() {
        if found.len() >= MAX_STOCK_RECORDS {
            break;
        }
        if data.get(offset..offset + 4) != Some(&STOCK_TAG[..]) {
            continue;
        }
        let (Some(&first), Some(&second)) = (data.get(offset - 14), data.get(offset - 13)) else {
            continue;
        };
        if !matches!(first, 0x00 | 0x02) || !matches!(second, 0x01 | 0x02) {
            continue;
        }
        let Some(name) = data.get(offset - 12..offset).and_then(padded_name) else {
            continue;
        };
        found.push(StockRecord { offset, name });
    }
    found
}

/// A Sampler instrument reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SamplerRecord {
    /// The offset of the Sampler tag.
    pub(crate) offset: usize,
    /// The instrument file the sampler loads, such as `Classic Bass Finger.exs`.
    pub(crate) file: Option<String>,
}

/// Finds the Sampler records by their tag (`PMAS`, which is `SAMP` reversed). The instrument
/// file name follows the `1MAS` sub-tag, NUL-terminated.
pub(crate) fn sampler_records(data: &[u8]) -> Vec<SamplerRecord> {
    let mut found = Vec::new();
    for offset in 0..data.len() {
        if data.get(offset..offset + 4) != Some(&SAMPLER_TAG[..]) {
            continue;
        }
        let start = if data.get(offset + 4..offset + 8) == Some(&SAMPLER_SUBTAG[..]) {
            offset + 8
        } else {
            offset + 4
        };
        let file = data.get(start..).and_then(|rest| {
            let window = rest.get(..rest.len().min(MAX_FILE_NAME_BYTES))?;
            let end = window
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(window.len());
            let name = String::from_utf8(window.get(..end)?.to_vec()).ok()?;
            let printable = name.bytes().all(|byte| (0x20..=0x7e).contains(&byte));
            (printable && name.ends_with(".exs")).then_some(name)
        });
        found.push(SamplerRecord { offset, file });
        if found.len() >= MAX_STOCK_RECORDS {
            break;
        }
    }
    found
}

// --- Audio Unit class info -------------------------------------------------------------------

/// Every XML property list in the file that parses as a dictionary, capped by size and count.
pub(crate) fn class_infos(data: &[u8]) -> Vec<Dictionary> {
    let mut found = Vec::new();
    let mut from = 0usize;
    while found.len() < MAX_CLASS_INFOS {
        let Some(rest) = data.get(from..) else {
            break;
        };
        let Some(relative) = find(rest, XML_START) else {
            break;
        };
        let start = from + relative;
        let Some(window) = data.get(start..(start + MAX_PLIST_BYTES).min(data.len())) else {
            break;
        };
        if let Some(end) = find(window, XML_END) {
            if let Some(slice) = window.get(..end + XML_END.len()) {
                if let Ok(Value::Dictionary(dict)) = plist::from_bytes::<Value>(slice) {
                    found.push(dict);
                }
            }
        }
        from = start + XML_START.len();
    }
    found
}

/// Whether a class-info dictionary belongs to the Audio Unit: its `type`, `subtype` and
/// `manufacturer` integers are the four codes, read big-endian.
fn matches_descriptor(dict: &Dictionary, descriptor: &AuDescriptor) -> bool {
    fourcc_value(dict, "type") == Some(descriptor.kind)
        && fourcc_value(dict, "subtype") == Some(descriptor.subtype)
        && fourcc_value(dict, "manufacturer") == Some(descriptor.manufacturer)
}

/// A four-character code stored as an integer, read big-endian.
fn fourcc_value(dict: &Dictionary, key: &str) -> Option<[u8; 4]> {
    let Some(Value::Integer(value)) = dict.get(key) else {
        return None;
    };
    let number = value.as_unsigned().or_else(|| {
        value
            .as_signed()
            .and_then(|signed| u64::try_from(signed).ok())
    })?;
    u32::try_from(number).ok().map(u32::to_be_bytes)
}

/// The class-info dictionary as an XML property list, ready for
/// `AudioUnitSetProperty(kAudioUnitProperty_ClassInfo)`.
///
/// Logic stores an Audio Unit's state under a vendor key instead of the standard `data` key:
/// FabFilter's is `FabFilterPluginState`, a binary `FFBS` blob. This moves that blob into
/// `data`, so the dictionary has the shape the Audio Unit API expects. Whether the vendor
/// accepts its own blob under `data` is not checked here.
fn class_info_bytes(dict: &Dictionary) -> Option<Vec<u8>> {
    let mut dict = dict.clone();
    if !dict.contains_key("data") {
        let key = dict
            .iter()
            .find(|(_, value)| matches!(value, Value::Data(_)))
            .map(|(key, _)| key.clone())?;
        let blob = dict.remove(&key)?;
        dict.insert("data".to_string(), blob);
    }
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &Value::Dictionary(dict)).ok()?;
    Some(bytes)
}

/// The first position of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A channel strip record: NUL, then the 16-byte name field, then the descriptor.
    fn strip_record(name: &str, descriptor: [u8; 8]) -> Vec<u8> {
        let mut record = vec![0u8];
        let mut field = vec![b' '];
        field.extend_from_slice(name.as_bytes());
        field.resize(16, 0);
        record.extend_from_slice(&field);
        record.extend_from_slice(&descriptor);
        record
    }

    #[test]
    fn a_channel_strip_gives_its_name_kind_and_flag() {
        let mut data = vec![0xAAu8; 3];
        data.extend(strip_record(
            "Audio 1",
            [0xAB, 0xF7, 0xD3, 0xCF, 0x08, 0, 0, 0],
        ));
        data.extend(strip_record("Aux 1", [0xAB, 0xF5, 0xD7, 0xCF, 0, 0, 0, 0]));
        data.extend(strip_record(
            "Inst 1",
            [0x29, 0xF5, 0xF7, 0xCF, 0x08, 0, 0, 0],
        ));
        data.extend(strip_record("Inst 2", [0x29, 0xF5, 0xF3, 0xCF, 0, 0, 0, 0]));
        data.extend(strip_record("Bus 1", [0xE9, 0xF5, 0xD7, 0xCD, 0, 0, 0, 0]));
        data.extend(strip_record("Master", [0x89, 0x75, 0xDD, 0xC5, 0, 0, 0, 0]));
        data.extend(strip_record(
            "Output 1-2",
            [0x49, 0xF7, 0xDF, 0xED, 0, 0, 0, 0],
        ));
        let strips = channel_strips(&data);
        let summary: Vec<(String, TrackKind, bool)> = strips
            .iter()
            .map(|s| (s.name.clone(), s.kind, s.active))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Audio 1".to_string(), TrackKind::Audio, true),
                ("Aux 1".to_string(), TrackKind::Aux, false),
                ("Inst 1".to_string(), TrackKind::Instrument, true),
                ("Inst 2".to_string(), TrackKind::Instrument, false),
                ("Bus 1".to_string(), TrackKind::Bus, false),
                ("Master".to_string(), TrackKind::Master, false),
                ("Output 1-2".to_string(), TrackKind::Output, false),
            ]
        );
    }

    #[test]
    fn an_unknown_head_byte_is_kept_and_a_bad_name_is_skipped() {
        let mut data = strip_record("Weird", [0x77, 0, 0, 0, 0, 0, 0, 0]);
        // A name with a byte after the padding is not a strip.
        let mut bad = vec![0u8, b' '];
        bad.extend_from_slice(b"Bad");
        bad.resize(17, 0);
        bad[10] = b'x';
        bad.extend_from_slice(&[0; 8]);
        data.extend(bad);
        let strips = channel_strips(&data);
        assert_eq!(strips.len(), 1);
        assert_eq!(strips[0].kind, TrackKind::Unknown(0x77));
    }

    #[test]
    fn padded_names_must_be_printable_and_nul_padded() {
        assert_eq!(
            padded_name(b"Pro-Q 4\0\0\0\0\0").as_deref(),
            Some("Pro-Q 4")
        );
        assert_eq!(padded_name(b"Pro\0\0\x01\0"), None);
        assert_eq!(padded_name(b"\0\0\0"), None);
        assert_eq!(padded_name(b"   \0\0"), None);
    }

    /// A 16-byte-free Audio Unit record: name, manufacturer, type, subtype.
    fn au_record(
        name: &[u8; 12],
        manufacturer: &[u8; 4],
        kind: &[u8; 4],
        subtype: &[u8; 4],
    ) -> Vec<u8> {
        let mut bytes = name.to_vec();
        let stored = |code: &[u8; 4]| {
            let mut reversed = *code;
            reversed.reverse();
            reversed
        };
        bytes.extend_from_slice(&stored(manufacturer));
        bytes.extend_from_slice(&stored(kind));
        bytes.extend_from_slice(&stored(subtype));
        bytes
    }

    #[test]
    fn an_audio_unit_descriptor_gives_its_codes_and_name() {
        // Stored little-endian: `FbaF` is `FabF`, `fmua` is `aumf`, `p4QF` is `FQ4p`.
        let mut data = vec![0u8; 2];
        data.extend(au_record(b"Pro-Q 4\0\0\0\0\0", b"FabF", b"aumf", b"FQ4p"));
        let descriptors = au_descriptors(&data);
        assert_eq!(descriptors.len(), 1);
        let descriptor = &descriptors[0];
        assert_eq!(descriptor.name.as_deref(), Some("Pro-Q 4"));
        assert_eq!(&descriptor.manufacturer, b"FabF");
        assert_eq!(&descriptor.kind, b"aumf");
        assert_eq!(&descriptor.subtype, b"FQ4p");
    }

    #[test]
    fn a_stock_record_needs_its_flags_and_a_name() {
        let mut data = vec![0x02, 0x02];
        data.extend_from_slice(b"SmpleDly\0\0\0\0");
        data.extend_from_slice(STOCK_TAG);
        // Flags 0x00 0x00 are not a record.
        data.extend_from_slice(&[0x00, 0x00]);
        data.extend_from_slice(b"Other\0\0\0\0\0\0\0");
        data.extend_from_slice(STOCK_TAG);
        let records = stock_records(&data);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].name, "SmpleDly");
        assert_eq!(records[0].offset, 14);
    }

    #[test]
    fn a_sampler_reference_gives_the_exs_file() {
        let mut data = b"junk".to_vec();
        data.extend_from_slice(b"PMAS1MAS");
        data.extend_from_slice(b"Classic Bass Finger.exs\0\0\0");
        let records = sampler_records(&data);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].file.as_deref(), Some("Classic Bass Finger.exs"));
    }

    #[test]
    fn a_class_info_plist_matches_its_descriptor_and_moves_the_blob() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>FabFilterPluginState</key><data>RkZCUwEAAAA=</data>
<key>manufacturer</key><integer>1180787270</integer>
<key>name</key><string>Default Setting</string>
<key>subtype</key><integer>1179726960</integer>
<key>type</key><integer>1635085670</integer>
<key>version</key><integer>0</integer>
</dict></plist>"#;
        let mut data = vec![0u8; 4];
        data.extend(au_record(b"Pro-Q 4\0\0\0\0\0", b"FabF", b"aumf", b"FQ4p"));
        data.extend_from_slice(xml);
        let descriptor = au_descriptors(&data).remove(0);
        let classes = class_infos(&data);
        assert_eq!(classes.len(), 1);
        assert!(matches_descriptor(&classes[0], &descriptor));
        let Some(state) = class_info_bytes(&classes[0]) else {
            panic!("the class info has a state");
        };
        let Ok(parsed) = plist::from_bytes::<Value>(&state) else {
            panic!("the state is a plist");
        };
        let Some(dict) = parsed.as_dictionary() else {
            panic!("the state is a dictionary");
        };
        assert!(dict.get("data").is_some());
        assert!(dict.get("FabFilterPluginState").is_none());
    }

    #[test]
    fn owners_are_the_last_strip_before_a_plugin() {
        let strips = vec![
            Strip {
                offset: 10,
                name: "A".into(),
                kind: TrackKind::Audio,
                active: true,
            },
            Strip {
                offset: 50,
                name: "B".into(),
                kind: TrackKind::Audio,
                active: true,
            },
        ];
        assert_eq!(owner(&strips, 5), None);
        assert_eq!(owner(&strips, 10), None);
        assert_eq!(owner(&strips, 11), Some(0));
        assert_eq!(owner(&strips, 60), Some(1));
    }

    #[test]
    fn a_truncated_root_header_is_an_error() {
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&[0; 20]);
        data[PAYLOAD_LENGTH_AT..PAYLOAD_LENGTH_AT + 4].copy_from_slice(&100u32.to_le_bytes());
        assert!(matches!(
            check_root_header(&data, "p"),
            Err(Error::Truncated {
                expected: 124,
                actual: 24,
                ..
            })
        ));
        data.resize(124, 0);
        assert!(check_root_header(&data, "p").is_ok());
        assert!(matches!(
            check_root_header(&data[..10], "p"),
            Err(Error::Truncated { .. })
        ));
    }

    /// A deterministic pseudo-random byte source, so the mutation test is repeatable.
    fn noise(seed: u64) -> impl FnMut() -> u8 {
        let mut state = seed | 1;
        move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        }
    }

    #[test]
    fn random_bytes_never_panic_the_scanners() {
        let mut next = noise(0x5EED);
        for _ in 0..200 {
            let mut data = MAGIC.to_vec();
            data.extend_from_slice(&[0, 0, 0, 0]);
            data.extend_from_slice(&[0; 4]);
            data.extend_from_slice(&[0; 4]);
            let length = 1 + usize::from(next()) * 4;
            data.extend_from_slice(&(length as u32).to_le_bytes());
            data.extend_from_slice(&[0; 4]);
            for _ in 0..length {
                data.push(next());
            }
            let _ = assemble(&data, "random");
        }
    }
}
