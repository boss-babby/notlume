//! Notion-like palettes (light, dark, high-contrast) with a custom accent,
//! interpolated for the animated theme switch.

use crate::anim::lerp_color;
use egui::{Color32, Context, CornerRadius, Stroke, Visuals};
use nw_core::ThemeMode;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub hover: Color32,
    pub active: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub border: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub code_bg: Color32,
    pub callout_bg: Color32,
    pub link: Color32,
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

impl Palette {
    pub fn light(accent: [u8; 3]) -> Self {
        let a = rgb(accent);
        Self {
            bg: Color32::from_rgb(255, 255, 255),
            sidebar: Color32::from_rgb(247, 247, 245),
            hover: Color32::from_rgb(239, 239, 237),
            active: Color32::from_rgb(228, 228, 225),
            text: Color32::from_rgb(55, 53, 47),
            dim: Color32::from_rgb(140, 138, 132),
            border: Color32::from_rgb(233, 233, 231),
            accent: a,
            accent_soft: Color32::from_rgba_unmultiplied(accent[0], accent[1], accent[2], 40),
            code_bg: Color32::from_rgb(247, 246, 243),
            callout_bg: Color32::from_rgb(241, 241, 239),
            link: a,
        }
    }

    pub fn dark(accent: [u8; 3]) -> Self {
        let a = rgb(accent);
        Self {
            bg: Color32::from_rgb(25, 25, 25),
            sidebar: Color32::from_rgb(32, 32, 32),
            hover: Color32::from_rgb(44, 44, 44),
            active: Color32::from_rgb(56, 56, 56),
            text: Color32::from_rgb(235, 235, 233),
            dim: Color32::from_rgb(145, 145, 142),
            border: Color32::from_rgb(52, 52, 52),
            accent: a,
            accent_soft: Color32::from_rgba_unmultiplied(accent[0], accent[1], accent[2], 60),
            code_bg: Color32::from_rgb(36, 36, 36),
            callout_bg: Color32::from_rgb(40, 40, 40),
            link: a,
        }
    }

    pub fn high_contrast(accent: [u8; 3]) -> Self {
        let mut p = Self::dark(accent);
        p.bg = Color32::BLACK;
        p.sidebar = Color32::from_rgb(10, 10, 10);
        p.text = Color32::WHITE;
        p.dim = Color32::from_rgb(210, 210, 210);
        p.border = Color32::from_rgb(180, 180, 180);
        p.hover = Color32::from_rgb(50, 50, 50);
        p.active = Color32::from_rgb(80, 80, 80);
        p.accent = Color32::from_rgb(255, 214, 0);
        p.link = p.accent;
        p.accent_soft = Color32::from_rgba_unmultiplied(255, 214, 0, 70);
        p
    }

    pub fn for_mode(mode: ThemeMode, accent: [u8; 3]) -> Self {
        match mode {
            ThemeMode::Light => Self::light(accent),
            ThemeMode::Dark => Self::dark(accent),
            ThemeMode::HighContrast => Self::high_contrast(accent),
        }
    }

    pub fn is_dark(&self) -> bool {
        let c = self.bg;
        u16::from(c.r()) + u16::from(c.g()) + u16::from(c.b()) < 384
    }

    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        let l = |a, b| lerp_color(a, b, t);
        Self {
            bg: l(self.bg, other.bg),
            sidebar: l(self.sidebar, other.sidebar),
            hover: l(self.hover, other.hover),
            active: l(self.active, other.active),
            text: l(self.text, other.text),
            dim: l(self.dim, other.dim),
            border: l(self.border, other.border),
            accent: l(self.accent, other.accent),
            accent_soft: l(self.accent_soft, other.accent_soft),
            code_bg: l(self.code_bg, other.code_bg),
            callout_bg: l(self.callout_bg, other.callout_bg),
            link: l(self.link, other.link),
        }
    }
}

/// Push the palette into egui's visuals.
pub fn apply(ctx: &Context, p: &Palette) {
    let mut v = if p.is_dark() { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.code_bg;
    v.faint_bg_color = p.hover;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.link;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.noninteractive,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(6);
    }
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = p.hover;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.weak_bg_fill = p.hover;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.weak_bg_fill = p.active;
    v.widgets.active.bg_fill = p.active;
    v.widgets.open.weak_bg_fill = p.active;
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(10);
    ctx.set_visuals(v);
}

pub const ACCENTS: &[(&str, [u8; 3])] = &[
    ("Blue", [35, 131, 226]),
    ("Purple", [144, 101, 176]),
    ("Pink", [193, 76, 138]),
    ("Red", [224, 62, 62]),
    ("Orange", [217, 115, 13]),
    ("Green", [68, 131, 97]),
    ("Gray", [120, 119, 116]),
];
