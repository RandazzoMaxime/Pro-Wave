# CLAUDE.md

Read `AGENTS.md` first: it has the repository rules (audio-thread rules, checks, conventions).

## Logic Pro project format (`.logicx`)

Whenever you read, write or change anything about Logic Pro's project format (`crates/prowave-logicx-import`, `crates/prowave-app/src/logicx.rs`, `LOGICX-FORMAT-SPEC.md`), use these two references first:

- **LogicProFormatWriter** (<https://github.com/jonkubis/LogicProFormatWriter>): an independent project that writes `.logicx` sessions and documents the binary `ProjectData` format byte by byte. The spec is `PROJECTDATA_FORMAT.md` in that repository (raw: <https://raw.githubusercontent.com/jonkubis/LogicProFormatWriter/main/PROJECTDATA_FORMAT.md>; it is long, so read it in parts). The Python in `logicx/projectdata.py` and `logicx/midimap.py` shows the layouts in code. Resolutions and offsets there (for example 960 pulses per quarter note, the placement events, the `gRuA` and `qeSM` records) were checked against a real project, and they are the ones to trust before guessing.
- **`LOGICX-FORMAT-SPEC.md`** in this repository: our own notes. Its section "What the real file shows" and "Second survey" say where a real Logic Pro 11 project agrees with the writer's document and where it does not. When you learn something new from a real project, write it there.

Rules for this area:

- A project file is untrusted input: bounds-check every read, cap sizes, never panic.
- Do not decode a field by guessing. Check it against a real `.logicx` (ideally a pair of projects where one value was changed), and say in the notes what was verified and what was not. What the format does not document (fader curve, mute, solo, bypass, settings of Logic's own plug-ins) stays "not decoded" until it is verified.
- Never modify a user's `.logicx`: copy it into a scratch folder to experiment.
