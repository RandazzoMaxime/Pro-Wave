//! Logic's "Create New Track" dialog: the + above the track headers opens it, rather than
//! adding a software instrument track straight away. It offers two of Logic's four families (MIDI
//! and Audio; Pattern and Session Player are left out), a choice inside each, a Details panel with the options of the chosen family, and
//! how many tracks to create.

use egui::{
    pos2, vec2, Align, Align2, Color32, Layout, Margin, Painter, Pos2, RichText, Sense, Stroke,
};

use crate::i18n::tr;
use crate::model::{Project, TrackKind};
use crate::UiState;

const WIDTH: f32 = 560.0;
const MARGIN: f32 = 16.0;
const CARD_GAP: f32 = 12.0;
const CARD_MARGIN: f32 = 12.0;
/// The height of a card's content, enough for the family with three choices, so the four
/// cards come out the same height.
const CARD_CONTENT_H: f32 = 200.0;
const CHOICE_H: f32 = 28.0;

const WINDOW_BG: Color32 = Color32::from_rgb(32, 35, 38);
const PANEL_BG: Color32 = Color32::from_rgb(27, 30, 33);
const CARD_BG: Color32 = Color32::from_rgb(50, 50, 52);
const CARD_BORDER: Color32 = Color32::from_rgb(70, 70, 72);
const CHOICE_BG: Color32 = Color32::from_rgb(60, 60, 62);
const CANCEL_BG: Color32 = Color32::from_rgb(84, 86, 90);
const LIGHT_TEXT: Color32 = Color32::from_rgb(220, 220, 222);
const DISABLED_TEXT: Color32 = Color32::from_rgb(105, 105, 108);
const GREEN: Color32 = Color32::from_rgb(40, 160, 60);
const BLUE: Color32 = Color32::from_rgb(20, 100, 220);

/// The track a dialog choice creates.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Choice {
    #[default]
    SoftwareInstrument,
    ExternalMidi,
    AudioMic,
    AudioGuitar,
}

impl Choice {
    fn kind(self) -> TrackKind {
        match self {
            Self::SoftwareInstrument => TrackKind::Instrument,
            Self::ExternalMidi => TrackKind::ExternalMidi,
            Self::AudioMic | Self::AudioGuitar => TrackKind::Audio,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::SoftwareInstrument => tr("Software Instrument"),
            Self::ExternalMidi => tr("External MIDI"),
            Self::AudioMic => tr("Mic or Line"),
            Self::AudioGuitar => tr("Guitar or Bass"),
        }
    }
}

/// The dialog's state, kept in [`UiState`] between frames.
#[derive(Default)]
pub struct NewTrackDialog {
    pub open: bool,
    choice: Choice,
    count: u32,
    /// Whether Create opens the Library, as Logic's "Open Library" box does.
    open_library: bool,
    /// Whether the audio tracks Create makes start armed for recording.
    record_enable: bool,
    /// The Details panel starts open, as in Logic; this is set when it is folded away.
    details_closed: bool,
}

impl NewTrackDialog {
    /// Opens the dialog at one track with the default (software instrument) choice.
    pub fn open(&mut self) {
        self.open = true;
        self.count = 1;
    }
}

/// The glyph drawn in a family's coloured circle.
#[derive(Clone, Copy)]
enum Glyph {
    Note,
    Wave,
}

/// One button in a card. `choice` is `None` for a choice Pro Wave does not offer yet; it is
/// drawn greyed out and cannot be clicked.
struct Entry {
    label: &'static str,
    choice: Option<Choice>,
}

impl Entry {
    fn offered(choice: Choice) -> Self {
        Self {
            label: choice.label(),
            choice: Some(choice),
        }
    }
}

/// One of the cards: its name, colour, glyph and the choices in it.
struct Family {
    name: &'static str,
    color: Color32,
    glyph: Glyph,
    entries: Vec<Entry>,
}

