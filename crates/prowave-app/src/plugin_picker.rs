//! The Audio FX plug-in menu (Logic: a slot's plug-in menu): "No Plug-in", the recently picked
//! plug-ins, the categories as submenus, and a submenu for each hosted plug-in format.
//!
//! The categories are Logic's, in its order, so the menu reads the same as a Logic channel
//! strip's. Categories with no effect yet are shown disabled rather than hidden, which keeps
//! the list stable as effects are added. Installed third-party plug-ins (CLAP, VST3, Audio Units)
//! follow in a submenu for each format, from the catalog in `crate::plugins`. Every plug-in is a
//! submenu with one entry, "Stereo", as in Logic: the entry is what picks it.
use crate::i18n::tr;
use crate::native_popup::Item;
use crate::plugins::{Format, PluginInfo, Scan};
use prowave_engine::FxKind;

/// Logic's Audio FX categories, in the order of its menu.
pub const CATEGORIES: [&str; 14] = [
    "Amps and Pedals",
    "Delay",
    "Distortion",
    "Dynamics",
    "EQ",
    "Filter",
    "Imaging",
    "Metering",
    "Modulation",
    "Multi Effects",
    "Pitch",
    "Reverb",
    "Specialized",
    "Utility",
];

/// How many recently picked plug-ins the menu keeps.
pub const RECENT_MAX: usize = 5;

/// What the user chose in the picker.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Pick {
    Effect(FxKind),
    /// An installed CLAP, VST3 or Audio Unit plug-in.
    Plugin(PluginInfo),
}

/// What the user chose in the menu: a plug-in, or none at all ("No Plug-in").
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Choice {
    NoPlugin,
    Pick(Pick),
}

/// The formats of hosted plug-ins, in the order of the picker's submenus (Logic has one,
/// "Audio Units").
const FORMATS: [Format; 3] = [Format::AudioUnit, Format::Clap, Format::Vst3];

/// The category Logic files `kind` under.
pub fn category(kind: FxKind) -> &'static str {
    match kind {
        FxKind::Amp => "Amps and Pedals",
        FxKind::Delay => "Delay",
        FxKind::Overdrive => "Distortion",
        FxKind::Compressor | FxKind::MultiComp | FxKind::Gate | FxKind::Limiter => "Dynamics",
        FxKind::ChannelEq | FxKind::MultiEq => "EQ",
        FxKind::Filter => "Filter",
        FxKind::Chorus | FxKind::Phaser => "Modulation",
        FxKind::Reverb => "Reverb",
        // A hosted plug-in is listed under its format, not in a category.
        FxKind::External => "Specialized",
    }
}

/// Puts `pick` first in the recent list. A plug-in picked again moves to the front rather than
/// appearing twice, and the list keeps only the last [`RECENT_MAX`].
pub fn remember(recent: &mut Vec<Pick>, pick: Pick) {
    recent.retain(|old| *old != pick);
    recent.insert(0, pick);
    recent.truncate(RECENT_MAX);
}

/// The name a pick shows in the menu.
fn label(pick: &Pick) -> &str {
    match pick {
        Pick::Effect(kind) => kind.name(),
        Pick::Plugin(info) => &info.name,
    }
}

