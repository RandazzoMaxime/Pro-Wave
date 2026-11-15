//! Feedback while a volume or pan control is dragged, as Logic Pro gives it: the mouse pointer
//! is hidden, and a small dark label above the control shows the value until the button is let
//! go. Every volume and pan control calls [`drag_feedback`] with its response, so the rule is
//! written once.
//!
//! The label texts are built here too, so every control writes its value the same way: a
//! volume in dB with one decimal (`Volume: -0.4 dB`), a pan on Logic's scale from -64 to 63
//! (`Pan: -7`), and a send level as the gain it sends (`Send: -6.0 dB`).
use crate::i18n::tr;
use crate::mixer::lin_to_db;
use egui::{pos2, vec2, Color32, Context, CursorIcon, FontId, Id, LayerId, Order, Rect, Response};

/// Space between the control's top edge and the label.
const GAP: f32 = 4.0;
/// Space between the label's text and the edge of its dark box.
const PAD_X: f32 = 6.0;
const PAD_Y: f32 = 3.0;

/// While `response` is dragged: hides the mouse pointer and draws the label that `text` builds
/// above the control, centred on it (below it when there is no room above). Does nothing
/// otherwise, so the label and the hidden pointer are gone as soon as the button is let go.
pub fn drag_feedback(ctx: &Context, response: &Response, text: impl FnOnce() -> String) {
    if !response.dragged() {
        return;
    }
    // Logic hides the pointer while a fader or knob is dragged. egui keeps following it
    // meanwhile, so the drag carries on, and the icon comes back on the next frame.
    ctx.set_cursor_icon(CursorIcon::None);
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("drag_feedback")));
    let galley = painter.layout_no_wrap(text(), FontId::proportional(11.0), Color32::WHITE);
    let size = galley.size() + vec2(2.0 * PAD_X, 2.0 * PAD_Y);
    let screen = ctx.content_rect();
    let mut top = response.rect.top() - GAP - size.y;
    if top < screen.top() {
        top = response.rect.bottom() + GAP;
    }
    // Clamped to the window, so a control at the edge keeps its label on screen.
    let left = (response.rect.center().x - size.x / 2.0)
        .clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
    let rect = Rect::from_min_size(pos2(left, top), size);
    painter.rect_filled(rect, 4.0, Color32::from_black_alpha(215));
    painter.galley(rect.min + vec2(PAD_X, PAD_Y), galley, Color32::WHITE);
}

/// A dB value as it is shown: one decimal, between -60 and +6 dB (the fader's range), and never
/// "-0.0" for a value just under zero.
fn db_number(db: f32) -> String {
    let shown = format!("{:.1}", db.clamp(-60.0, 6.0));
    if shown == "-0.0" {
        "0.0".to_string()
    } else {
        shown
    }
}

/// `<label>: <dB> dB`.
fn db_text(label: &str, db: f32) -> String {
    format!("{label}: {} dB", db_number(db))
}

/// `Volume: -0.4 dB`: a fader, a track header's slider, the master volume.
pub fn volume_text(db: f32) -> String {
    db_text(tr("Volume"), db)
}

/// `Send: -6.0 dB` for a send level in dB, as a fader in Sends on Faders mode shows it.
pub fn send_db_text(db: f32) -> String {
    db_text(tr("Send"), db)
}

/// `Send: -6.0 dB` for a send knob at `level` (0..1): the gain the send adds, in dB.
pub fn send_text(level: f32) -> String {
    send_db_text(lin_to_db(prowave_engine::send_gain(level)))
}

/// A pan (-1..1) on Logic's scale, -64 (full left) to 63 (full right), 0 at the centre.
pub fn pan_number(pan: f32) -> i32 {
    (pan * 64.0).round().clamp(-64.0, 63.0) as i32
}

/// `Pan: -7`.
pub fn pan_text(pan: f32) -> String {
    format!("{}: {}", tr("Pan"), pan_number(pan))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_volume_shows_one_decimal_in_db() {
        assert_eq!(volume_text(-0.4), "Volume: -0.4 dB");
        assert_eq!(volume_text(0.0), "Volume: 0.0 dB");
        assert_eq!(volume_text(-12.26), "Volume: -12.3 dB");
        // A value just under zero is not shown as a negative zero.
        assert_eq!(volume_text(-0.04), "Volume: 0.0 dB");
        // The fader's range: below -60 and above +6 dB are not shown.
        assert_eq!(volume_text(-120.0), "Volume: -60.0 dB");
        assert_eq!(volume_text(9.0), "Volume: 6.0 dB");
    }

    #[test]
    fn a_pan_shows_logics_integer_scale() {
        assert_eq!(pan_text(-7.0 / 64.0), "Pan: -7");
        assert_eq!(pan_text(0.0), "Pan: 0");
        assert_eq!(pan_text(0.5), "Pan: 32");
        // Rounded to the nearest step, halves away from zero.
        assert_eq!(pan_number(0.5 / 64.0), 1);
        assert_eq!(pan_number(-0.5 / 64.0), -1);
        assert_eq!(pan_number(0.4 / 64.0), 0);
        // Full left is -64; full right stops at 63, as Logic's scale does.
        assert_eq!(pan_text(-1.0), "Pan: -64");
        assert_eq!(pan_text(1.0), "Pan: 63");
        assert_eq!(pan_number(-3.0), -64);
        assert_eq!(pan_number(3.0), 63);
    }

    #[test]
    fn a_send_shows_the_gain_it_sends_in_db() {
        // send_gain(level) is 2 * level squared: full level is about +6 dB, half is -6 dB.
        assert_eq!(send_text(1.0), "Send: 6.0 dB");
        assert_eq!(send_text(0.5), "Send: -6.0 dB");
        assert_eq!(send_text(0.0), "Send: -60.0 dB");
        assert_eq!(send_db_text(-3.0), "Send: -3.0 dB");
    }
}
