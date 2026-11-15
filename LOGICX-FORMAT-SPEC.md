# Pro Wave — Official Logic Pro .logicx Support Spec
**Target: Claude must implement this 100% in pure Rust | File: `crates/prowave-logicx-import`**
> This spec allows ProWave to OPEN and SAVE official Apple Logic Pro .logicx projects on Linux and macOS, while being 10-20x lighter than Logic Pro.

## 1. Why This Matters & Lightweight Goal

Logic Pro official is ~1.2GB app + 70GB library, Objective-C++ legacy, 3 sec cold start, 1.5GB RAM idle, macOS only.
ProWave goal:
- **Binary size:** <50MB single binary (vs 1.2GB)
- **RAM idle:** <150MB (vs 1.5GB)
- **Cold start:** <400ms (vs 3-4 sec) — achieved by no NSKeyedArchiver, no CoreData, lazy loading
- **Linux support:** No CoreAudio, no AU, only ALSA/JACK/PipeWire + CLAP/VST3, Vulkan via wgpu
- **macOS support:** CoreAudio + AU + Metal via wgpu, universal binary arm64/x86_64, Hardened Runtime, notarized
- **Pure Rust:** `#![forbid(unsafe_code)]` except FFI shims, no Objective-C runtime for parsing

## 2. What is a .logicx File?

A `.logicx` is NOT a file. It's a **macOS bundle** — a directory that Finder shows as one file.

```
MySong.logicx/  (actually a directory)
├── Alternatives/
│   ├── 000/                          # Alternative 0 = default arrangement
│   │   ├── ProjectData               # <<< MAIN BINARY BLOB — 1-50 MB, undocumented
│   │   ├── MetaData.plist            # BPM, key, time signature, sample rate (binary or XML plist)
│   │   ├── DisplayState.plist        # Window positions, NOT needed
│   │   ├── DisplayStateArchive       # Binary UI state, NOT needed
│   │   ├── WindowImage.jpg           # Screenshot, NOT needed
│   │   └── Undo Data.nosync/         # Undo history, ignore
│   ├── 001/, 002/...                 # Other alternatives (versions of arrangement)
│   └── ...
├── Resources/
│   └── ProjectInformation.plist      # Which alternative is active, Logic version
└── Media/
    └── Audio Files/                  # WAV/AIFF/CAF recorded audio
```

Package vs Folder: Logic can save as Package (single .logicx) or Folder (plain folder with .logicx sub-bundle). ProWave must support both.

### 2.1 Plists — Easy Part

Two plists are standard Apple plists (XML or binary). Parse with `plist` crate in Rust (pure Rust, no Foundation).

- `MetaData.plist`: Contains `tempo`, `keySignature`, `timeSignature`, `sampleRate`, `channelCount`
- `ProjectInformation.plist`: Contains `activeAlternative` (e.g., 0), `logicVersion` (e.g., 11.0)

**Rust:**
```rust
use plist::Value;
let metadata = Value::from_file("Alternatives/000/MetaData.plist")?;
let tempo = metadata.as_dictionary().unwrap()["tempo"].as_real().unwrap();
```

### 2.2 ProjectData — Hard Part — Undocumented Binary Blob

This is where all musical data lives. No documentation from Apple. Based on reverse engineering from `lpx-explorer`, `logicx-analyzer`, `LogicProFormatWriter`.

**Key facts:**
- **Magic:** First 4 bytes `23 47 C0 AB` (file signature) — see `logicx-analyzer`[^1]
- **Endianness:** Mixed, primarily Big-Endian for lengths, Little-Endian for FourCC codes
- **Structure:** Chunk-based like IFF/RIFF, NO central header table. Must linear scan entire file. No seek table.
- **Size:** 1-50 MB, read-only. Never write directly without re-synthesizing via seed method.

#### Chunk Markers — Reversed FourCC

Apple stores FourCC reversed in file. Example: Track marker stored as `karT` = "Trak" backwards.

Known markers (from `logicx-analyzer`):

