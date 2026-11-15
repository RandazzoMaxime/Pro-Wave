//! The musical content of a project: tracks, their plug-ins and their regions.
//!
//! Every field that the reader could not decode is an `Option` or an empty `Vec`, never a
//! guess. The notes on each type say what the reader fills in and what it leaves empty.

use crate::Project;

/// What a channel strip is, as its descriptor says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    /// A software instrument track: MIDI in, an instrument plug-in in its first slot.
    Instrument,
    /// An audio track.
    Audio,
    /// A hardware input strip.
    Input,
    /// An aux (send return) channel.
    Aux,
    /// A bus: a summing or routing channel.
    Bus,
    /// An output strip, such as `Output 1-2`.
    Output,
    /// The master strip.
    Master,
    /// A descriptor head the reader does not know. The head byte is kept.
    Unknown(u8),
}

/// Where a plug-in comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginFormat {
    /// A third-party or Apple Audio Unit, identified by its four-character codes.
    AudioUnit,
    /// A stock Logic plug-in (Sample Delay, the Sampler, the metronome), identified by name.
    Stock,
    /// A plug-in the reader saw but could not classify.
    Unknown,
}

/// One insert slot of a track, or a plug-in the reader could not attach to a track.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginSlot {
    /// The position of the plug-in among the plug-ins attached to the same track, from zero.
    pub slot: usize,
    /// The name the project stores for the plug-in (the Audio Unit name, or the stock name).
    pub name: String,
    /// The vendor name. Logic stores only the four-character manufacturer code, so this is
    /// always `None` for now.
    pub vendor: Option<String>,
    /// Where the plug-in comes from.
    pub format: PluginFormat,
    /// The Audio Unit type code (`aumu`, `aufx`, `aumf` or `aumi`), in reading order.
    pub au_type: Option<[u8; 4]>,
    /// The Audio Unit subtype code, in reading order.
    pub au_subtype: Option<[u8; 4]>,
    /// The Audio Unit manufacturer code, in reading order (`FabF` for FabFilter).
    pub au_manufacturer: Option<[u8; 4]>,
    /// Whether the slot is bypassed. The bypass flag is not decoded yet, so this is `None`.
    pub bypass: Option<bool>,
    /// The state as an Audio Unit class-info property list (an XML plist with the keys `type`,
    /// `subtype`, `manufacturer`, `version`, `name` and `data`). Hand it to
    /// `AudioUnitSetProperty(kAudioUnitProperty_ClassInfo)` after parsing. `None` for stock
    /// plug-ins and for Audio Units whose state the project does not store.
    pub state: Option<Vec<u8>>,
}

/// One note of a MIDI region. The pitch, velocity and times were read from the note records of
/// `ProjectData` and matched against the piano roll the user showed (three notes, B2, D3, E3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiNote {
    /// The MIDI note number, 0 to 127.
    pub pitch: u8,
    /// The MIDI velocity, 1 to 127.
    pub velocity: u8,
    /// The start in ticks at 960 pulses per quarter note.
    pub start_tick: u64,
    /// The length in ticks at 960 pulses per quarter note.
    pub length_ticks: u64,
}

/// A MIDI region on a track: the sequence the track plays. Its notes are read from the event
/// records that follow the sequence. The region's own start and length are not located yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiRegion {
    /// The region name (the track name, as Logic names a region after its track).
    pub name: String,
    /// The start in ticks. Not located yet, so `None`.
    pub start_tick: Option<u64>,
    /// The length in ticks. Not located yet, so `None`.
    pub length_ticks: Option<u64>,
    /// The notes, sorted by start. Their times are relative to the origin the note records are
    /// measured from (see [`crate::FullProject::ticks_per_quarter`] and `LOGICX-FORMAT-SPEC.md`).
    pub notes: Vec<MidiNote>,
}

/// An audio region record (`gRuA` in `ProjectData`). The name and the audio file are read.
/// The start and the length are not located in the record yet, so they are `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioRegion {
    /// The region name as the record stores it (`Inst 1_bip`, or `Inst 1_bip.1` for a second
    /// region on the same file).
    pub name: String,
    /// The file in `Media/Audio Files/` whose name the region shares, from `MetaData.plist`.
    /// `None` when no listed file matches.
    pub file: Option<String>,
    /// The start in ticks at 960 pulses per quarter note. Not located yet.
    pub start_tick: Option<u64>,
    /// The length in ticks at 960 pulses per quarter note. Not located yet.
    pub length_ticks: Option<u64>,
}

/// One channel strip of the project.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// The strip name, such as `Inst 1` or `Bus 12`.
    pub name: String,
    /// What the strip is.
    pub kind: TrackKind,
    /// Whether the strip is in use. Set when descriptor byte 4 is non-zero. This is a guess
    /// from one project: it marks the three tracks the metadata counts (`NumberOfTracks=3`),
    /// but its meaning is not confirmed.
    pub active: bool,
    /// The fader level in decibels. `None`: the byte below is read, but its curve to dB is not
    /// known yet.
    pub volume_db: Option<f32>,
    /// The fader byte of the channel record (offset 121 of the strip's `OCuA` record). It is 90
    /// on the default strips (0 dB on the project the reader was built against) and 100 and 117
    /// on the two strips the user moved.
    pub fader_raw: Option<u8>,
    /// The pan in Logic's scale, -64 (hard left) to +63 (hard right), read from byte 125 of the
    /// channel record. The default strips hold 64, which the reader maps to 0: centre.
    pub pan: Option<f32>,
    /// Whether the strip is muted. Not decoded yet, so `None`.
    pub mute: Option<bool>,
    /// Whether the strip is soloed. Not decoded yet, so `None`.
    pub solo: Option<bool>,
    /// The insert plug-ins of the strip, in slot order.
    pub plugins: Vec<PluginSlot>,
    /// The MIDI regions of the strip. Empty until the note layout is verified.
    pub midi_regions: Vec<MidiRegion>,
    /// The instrument files the strip's sampler refers to, such as `Classic Bass Finger.exs`.
    pub instrument_files: Vec<String>,
}

/// A whole project: the bundle's metadata, every channel strip, the plug-ins that could not be
/// attached to a strip, and the audio regions the file holds.
#[derive(Clone, Debug, PartialEq)]
pub struct FullProject {
    /// The metadata the bundle envelope gives (see [`crate::read`]).
    pub info: Project,
    /// Every channel strip, in the order the project stores them.
    pub tracks: Vec<Track>,
    /// Plug-ins that belong to no active strip, such as the metronome.
    pub unassigned_plugins: Vec<PluginSlot>,
    /// Every audio region record, in file order. Their owning track is not decoded.
    pub audio_regions: Vec<AudioRegion>,
    /// The ticks per quarter note of the note and region times. The file's note times fit 240
    /// per beat (against the piano roll the user showed), not Logic's usual 960. This value is
    /// inferred from that fit, not read from a field of the file.
    pub ticks_per_quarter: u32,
}