/// The menu's lines, top to bottom as in Logic: "No Plug-in" (checked on an empty slot; on a
/// `filled` one it empties the slot), the `recent` picks, the categories of Pro Wave's own
/// effects, then the installed plug-ins by format and manufacturer. Every plug-in is a submenu
/// with the one entry "Stereo", which is what picks it.
pub fn items(recent: &[Pick], filled: bool) -> Vec<Item<Choice>> {
    // The first time a menu opens, the installed plug-ins are listed in the background.
    crate::plugins::scan(false);
    let catalog = crate::plugins::catalog();
    let mut items = vec![
        Item::checked(tr("No Plug-in"), !filled, Choice::NoPlugin),
        Item::Separator,
    ];
    if !recent.is_empty() {
        items.push(Item::Label(tr("Recent").to_string()));
        items.extend(recent.iter().map(|pick| leaf(label(pick), pick)));
        items.push(Item::Separator);
    }
    for name in CATEGORIES {
        let effects: Vec<Item<Choice>> = FxKind::ALL
            .iter()
            .copied()
            .filter(|kind| category(*kind) == name)
            .map(|kind| leaf(kind.name(), &Pick::Effect(kind)))
            .collect();
        // A category with no effect yet is shown greyed rather than hidden, which keeps the
        // list as Logic's.
        let enabled = !effects.is_empty();
        items.push(Item::sub(tr(name), effects).enabled(enabled));
    }
    if matches!(crate::plugins::scan_status(), Scan::Running(..)) {
        items.push(Item::Separator);
        items.push(Item::Label(tr("Scanning plug-ins…").to_string()));
    }
    if catalog.is_empty() {
        return items;
    }
    items.push(Item::Separator);
    for format in FORMATS {
        let of_format: Vec<&PluginInfo> = catalog.iter().filter(|i| i.format == format).collect();
        if of_format.is_empty() {
            continue;
        }
        // The catalog is sorted by manufacturer, so each one's plug-ins are together.
        let vendors = of_format
            .chunk_by(|a, b| a.vendor.eq_ignore_ascii_case(&b.vendor))
            .map(|group| {
                let vendor = match group[0].vendor.as_str() {
                    "" => "—",
                    vendor => vendor,
                };
                let plugins = group
                    .iter()
                    .map(|info| leaf(&info.name, &Pick::Plugin((*info).clone())))
                    .collect();
                Item::sub(vendor, plugins)
            })
            .collect();
        items.push(Item::sub(format_menu(format), vendors));
    }
    items
}

/// A plug-in's submenu with its one entry, "Stereo"; choosing that entry picks the plug-in.
fn leaf(text: &str, pick: &Pick) -> Item<Choice> {
    Item::sub(
        text,
        vec![Item::action(tr("Stereo"), Choice::Pick(pick.clone()))],
    )
}

/// The name of a format's submenu: Logic's "Audio Units" for that format.
fn format_menu(format: Format) -> &'static str {
    match format {
        Format::AudioUnit => "Audio Units",
        other => other.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_is_in_a_category_of_the_menu() {
        for kind in FxKind::ALL {
            assert!(
                CATEGORIES.contains(&category(kind)),
                "{kind:?} is in {:?}, which the menu does not show",
                category(kind)
            );
        }
    }

    #[test]
    fn the_categories_follow_logics_menu() {
        assert_eq!(CATEGORIES[0], "Amps and Pedals");
        assert_eq!(CATEGORIES[1], "Delay");
        assert_eq!(CATEGORIES[4], "EQ");
        assert_eq!(CATEGORIES[11], "Reverb");
    }

    #[test]
    fn the_recent_list_puts_the_latest_pick_first() {
        let mut recent = Vec::new();
        remember(&mut recent, Pick::Effect(FxKind::Delay));
        remember(&mut recent, Pick::Effect(FxKind::Reverb));
        assert_eq!(
            recent,
            [Pick::Effect(FxKind::Reverb), Pick::Effect(FxKind::Delay)]
        );
    }

    #[test]
    fn a_pick_made_again_moves_to_the_front_without_a_copy() {
        let mut recent = Vec::new();
        remember(&mut recent, Pick::Effect(FxKind::Delay));
        remember(&mut recent, Pick::Effect(FxKind::Reverb));
        remember(&mut recent, Pick::Effect(FxKind::Delay));
        assert_eq!(
            recent,
            [Pick::Effect(FxKind::Delay), Pick::Effect(FxKind::Reverb)]
        );
    }

    #[test]
    fn the_recent_list_keeps_five_picks() {
        let mut recent = Vec::new();
        for kind in [
            FxKind::ChannelEq,
            FxKind::Compressor,
            FxKind::Reverb,
            FxKind::Delay,
            FxKind::Chorus,
            FxKind::Phaser,
        ] {
            remember(&mut recent, Pick::Effect(kind));
        }
        assert_eq!(recent.len(), RECENT_MAX);
        assert_eq!(recent[0], Pick::Effect(FxKind::Phaser));
        assert_eq!(
            recent.last(),
            Some(&Pick::Effect(FxKind::Compressor)),
            "the oldest pick, Channel EQ, is the one dropped"
        );
    }
}