fn families() -> [Family; 2] {
    [
        Family {
            name: tr("MIDI"),
            color: GREEN,
            glyph: Glyph::Note,
            entries: vec![
                Entry::offered(Choice::SoftwareInstrument),
                Entry::offered(Choice::ExternalMidi),
            ],
        },
        Family {
            name: tr("Audio"),
            color: BLUE,
            glyph: Glyph::Wave,
            entries: vec![
                Entry::offered(Choice::AudioMic),
                Entry::offered(Choice::AudioGuitar),
            ],
        },
    ]
}

/// Adds the tracks the dialog describes and returns whether the Library should open.
fn add_tracks(p: &mut Project, dialog: &NewTrackDialog) -> bool {
    let kind = dialog.choice.kind();
    for _ in 0..dialog.count.clamp(1, 64) {
        let index = p.add_track(kind);
        // Record Enable only exists for audio choices, so it only applies to audio tracks.
        if kind == TrackKind::Audio && dialog.record_enable {
            p.tracks[index].rec = true;
        }
    }
    p.selected_track = p.tracks.len().saturating_sub(1);
    dialog.open_library
}

/// Draws the dialog when it is open and creates the tracks when Create is pressed.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.new_track.open {
        return;
    }
    // Enter creates and Escape cancels, as the dialog's default and cancel buttons do.
    let mut create = ctx.input(|i| i.key_pressed(egui::Key::Enter));
    let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let families = families();
    let current = s.new_track.choice;
    let selected_family = families
        .iter()
        .position(|f| f.entries.iter().any(|e| e.choice == Some(current)))
        .unwrap_or(0);
    let audio = current.kind() == TrackKind::Audio;

    egui::Window::new(tr("Create New Track"))
        .id(egui::Id::unique("create_new_track"))
        .title_bar(false)
        .resizable(false)
        .default_width(WIDTH)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(
            egui::Frame::new()
                .fill(WINDOW_BG)
                .stroke(Stroke::new(1.0, CARD_BORDER))
                .corner_radius(10)
                .inner_margin(Margin::same(MARGIN as i8)),
        )
        .show(ctx, |ui| {
            let content_w = WIDTH - 2.0 * MARGIN;
            ui.set_width(content_w);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(tr("Create New Track")).strong().size(14.0));
            });
            ui.add_space(12.0);

            // The cards share the width equally, with a gap between each.
            let cards = families.len() as f32;
            let card_w = (content_w - (cards - 1.0) * CARD_GAP) / cards;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = CARD_GAP;
                for (i, family) in families.iter().enumerate() {
                    let selected = i == selected_family;
                    let border = if selected {
                        Stroke::new(2.0, family.color)
                    } else {
                        Stroke::new(1.0, CARD_BORDER)
                    };
                    egui::Frame::new()
                        .fill(CARD_BG)
                        .stroke(border)
                        .corner_radius(10)
                        .inner_margin(Margin::same(CARD_MARGIN as i8))
                        .show(ui, |ui| {
                            ui.set_width(card_w - 2.0 * CARD_MARGIN - 4.0);
                            ui.set_min_height(CARD_CONTENT_H);
                            ui.vertical_centered(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                                ui.painter()
                                    .circle_filled(rect.center(), 28.0, family.color);
                                draw_glyph(ui.painter(), rect.center(), family.glyph);
                                ui.add_space(10.0);
                                ui.label(
                                    RichText::new(family.name)
                                        .strong()
                                        .size(17.0)
                                        .color(Color32::WHITE),
                                );
                                ui.add_space(10.0);
                                for (n, entry) in family.entries.iter().enumerate() {
                                    if n > 0 {
                                        ui.add_space(6.0);
                                    }
                                    choice_button(ui, entry, family.color, &mut s.new_track.choice);
                                }
                            });
                        });
                }
            });

            ui.add_space(14.0);
            details_header(ui, &mut s.new_track.details_closed);
            if !s.new_track.details_closed {
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .inner_margin(Margin::symmetric(MARGIN as i8, 12))
                    .show(ui, |ui| {
                        ui.set_width(content_w - 2.0 * MARGIN);
                        ui.columns(2, |cols| {
                            let dialog = &mut s.new_track;
                            let w = cols[0].available_width();
                            let left = &mut cols[0];
                            left.label(if audio {
                                tr("Audio Input:")
                            } else {
                                tr("Instrument:")
                            });
                            let source = if audio {
                                tr("No Input")
                            } else {
                                tr("Studio Bass")
                            };
                            egui::ComboBox::from_id_salt("new_track_source")
                                .width(w)
                                .selected_text(source)
                                .show_ui(left, |ui| {
                                    ui.label(source);
                                });
                            if audio {
                                let mut patch = false;
                                left.add_enabled(
                                    false,
                                    egui::Checkbox::new(&mut patch, tr("Load Default Patch")),
                                );
                            }
                            left.checkbox(&mut dialog.open_library, tr("Open Library"));

                            let w = cols[1].available_width();
                            let right = &mut cols[1];
                            right.label(tr("Audio Output:"));
                            egui::ComboBox::from_id_salt("new_track_output")
                                .width(w)
                                .selected_text(tr("Output 1 + 2"))
                                .show_ui(right, |ui| {
                                    ui.label(tr("Output 1 + 2"));
                                });
                            if audio {
                                let mut monitoring = false;
                                right.add_enabled(
                                    false,
                                    egui::Checkbox::new(&mut monitoring, tr("Input Monitoring")),
                                );
                                right.checkbox(&mut dialog.record_enable, tr("Record Enable"));
                            }
                        });
                    });
            }

            ui.add_space(14.0);
            // The number of tracks on the left and the two buttons on the right: with two cards
            // the dialog is too narrow to centre the number as Logic's wider dialog does.
            ui.horizontal(|ui| {
                ui.label(tr("Number of tracks to create:"));
                let mut count = s.new_track.count.max(1);
                if ui
                    .add(egui::DragValue::new(&mut count).range(1..=64))
                    .changed()
                {
                    s.new_track.count = count;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let create_button =
                        egui::Button::new(RichText::new(tr("Create")).color(Color32::WHITE))
                            .fill(BLUE)
                            .corner_radius(5)
                            .min_size(vec2(90.0, 26.0));
                    if ui.add(create_button).clicked() {
                        create = true;
                    }
                    let cancel_button =
                        egui::Button::new(RichText::new(tr("Cancel")).color(Color32::WHITE))
                            .fill(CANCEL_BG)
                            .corner_radius(5)
                            .min_size(vec2(90.0, 26.0));
                    if ui.add(cancel_button).clicked() {
                        cancel = true;
                    }
                });
            });
        });

    if create {
        if add_tracks(p, &s.new_track) {
            s.show_library = true;
        }
        s.new_track.open = false;
    } else if cancel {
        s.new_track.open = false;
    }
}