| Stored (LE in file) | Reversed = Display | Purpose | Frequency in typical project |
|---|---|---|---|
| `karT` | Trak | Track definition | ~320 |
| `qeSM` | MSeq | MIDI sequences | ~169 |
| `qSvE` | EvSq | Event sequences (automation) | ~169 |
| `gRuA` | AuRg | Audio regions | ~38 |
| `tSxT` | TxSt | Text/notation styles | ~32 |
| `LFUA` / `lFuA` | AUFL/AuFl | Audio file references | ~23 |
| `PMOC` | COMP | Comping/take data | ~23 |
| `tSnI` | InSt | Instrument definition | ~1 |

**Rust scanning pattern (must be linear, no assumptions):**
```rust
const KNOWN_MARKERS: &[&[u8;4]] = &[b"karT", b"gRuA", b"qeSM", b"qSvE"];
for i in 0..data.len()-4 {
    let marker = &data[i..i+4];
    if KNOWN_MARKERS.contains(&marker.try_into().unwrap()) {
        // found chunk at offset i
    }
}
```
Guard against false positives: check printable ASCII around marker, length caps, neighboring-byte invariants.

#### 3 Distinct Storage Shapes Inside ProjectData

**Shape 1: AU Component Descriptors — 3x FourCC Triple (for 3rd party plugins)**

Anchor: AU type 4CC stored little-endian, one of `umua` (aumu), `xfua` (aufx), `fmua` (aumf), `imua` (aumi)

Layout relative to type anchor T:
```
T-4: Manufacturer 4CC (4 bytes LE)
T:   Type 4CC (4 bytes LE)  // e.g., umua = instrument
T+4: Subtype 4CC (4 bytes LE)
```
- All three must be printable ASCII 0x20-0x7e or discard (guard against noise)
- Reverse before display: `umua` -> `aumu`
- Fingerprint: `"{type}/{subtype}/{manufacturer}"` e.g., `aumu/Synt/appl`
- Trailing spaces significant: Kilohearts manufacturer `"kHs "` has space
- Types: `aumu` = MusicDevice (instrument), `aufx` = effect, `aumf` = MIDI effect, `aumi` = MIDI processor

**Shape 2: Apple Stock Plug-ins — GAME Anchor (Compressor, ChromaVerb, Alchemy...)**

Apple stock plugins NOT stored as AU triple. Anchored on literal ASCII `GAME`.

Layout relative to G of GAME:
```
-14: flag1 (0x02 = FX insert -> aufx, 0x00 = instrument -> aumu, else reject)
-13: flag2 (0x02 = FX/instrument, 0x01 = Klopfgeist metronome, else reject)
-12..-1: 12-byte ASCII name, NUL-padded, must be NUL-padded not garbage
0..3: GAME marker
```
- Name examples: "Compressor", "Channel EQ", "SpaceDesigner"
- Fingerprint not recoverable from storage, so:
  - Lookup in static table `STOCK_FINGERPRINTS`: `Compressor -> aufx/Comp/appl`
  - Fallback: synthesize subtype from name lowercased, strip non-alphanumeric, pad to 4 chars
- Display name = recovered name

**Shape 3: Drummer / Bass Player / Keyboard Player — Embedded JSON**

Logic's Session Players store full JSON state inside ProjectData.

Anchor: literal string `"selectedPersistentCharacterTypeIdentifier":"`

Value read until next `"` within 96 bytes, must start with `Type_`.

Example:
```json
{
  "Preset": {"Name": "Sweet Memories", "CharacterIdentifier": "Acoustic Piano - Strummed"},
  "RegionType": "Type_AcousticPianoV2",
  "GeneratorMemento": {"Seeds": [...], "MementoParameters": {...}}
}
```
- One slot emits MANY snapshots (per-region). Cluster by offset proximity: `CLUSTER_THRESHOLD = 32768` bytes. Two hits >32KB apart = new cluster. Latest Type in cluster = active.
- Known mapping: `Type_AcousticDrummerV2` -> `Drummer`, `Type_ElectricBassV2` -> `Bass Player`

**Shape 4: Channel-strip / Track Records**

Anchor: 16-byte name field opening with `0x20` (space) preceded by NUL `0x00`, followed by 8-byte descriptor.

Layout:
```
N-1: 0x00
N: 0x20 + ascii-name (printable 0x21-0x7e) NUL-padded to 16 bytes, whitespace-only rejected
N+16: descriptor 8 bytes
  descriptor[0] = head: 0x89=Master, 0x49=Output, 0xE9=Bus, 0xAB with b1=0xF5=Aux else Audio, 0x29 with b2=0xF3/0xF7=Instrument else Input
  descriptor[2] & 0x04 !=0 or descriptor[4]!=0 => is_active (plugin loaded or routing)
```

