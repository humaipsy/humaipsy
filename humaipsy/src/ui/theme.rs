//! Catppuccin palettes and the small egui `Visuals` mapping the app uses.
//!
//! The `catppuccin-egui` crate does not yet support egui 0.37 (its latest
//! release supports up to 0.33), so the palette values live here instead of
//! pulling a second, incompatible copy of egui into the build. These are the
//! published Catppuccin colours (https://catppuccin.com/palette); the mapping
//! follows the same intent as catppuccin-egui (MIT): base/mantle/crust for the
//! surfaces, the accent colours for links/warnings/errors, and `text` for
//! widget foregrounds.

use eframe::egui;

use crate::app_model::CatppuccinTheme;

struct Palette {
    rosewater: egui::Color32,
    maroon: egui::Color32,
    peach: egui::Color32,
    blue: egui::Color32,
    text: egui::Color32,
    overlay1: egui::Color32,
    surface2: egui::Color32,
    surface1: egui::Color32,
    surface0: egui::Color32,
    base: egui::Color32,
    mantle: egui::Color32,
    crust: egui::Color32,
    light: bool,
}

const LATTE: Palette = Palette {
    rosewater: egui::Color32::from_rgb(220, 138, 120),
    maroon: egui::Color32::from_rgb(230, 69, 83),
    peach: egui::Color32::from_rgb(254, 100, 11),
    blue: egui::Color32::from_rgb(30, 102, 245),
    text: egui::Color32::from_rgb(76, 79, 105),
    overlay1: egui::Color32::from_rgb(140, 143, 161),
    surface2: egui::Color32::from_rgb(172, 176, 190),
    surface1: egui::Color32::from_rgb(188, 192, 204),
    surface0: egui::Color32::from_rgb(204, 208, 218),
    base: egui::Color32::from_rgb(239, 241, 245),
    mantle: egui::Color32::from_rgb(230, 233, 239),
    crust: egui::Color32::from_rgb(220, 224, 232),
    light: true,
};

const FRAPPE: Palette = Palette {
    rosewater: egui::Color32::from_rgb(242, 213, 207),
    maroon: egui::Color32::from_rgb(234, 153, 156),
    peach: egui::Color32::from_rgb(239, 159, 118),
    blue: egui::Color32::from_rgb(140, 170, 238),
    text: egui::Color32::from_rgb(198, 208, 245),
    overlay1: egui::Color32::from_rgb(131, 139, 167),
    surface2: egui::Color32::from_rgb(98, 104, 128),
    surface1: egui::Color32::from_rgb(81, 87, 109),
    surface0: egui::Color32::from_rgb(65, 69, 89),
    base: egui::Color32::from_rgb(48, 52, 70),
    mantle: egui::Color32::from_rgb(41, 44, 60),
    crust: egui::Color32::from_rgb(35, 38, 52),
    light: false,
};

const MACCHIATO: Palette = Palette {
    rosewater: egui::Color32::from_rgb(244, 219, 214),
    maroon: egui::Color32::from_rgb(238, 153, 160),
    peach: egui::Color32::from_rgb(245, 169, 127),
    blue: egui::Color32::from_rgb(138, 173, 244),
    text: egui::Color32::from_rgb(202, 211, 245),
    overlay1: egui::Color32::from_rgb(128, 135, 162),
    surface2: egui::Color32::from_rgb(91, 96, 120),
    surface1: egui::Color32::from_rgb(73, 77, 100),
    surface0: egui::Color32::from_rgb(54, 58, 79),
    base: egui::Color32::from_rgb(36, 39, 58),
    mantle: egui::Color32::from_rgb(30, 32, 48),
    crust: egui::Color32::from_rgb(24, 25, 38),
    light: false,
};

const MOCHA: Palette = Palette {
    rosewater: egui::Color32::from_rgb(245, 224, 220),
    maroon: egui::Color32::from_rgb(235, 160, 172),
    peach: egui::Color32::from_rgb(250, 179, 135),
    blue: egui::Color32::from_rgb(137, 180, 250),
    text: egui::Color32::from_rgb(205, 214, 244),
    overlay1: egui::Color32::from_rgb(127, 132, 156),
    surface2: egui::Color32::from_rgb(88, 91, 112),
    surface1: egui::Color32::from_rgb(69, 71, 90),
    surface0: egui::Color32::from_rgb(49, 50, 68),
    base: egui::Color32::from_rgb(30, 30, 46),
    mantle: egui::Color32::from_rgb(24, 24, 37),
    crust: egui::Color32::from_rgb(17, 17, 27),
    light: false,
};

pub(crate) fn apply(ctx: &egui::Context, theme: CatppuccinTheme) {
    let palette = match theme {
        CatppuccinTheme::Latte => &LATTE,
        CatppuccinTheme::Frappe => &FRAPPE,
        CatppuccinTheme::Macchiato => &MACCHIATO,
        CatppuccinTheme::Mocha => &MOCHA,
    };

    let mut visuals = if palette.light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };

    visuals.panel_fill = palette.base;
    visuals.window_fill = palette.base;
    visuals.faint_bg_color = palette.surface0;
    visuals.extreme_bg_color = palette.crust;
    visuals.code_bg_color = palette.mantle;
    visuals.hyperlink_color = palette.rosewater;
    visuals.warn_fg_color = palette.peach;
    visuals.error_fg_color = palette.maroon;
    visuals.window_stroke.color = palette.overlay1;
    visuals.dark_mode = !palette.light;

    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.fg_stroke.color = palette.text;
        widget.bg_stroke.color = palette.overlay1;
    }
    visuals.widgets.noninteractive.bg_fill = palette.base;
    visuals.widgets.noninteractive.weak_bg_fill = palette.base;
    visuals.widgets.inactive.bg_fill = palette.surface0;
    visuals.widgets.inactive.weak_bg_fill = palette.surface0;
    visuals.widgets.hovered.bg_fill = palette.surface2;
    visuals.widgets.hovered.weak_bg_fill = palette.surface2;
    visuals.widgets.active.bg_fill = palette.surface1;
    visuals.widgets.active.weak_bg_fill = palette.surface1;
    visuals.widgets.open.bg_fill = palette.surface0;
    visuals.widgets.open.weak_bg_fill = palette.surface0;

    visuals.selection.bg_fill = palette
        .blue
        .linear_multiply(if palette.light { 0.4 } else { 0.2 });
    visuals.selection.stroke.color = palette.text;

    ctx.set_visuals(visuals);
}