/// One choice as a full-width button: filled with the family colour when it is the chosen one,
/// dark otherwise, and greyed out when Pro Wave does not offer it yet.
fn choice_button(ui: &mut egui::Ui, entry: &Entry, color: Color32, chosen: &mut Choice) {
    let width = ui.available_width();
    let selected = entry.choice.is_some() && entry.choice == Some(*chosen);
    let (fill, text) = match (entry.choice, selected) {
        (None, _) => (CHOICE_BG, DISABLED_TEXT),
        (Some(_), true) => (color, Color32::WHITE),
        (Some(_), false) => (CHOICE_BG, LIGHT_TEXT),
    };
    let button = egui::Button::new(RichText::new(entry.label).size(13.0).color(text))
        .fill(fill)
        .corner_radius(5)
        .min_size(vec2(width, CHOICE_H));
    let response = ui.add_enabled(entry.choice.is_some(), button);
    if response.clicked() {
        if let Some(choice) = entry.choice {
            *chosen = choice;
        }
    }
}

/// The "Details" line that folds the panel below the cards away and back.
fn details_header(ui: &mut egui::Ui, closed: &mut bool) {
    ui.horizontal(|ui| {
        let (chevron_rect, chevron) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        draw_chevron(ui.painter(), chevron_rect.center(), *closed);
        let label = ui.add(
            egui::Label::new(RichText::new(tr("Details")).size(14.0).color(LIGHT_TEXT))
                .sense(Sense::click()),
        );
        if chevron.clicked() || label.clicked() {
            *closed = !*closed;
        }
    });
}