**Shape 5: Region Records — 0x61 0xFF Anchor (Audio Region Names)**

User track names like "Acoustic GTR" live in audio region records, not channel-strip.

Layout:
```
[4 bytes opaque id]
0x61 0xFF + 24x 0x00
LEN: u16 LE (1..=200 max, else garbage)
name: LEN bytes printable ASCII 0x20-0x7f
```
- Regions clustered: strip suffixes `: Take 14`, `: Comp A.1`, `_001`, `#06`, `.N` iteratively. Consecutive records sharing base name = one user track.

### What the real file shows

Studied on `Projet.logicx` (Logic Pro 11.0.0, 3 tracks in `MetaData.plist`, `ProjectData` 143 288 bytes). Each item says whether the reader decodes it (`read_full`), whether it was only checked by hand, or whether it is still unknown.

**Root header (decoded, checked on the real file).** 24 bytes: magic `23 47 C0 AB`, then `CD 09 03 00`, `04 00 00 00`, `01 00 08 00`, a `u32` little-endian payload length at offset 16 (143 264 here), and four zero bytes. The payload starts at 24, and `24 + length` equals the file size. `read_full` refuses a file shorter than that with `Error::Truncated`. Two more magics sit inside the payload (at 60 and 760), which the scanners do not use.

**Stored tags.** Four-character tags are stored reversed (`karT` is `Trak`, `qeSM` is `MSeq`, `qSvE` is `EvSq`, `ivnE` is `Envi`, `gnoS` is `Song`). Only the record shapes listed below are decoded; the tag counts in the table above were not re-checked.

**Channel strips (decoded, 267 records).** Each record is one `0x00` byte, then a 16-byte name field that starts with a space (`0x20`) and holds a NUL-padded printable name, then an 8-byte descriptor. This gives the track names and kinds in file order: `Audio 1` to `Audio 5`, `Aux 1`, `Aux 2`, `Inst 1`, `Inst 2`, then 256 default busses, `Master` and `Output 1-2`. Descriptor head byte: `0x89` master, `0x49` output, `0xE9` bus, `0xAB` audio, or aux when byte 1 is `0xF5`, `0x29` instrument when byte 2 is `0xF7` (Inst 1) or `0xF3` (Inst 2).
- Correction to the Shape 4 rule: byte 2 bit 2 is set on busses, master and output too, so it does not mark a strip in use. Byte 4 non-zero marks exactly `Audio 1`, `Audio 2` and `Inst 1`, which matches `NumberOfTracks=3`. The reader uses that rule. It is a guess from one project.

**Audio Unit descriptors (decoded, 1 found).** The type code is stored little-endian (`fmua`, which is `aumf`, a music effect; the file says so, although the plug-in is an insert). The manufacturer code sits 4 bytes before it (`FbaF`, which is `FabF`), the subtype 4 bytes after (`p4QF`, which is `FQ4p`), and the plug-in name `Pro-Q 4` sits in the 12 bytes before the manufacturer code, NUL-padded.

**Audio Unit state (decoded, verified).** An XML property list follows the descriptor at 55 954. It is an Audio Unit class-info dictionary with `type`, `subtype` and `manufacturer` integers (which match the descriptor when read big-endian), `version` 0, `name` `Default Setting`, and a vendor key `FabFilterPluginState`, a 2 619-byte binary blob that starts with `FFBS`. Logic does not store the standard `data` key. `read_full` moves the blob into `data` and writes the dictionary back as an XML plist, which is what the host hands to `kAudioUnitProperty_ClassInfo`. Whether FabFilter accepts its own blob under `data` has not been tested on macOS.

**Stock plug-ins (decoded for two).** The literal `GAME` tag is preceded by two flag bytes (14 and 13 bytes before it) and by the 12-byte name. `02 02 SmpleDly` is an insert; the name is what the project stores, and it is presumably Logic's Sample Delay. `00 01 Klopfgeist` is the metronome. The other 22 occurrences of `GAME` are not records (their flags are `00 00` or they sit inside other structures).

