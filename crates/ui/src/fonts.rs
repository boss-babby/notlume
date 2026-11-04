//! Font setup.
//!
//! * Always registers a `"bold"` family so the editor can render bold spans
//!   without panicking; without bundled fonts it falls back to the proportional
//!   font (spans are then emphasised by colour instead).
//! * With `--features bundled-fonts` Inter (UI) and Noto Emoji are embedded.
//!   Run `scripts/fetch-fonts.sh` once to download them into `assets/fonts/`.

use egui::{Context, FontFamily, FontId};

pub const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn install(ctx: &Context) {
    #[allow(unused_mut)]
    let mut fonts = egui::FontDefinitions::default();
    let proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.insert(FontFamily::Name(BOLD.into()), proportional);

    #[cfg(feature = "bundled-fonts")]
    {
        use std::sync::Arc;
        fonts.font_data.insert(
            "inter".into(),
            Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf"))),
        );
        fonts.font_data.insert(
            "inter-bold".into(),
            Arc::new(egui::FontData::from_static(include_bytes!("../assets/fonts/Inter-Bold.ttf"))),
        );
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, "inter".into());
        fonts
            .families
            .entry(FontFamily::Name(BOLD.into()))
            .or_default()
            .insert(0, "inter-bold".into());
    }
    ctx.set_fonts(fonts);
}
