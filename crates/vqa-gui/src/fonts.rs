//! Loads the IBM Plex font files that ship inside `assets/fonts/`.

use egui::{FontData, FontDefinitions, FontFamily};
use std::sync::Arc;

/// The name of the proportional family, at Regular weight. `widgets::sans` reads it.
pub const SANS_FAMILY: &str = "IBM Plex Sans";

/// The name of the monospace family, at Regular weight. `widgets::mono` reads it.
pub const MONO_FAMILY: &str = "IBM Plex Mono";

/// The name of the proportional family, at Medium weight.
pub const SANS_MEDIUM_FAMILY: &str = "IBM Plex Sans Medium";

/// The name of the monospace family, at Medium weight.
pub const MONO_MEDIUM_FAMILY: &str = "IBM Plex Mono Medium";

/// The name of the proportional family, at SemiBold weight.
pub const SANS_SEMIBOLD_FAMILY: &str = "IBM Plex Sans SemiBold";

/// The name of the monospace family, at SemiBold weight.
pub const MONO_SEMIBOLD_FAMILY: &str = "IBM Plex Mono SemiBold";

/// The faces an exported graph is drawn with.
///
/// The rasterizer reads its own font database, so the export hands it the same files
/// the window draws with rather than hoping the machine has them installed.
pub const FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
];

/// Builds the font table that the window loads at startup.
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();

    add_family(
        &mut fonts,
        SANS_FAMILY,
        "ibm-plex-sans-regular",
        include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    );
    add_family(
        &mut fonts,
        SANS_MEDIUM_FAMILY,
        "ibm-plex-sans-medium",
        include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
    );
    add_family(
        &mut fonts,
        SANS_SEMIBOLD_FAMILY,
        "ibm-plex-sans-semibold",
        include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
    );
    add_family(
        &mut fonts,
        MONO_FAMILY,
        "ibm-plex-mono-regular",
        include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
    );
    add_family(
        &mut fonts,
        MONO_MEDIUM_FAMILY,
        "ibm-plex-mono-medium",
        include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
    );
    add_family(
        &mut fonts,
        MONO_SEMIBOLD_FAMILY,
        "ibm-plex-mono-semibold",
        include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf"),
    );

    fonts
}

/// Registers one font file under one named family.
fn add_family(fonts: &mut FontDefinitions, family_name: &str, key: &str, bytes: &'static [u8]) {
    fonts
        .font_data
        .insert(key.to_owned(), Arc::new(FontData::from_static(bytes)));
    fonts
        .families
        .insert(FontFamily::Name(family_name.into()), vec![key.to_owned()]);
}