**Sampler (decoded, the instrument file only).** The tag `PMAS` (which is `SAMP` reversed) is followed by the sub-tag `1MAS` and the NUL-terminated name `Classic Bass Finger.exs`. The file itself is at `Media/Sampler Instruments/Classic Bass Finger.exs` (5.4 MB). The sampler's zones and parameters are not decoded.

**Ownership of plug-ins (guess).** A plug-in belongs to the nearest channel strip that starts before it. The Sampler, Sample Delay and Pro-Q 4 all sit between `Inst 1` and `Inst 2`, so they go to `Inst 1`. The Klopfgeist sits after `Inst 2`, which is not in use, so it is unassigned. The project does not state the owner of a plug-in in any field the reader found.

**Found but not decoded.**
- Seven NSKeyedArchiver blobs (`bplist00`, at 27 033, 27 356, 27 969, 28 292, 50 462, 60 094 and 62 883). The one at 60 094 holds the Smart Controls mapping (`kSavedValueFloatKey`, `MAPlugInParameterMapping`); the one at 62 883 holds `Smart Knob 1` to `Smart Knob 8` and `IdentityIndexLookup`. The others hold empty dictionaries or the processor ID.
- `Autosave/*.songData` begins with a 16-byte header and a zlib stream at offset 16. Decompressed, it is 325 663 bytes, starts with the same magic, and contains the same tags (`GAME` and `Pro-Q` strings included). It was not parsed. `*.eventData` decompresses to 1 264 bytes of 32-byte records that begin `68 26 94 F5`. Neither is decoded.
- `MSeq` (`qeSM`) and `EvSq` (`qSvE`) records: the MIDI notes are not decoded, so `MidiRegion::notes` stays empty. The project has no audio region records (`gRuA` is absent), and `Media/Audio Files/` is empty, so `AudioRegion` stays empty too.
- Volume, pan, mute, solo and bypass are not located in `ProjectData`, so the reader reports them as `None`.
- Track names in the regions and the automation section (`Audio 1` at 133 512, and similar) are not used.

**Not checked.** Only one project was studied, made on one machine, and the shapes above are not yet confirmed on other Logic versions or other projects.

### Second survey: notes, audio regions, faders and pans

Three saved versions of the same project were used: the first survey (V0, 143 288 bytes, kept as `Project File Backups/00`), a version with the fourth track and the bounce (V1, 148 780 bytes, `Project File Backups/01`), and the current file (V3, 158 369 bytes). Checksums are in the snapshots under `/private/tmp/pw`. Each item says what was verified.

**Correction: the Audio Unit state is kept as saved.** The first version of the reader moved Logic's `FabFilterPluginState` blob under a `data` key. Pro-Q 4 rejects that form. Its own `save_state` has the same keys as Logic's dictionary (`type`, `subtype`, `manufacturer`, `version`, `name`, and the vendor key `FabFilterPluginState`), and `AudioUnitSetProperty(kAudioUnitProperty_ClassInfo)` accepts it. The reader now returns the dictionary unchanged. `tests/fabfilter_host.rs` (macOS, ignored) loads the project's state into Pro-Q 4 and gets `load_state` = true.

**Correction: the note times are at 240 ticks per beat in this file, not 960.** See the notes below.