/// A small chevron pointing down when the panel is open and right when it is folded away.
fn draw_chevron(painter: &Painter, c: Pos2, pointing_right: bool) {
    let stroke = Stroke::new(1.5, LIGHT_TEXT);
    let points = if pointing_right {
        vec![
            pos2(c.x - 2.0, c.y - 4.0),
            pos2(c.x + 2.0, c.y),
            pos2(c.x - 2.0, c.y + 4.0),
        ]
    } else {
        vec![
            pos2(c.x - 4.0, c.y - 2.0),
            pos2(c.x, c.y + 2.0),
            pos2(c.x + 4.0, c.y - 2.0),
        ]
    };
    painter.line(points, stroke);
}

/// The white glyph in a family's circle, drawn with painter primitives so no font is needed.
fn draw_glyph(painter: &Painter, c: Pos2, glyph: Glyph) {
    let white = Color32::WHITE;
    let stroke = Stroke::new(2.5, white);
    match glyph {
        Glyph::Note => {
            // Two note heads joined by a beam, the way Logic's MIDI icon has them.
            let left = pos2(c.x - 8.0, c.y + 9.0);
            let right = pos2(c.x + 8.0, c.y + 6.0);
            painter.circle_filled(left, 5.5, white);
            painter.circle_filled(right, 5.5, white);
            let left_stem = left.x + 5.0;
            let right_stem = right.x + 5.0;
            painter.line_segment(
                [pos2(left_stem, left.y), pos2(left_stem, c.y - 12.0)],
                stroke,
            );
            painter.line_segment(
                [pos2(right_stem, right.y), pos2(right_stem, c.y - 15.0)],
                stroke,
            );
            painter.line_segment(
                [pos2(left_stem, c.y - 12.0), pos2(right_stem, c.y - 15.0)],
                Stroke::new(4.5, white),
            );
        }
        Glyph::Wave => {
            // Five vertical bars of rising and falling height, a short audio waveform.
            for (dx, half_height) in [
                (-12.0, 6.0),
                (-6.0, 14.0),
                (0.0, 22.0),
                (6.0, 14.0),
                (12.0, 6.0),
            ] {
                painter.line_segment(
                    [
                        pos2(c.x + dx, c.y - half_height),
                        pos2(c.x + dx, c.y + half_height),
                    ],
                    Stroke::new(3.0, white),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_choice_creates_its_track_kind() {
        assert_eq!(Choice::SoftwareInstrument.kind(), TrackKind::Instrument);
        assert_eq!(Choice::ExternalMidi.kind(), TrackKind::ExternalMidi);
        assert_eq!(Choice::AudioMic.kind(), TrackKind::Audio);
        assert_eq!(Choice::AudioGuitar.kind(), TrackKind::Audio);
    }

    #[test]
    fn open_library_and_record_enable_apply_when_creating() {
        let mut p = Project::demo();
        let before = p.tracks.len();
        let mut dialog = NewTrackDialog {
            open: true,
            choice: Choice::AudioMic,
            count: 2,
            ..Default::default()
        };

        // Both boxes off: the tracks are plain and the Library stays closed.
        assert!(!add_tracks(&mut p, &dialog));
        assert_eq!(p.tracks.len(), before + 2);
        assert!(p.tracks[before..].iter().all(|t| !t.rec));

        // Both boxes on: the Library opens and the new audio tracks start armed.
        dialog.open_library = true;
        dialog.record_enable = true;
        assert!(add_tracks(&mut p, &dialog));
        assert_eq!(p.tracks.len(), before + 4);
        assert!(p.tracks[before + 2..].iter().all(|t| t.rec));

        // Record Enable is only for audio, so a MIDI choice leaves the flag alone.
        dialog.choice = Choice::SoftwareInstrument;
        add_tracks(&mut p, &dialog);
        assert!(p.tracks[before + 4..].iter().all(|t| !t.rec));
    }
}
