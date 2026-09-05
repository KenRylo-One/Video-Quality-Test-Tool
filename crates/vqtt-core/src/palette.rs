//! The series palette.
//!
//! Eight fixed slots, keyed to file identity and never to row position. The palette never
//! cycles. Above eight encodes the tool draws small multiples.

/// Which theme the window is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// The default.
    #[default]
    Dark,
    /// Fully supported.
    Light,
}

impl Theme {
    /// The label on the theme button.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

/// One series color, as red, green and blue bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeriesColor {
    /// The name of the slot, for the legend and for a report.
    pub name: &'static str,
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

/// How many slots the palette holds. Above this the tool draws small multiples.
pub const SERIES_SLOTS: usize = 8;

/// The dark theme column.
pub const DARK_SERIES: [SeriesColor; SERIES_SLOTS] = [
    SeriesColor {
        name: "blue",
        r: 0x39,
        g: 0x87,
        b: 0xe5,
    },
    SeriesColor {
        name: "orange",
        r: 0xd9,
        g: 0x59,
        b: 0x26,
    },
    SeriesColor {
        name: "aqua",
        r: 0x19,
        g: 0x9e,
        b: 0x70,
    },
    SeriesColor {
        name: "yellow",
        r: 0xc9,
        g: 0x85,
        b: 0x00,
    },
    SeriesColor {
        name: "magenta",
        r: 0xd5,
        g: 0x51,
        b: 0x81,
    },
    SeriesColor {
        name: "green",
        r: 0x00,
        g: 0x83,
        b: 0x00,
    },
    SeriesColor {
        name: "violet",
        r: 0x90,
        g: 0x85,
        b: 0xe9,
    },
    SeriesColor {
        name: "red",
        r: 0xe6,
        g: 0x67,
        b: 0x67,
    },
];

/// The light theme column.
pub const LIGHT_SERIES: [SeriesColor; SERIES_SLOTS] = [
    SeriesColor {
        name: "blue",
        r: 0x2a,
        g: 0x78,
        b: 0xd6,
    },
    SeriesColor {
        name: "orange",
        r: 0xeb,
        g: 0x68,
        b: 0x34,
    },
    SeriesColor {
        name: "aqua",
        r: 0x1b,
        g: 0xaf,
        b: 0x7a,
    },
    SeriesColor {
        name: "yellow",
        r: 0xed,
        g: 0xa1,
        b: 0x00,
    },
    SeriesColor {
        name: "magenta",
        r: 0xe8,
        g: 0x7b,
        b: 0xa4,
    },
    SeriesColor {
        name: "green",
        r: 0x00,
        g: 0x83,
        b: 0x00,
    },
    SeriesColor {
        name: "violet",
        r: 0x4a,
        g: 0x3a,
        b: 0xa7,
    },
    SeriesColor {
        name: "red",
        r: 0xe3,
        g: 0x49,
        b: 0x48,
    },
];

/// The dash pattern of each slot, for the high contrast switch and for print.
pub const DASH_PATTERNS: [&[f32]; SERIES_SLOTS] = [
    &[],
    &[6.0, 4.0],
    &[2.0, 3.0],
    &[8.0, 3.0, 2.0, 3.0],
    &[1.0, 3.0],
    &[10.0, 2.0, 2.0, 2.0],
    &[4.0, 2.0, 1.0, 2.0],
    &[3.0, 3.0, 6.0, 3.0],
];

/// The color of one slot in one theme.
///
/// The slot number comes from file identity. It is never the row index, so a reorder
/// never repaints a line.
pub fn series_color(theme: Theme, slot: usize) -> Option<SeriesColor> {
    if slot >= SERIES_SLOTS {
        return None;
    }
    Some(match theme {
        Theme::Dark => DARK_SERIES[slot],
        Theme::Light => LIGHT_SERIES[slot],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_columns_hold_eight_slots() {
        assert_eq!(DARK_SERIES.len(), SERIES_SLOTS);
        assert_eq!(LIGHT_SERIES.len(), SERIES_SLOTS);
        assert_eq!(DASH_PATTERNS.len(), SERIES_SLOTS);
    }

    #[test]
    fn the_palette_never_cycles() {
        assert!(series_color(Theme::Dark, 7).is_some());
        assert!(series_color(Theme::Dark, 8).is_none());
        assert!(series_color(Theme::Light, 8).is_none());
    }

    #[test]
    fn the_slot_names_match_across_the_two_columns() {
        for slot in 0..SERIES_SLOTS {
            assert_eq!(DARK_SERIES[slot].name, LIGHT_SERIES[slot].name);
        }
    }
}