**Notes (decoded, matches the user's piano roll).** A note record is 32 bytes. Byte 21 is `0x89`, bytes 22 to 25 are zero, byte 30 is `0x90` (note on) for two notes and `0xF1` for the third. The start is a little-endian `u16` at byte 2, the pitch is the byte at 10, the velocity the byte at 14 (64 in all three) and the length a `u16` at 26. The three notes are `0x96F0` (38 640) pitch 59 length 240, `0x97E0` (38 880) pitch 62 length 240, and `0x99C0` (39 360) pitch 64 length 1 920. Measured from 38 400 at 240 ticks per beat, they start on beat 2, beat 3 and bar 2 beat 1, with lengths of 1, 1 and 8 beats. The user's piano roll gave the same beats and pitches (B2, D3, E3). The note records sit between the `qeSM` record named `Inst 1` and the next sequence, so the notes belong to Inst 1's region. Two things are not verified: the 240-tick unit is inferred from the match with the piano roll, not read from a field, and 38 400 is an origin chosen to make the first note land on beat 2 of a region that starts at bar 1. The region's own start field is not located. Logic's usual 960 PPQ would make the lengths four times as long.

**Audio regions (name and file decoded, start and length not located).** A `gRuA` record is 248 bytes. Bytes 4 to 5 are the version (2, 0). The `u16` at 0x6E is the name length and the name follows at 0x70. Two records exist: `Inst 1_bip` and `Inst 1_bip.1`. Both name `Audio Files/Inst 1_bip.wav` (24-bit stereo 44.1 kHz, 99 807 frames, 2.26 s), which is listed in `MetaData.plist`'s `AudioFiles`. The start and length were searched as frame counts (99 807), the rate (44 100), tick values (4 320 and 240 at 240 per beat), and as `f32`/`f64` durations and beat counts. None matched. The owning track is not decoded.

**Faders and pans (candidates, verified on the three saved versions).** A channel strip has an `OCuA` record that starts 96 bytes before its name field. Byte 121 of that record is a fader byte: 90 on every default strip, 100 on Inst 1 and 117 on Audio 3 in V3. Byte 125 is a pan byte: 64 on every default strip (centre), 93 on Inst 1 and 23 on Audio 3. Read as Logic's -64 to +63 scale, V3 holds pan +29 for Inst 1 and -41 for Audio 3. These are the values the reader reports. The user's latest values (Inst 1 at -2.2 dB and pan +36, Audio 3 at +4.6 dB and pan -64) are not in the saved file, which matches the earlier screenshot instead (Inst 1 at +2.0 dB, pan +29). Nothing else of this kind was found: no float or integer encoding of the dB or pan values exists in `ProjectData`, and the `UCuA` records and the `bplist00` archives hold no fader or pan. The fader byte is not converted to dB because the curve from byte to dB is not known from two points.

**Versions.** V1 has six audio strips (`Audio 1` to `Audio 6`), two aux, and `Inst 1`. The reader's active flag (descriptor byte 4) marks four strips in V3: `Audio 1`, `Audio 2`, `Audio 3` and `Inst 1`, matching `NumberOfTracks=4`. V1 to V3 differs in strip records, the notes and the faders: V1 and the current file differ in only nine bytes apart from the strip changes, and a diff of V2 against V3 shows a 9 KB shift early in the file. The last byte difference between V1 and V2 (a 16-bit value changing from 1920 to 1830) is not a note.

**Not decoded.** The start and length of the Inst 1 region and the audio region. Mute, solo, record-enable and input monitoring: the strip bytes 164, 166, 168, 176, 180 and 184 differ on Inst 1 only, which is a candidate for the record and monitor state, not verified. Bypass of plug-in slots: not located. Plug-in vendors: the file stores only the manufacturer code.

## 3. How to Write .logicx (Export) — Seed Synthesis Method

You CANNOT hand-craft ProjectData from scratch reliably. Logic is strict, will show "Project corrupted" if bytes wrong.

Proven method from `LogicProFormatWriter` (MIT, pure Python, Logic-validated):

- Bundle ~62KB donor data in `logicx/data/` = two base sessions: `audio_base.seed`, `mixed_base.seed` + `infra.json.gz` (constant records)
- Donor data is GENERATED ARTIFACT from real Logic control sessions. Never hand-edited. Recipe in DONORS.md
- To synthesize new session with M instrument + N audio tracks:
  1. Load base seeds
  2. Parse TimeMap from MIDI (tempo/meter/markers)
  3. For each track, clone track record + channel strip + AU descriptor, assign new UUIDs
  4. For each region, synthesize AuRg (audio) or MSeq (MIDI) records with correct tick positions (PPQ 960)
  5. Patch offsets, lengths, fix 24-byte root header `u32 @ +0x10 = payload length`
  6. Write bundle structure

**Rust port for ProWave:**
- Crate `prowave-logicx-import` will have `logicx/data/` embedded via `include_bytes!`
- Function `synthesize_av_bundle(tracks: Vec<TrackDef>) -> Vec<u8>` where `TrackDef { name, kind, regions: Vec<RegionDef> }`
- `RegionDef` = `Audio { file: Path, start_tick, length_ticks, gain }` or `Midi { notes: Vec<MidiNote>, start_tick }`
- Audio files: must be 44.1/48kHz WAV (Logic expects). If mp3/aiff, transcode via `symphonia` + `wav` writer, or ALAC in CAF via `alac` crate for lossless 3-4x smaller (Logic plays ALAC/CAF natively)
- For reading, we parse. For writing, we synthesize via seeds.

**Key Rust crates:**
- `plist` for MetaData
- `byteorder` for BE/LE reads
- `memchr` for fast scanning
- `serde_json` for Drummer JSON
- `uuid` for track UUIDs

## 4. ProWave Implementation Plan for Claude (Linux + macOS Lightweight)

### Crate Structure
```
crates/prowave-logicx-import/
  src/
    lib.rs              # public API: open_logicx(path) -> ProWaveProject, save_logicx(project, path)
    bundle.rs           # parse .logicx bundle structure, handle Package vs Folder, Alternatives
    plist.rs            # parse MetaData.plist, ProjectInformation.plist
    project_data/
      mod.rs            # linear scan dispatcher
      scanner.rs        # fast memchr scan for markers
      au.rs             # Shape 1: AU triple
      stock.rs          # Shape 2: GAME anchor
      drummer.rs        # Shape 3: JSON
      tracks.rs         # Shape 4: channel-strip records
      regions.rs        # Shape 5: audio region records + clustering
      time_map.rs       # tempo/meter/markers extraction
    synth/
      mod.rs            # synthesis from seeds (export)
      seeds.rs          # include_bytes! for audio_base.seed, mixed_base.seed, infra.json
      audio.rs          # audio region synthesis
      midi.rs           # MIDI note synthesis
  data/
    audio_base.seed     # embedded donor (62KB)
    mixed_base.seed
    infra.json.gz
  tests/
    fixtures/           # real .logicx from Logic Pro 11 (small, sine tones)
```

### API for ProWave Core

```rust
// Open official Logic Pro project on Linux/macOS
pub fn open_logicx(path: &Path) -> Result<ProWaveProject, LogicxError> {
    // 1. Detect if path is bundle (dir with Alternatives) or folder project
    // 2. Read Resources/ProjectInformation.plist -> active alternative index
    // 3. Read Alternatives/XXX/MetaData.plist -> tempo, key, sample rate
    // 4. Read Alternatives/XXX/ProjectData via linear scan:
    //    - scan_au_descriptors() -> Vec<AuPlugin>
    //    - scan_stock_plugins() -> Vec<StockPlugin>
    //    - scan_drummer() -> Vec<DrummerPlayer>
    //    - scan_tracks() -> Vec<TrackRecord>
    //    - scan_regions() -> Vec<RegionRecord> -> cluster to tracks
    // 5. Resolve: assign AUs to tracks by nearest preceding track offset (like lpx-explorer)
    // 6. Convert to ProWaveProject { tracks: Vec<Track>, tempo_map, audio_files: Vec<Path> }
}

pub struct ProWaveProject {
    pub tempo_map: TempoMap, // tempo events, time sig, markers
    pub tracks: Vec<Track>,
    pub audio_files: Vec<AudioFileRef>, // paths inside Media/Audio Files/
}

pub enum Track {
    Audio { name: String, regions: Vec<AudioRegion>, channel_strip: ChannelStrip },
    Instrument { name: String, instrument: Instrument, midi_regions: Vec<MidiRegion>, fx: Vec<Fx> },
    Drummer { name: String, drummer_type: String, regions: Vec<DrummerRegion> },
    Aux { name: String }, Bus, Master
}

// Save to .logicx that Logic Pro opens clean (no repair prompt)
pub fn save_logicx(project: &ProWaveProject, out_path: &Path) -> Result<(), LogicxError> {
    // Use seed synthesis method, not hand-crafted
}
```

### Linux vs macOS Specifics

**Linux:**
- No AU hosting. When opening .logicx with AU plugins (e.g., Alchemy), map to ProWave native equivalents: Alchemy -> ProWave Alchemy-lite, Channel EQ -> prowave Channel EQ, Compressor -> prowave Compressor. Show UI: "Alchemy not available on Linux, replaced with ProWave Alchemy-lite (presets preserved where possible)"
- Audio I/O: ALSA/JACK/PipeWire via cpal. No CoreAudio. No need for `com.apple.*` entitlements.
- File system: .logicx bundle is just a directory on Linux (no Finder magic). Handle both as dir. Case-sensitive FS: preserve case.
- Performance: Use `mimalloc` allocator, `mmap` for ProjectData, lazy load audio files (streaming, not load all). Target <150MB RAM idle.

**macOS:**
- AU hosting: Can host AU via `coreaudio-sys`. When opening .logicx, if AU present on system (e.g., FabFilter), use it. If not, fallback to ProWave native.
- CoreAudio: Use `cpal` with CoreAudio backend, support aggregate devices.
- Bundle: .logicx is package, use `std::fs::is_dir` + check for `Alternatives` subdir to detect.
- Notarization: No need for parsing, but for saving .logicx, ensure bundle has correct Finder attributes (package bit). Use `xattr`? Not needed, just dir.

### Lightweight Optimizations (Why 20x lighter than Logic)

1. **No 70GB Library:** ProWave ships with <100MB of essential IRs and wavetables, not 70GB. User can optionally download Logic library if they have Logic installed (read from `/Library/Application Support/Logic/`), but not required.
2. **No Objective-C / Swift runtime:** Logic uses NSKeyedArchiver, CoreData. ProWave uses pure Rust `rkyv` zero-copy, no ObjC.
3. **Lazy loading:** Logic loads all plugins on open. ProWave: scan ProjectData linearly, only instantiate channel strips when track selected, plugins sandboxed and lazy.
4. **Single binary:** Logic = 1.2GB app + helpers. ProWave = <50MB single binary via `cargo build --release` + `strip` + `mimalloc` + LTO.
5. **WASM core:** Same engine for native and web, no duplicate code.
6. **No legacy:** Logic has 30 years of legacy (Notator, etc.). ProWave starts clean, only modern formats (CLAP primary, VST3, AU on macOS).

### Tests (Must pass for Claude)

- `cargo test --test logicx_import -- fixtures/logic_11_empty.logicx` -> should parse 0 tracks, tempo 120, 4/4
- `cargo test --test logicx_import -- fixtures/logic_11_drummer.logicx` -> should parse 1 Drummer track, type Acoustic, 8 bars
- `cargo test --test logicx_import -- fixtures/logic_11_audio_midi.logicx` -> 1 audio + 1 instrument, audio region name, MIDI notes count
- Round-trip: `open_logicx -> save_logicx -> open_logicx` should preserve track count, names, tempo, regions (byte-identical ProjectData not required, but musical data identical)
- Fuzz: `cargo fuzz run project_data_parser -- -max_total_time=300` with corpus of ProjectData files

## 5. Security (for .logicx)

- ProjectData is untrusted (downloaded from internet). Limits:
  - MAX_PROJECTDATA_SIZE = 200 MB
  - MAX_TRACKS = 2000 (Logic max 1000, we allow 2000)
  - MAX_REGIONS_PER_TRACK = 10000
  - String lengths: region name max 200 chars (from lpx-explorer), track name 16 chars, but allow up to 100 for ProWave
  - No path traversal: audio file refs inside Media/Audio Files/ only, reject `..` or absolute paths
  - No exec: never `eval` Drummer JSON params, parse as JSON only

## 6. Future Options

- **Write full Alchemy preset parser:** Alchemy stores presets as binary blobs inside ProjectData, not yet decoded. For v2, reverse engineer Alchemy preset chunk (likely `Alch` marker)
- **Flex Time/Pitch import:** Flex markers stored in ProjectData, not yet decoded. Need to find `FlxT` marker?
- **Automation import:** Event sequences `qSvE` contains automation, need to decode curve type (linear, bezier)
- **Score import:** Text styles `tSxT` contains notation, need MusicXML conversion
- **Export to Logic with full instruments:** Currently seed method supports audio + basic instrument tracks. Extend to support Alchemy, Sampler zones, Drum Machine Designer pads

---

**For Claude:** Start with `crates/prowave-logicx-import/src/bundle.rs` + `plist.rs` + `project_data/scanner.rs`. Get empty project parsing working first, then add AU, then stock, then drummer, then regions. Use `lpx-explorer` Rust crate as reference (MIT), but rewrite in pure Rust with no Tauri dependency.

[^1]: logicx-analyzer/CLAUDE.md at 655fa43dd74be15a83e85e59bb8f886e0f25ed6f · geoffmyers/logicx-analyzer — https://github.com/geoffmyers/logicx-analyzer/blob/HEAD/CLAUDE.md
