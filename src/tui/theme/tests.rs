use super::*;

#[test]
fn styles_paint_attributes_before_colors() {
    assert_eq!(BODY.to_string(), "38;2;209;210;214");
    assert_eq!(
        STRONG.bold().italic().strike().to_string(),
        "1;3;9;38;2;239;240;243"
    );
    assert_eq!(LINK.bold().to_string(), "1;4;38;2;199;175;255");
    assert_eq!(BODY.italic().strike().to_string(), "3;9;38;2;209;210;214");
    assert_eq!(CURSOR.to_string(), "38;2;12;12;12;48;2;239;240;243");
    assert_eq!(BASE.to_string(), "49");
    assert_eq!(USER_SURFACE.to_string(), "48;2;41;42;47");
    assert_eq!(SELECTED_SURFACE.to_string(), "48;2;58;59;66");
    assert_eq!(CODE_SURFACE.to_string(), "48;2;33;34;38");
    assert_eq!(FAINT.to_string(), "38;2;109;111;114");
}

#[test]
fn roles_paint_the_chosen_terminal_and_only_a_change_repaints() {
    assert!(!set_light(false), "dark is the default");
    assert!(set_light(true));
    assert!(!set_light(true));
    assert_eq!(BODY.to_string(), "38;2;44;46;49");
    assert_eq!(CURSOR.to_string(), "38;2;255;255;255;48;2;25;27;29");
    assert_eq!(USER_SURFACE.to_string(), "48;2;230;232;236");
    // Family surfaces read the same on either terminal; the base stays the terminal's.
    assert_eq!(BADGE_EDIT.to_string(), "48;2;127;60;147");
    assert_eq!(BADGE_TEXT.to_string(), "38;2;239;240;243");
    assert_eq!(BASE.to_string(), "49");
    assert!(set_light(false));
    assert_eq!(BODY.to_string(), "38;2;209;210;214");
}

fn luminance(Rgb(red, green, blue): Rgb) -> f64 {
    let linear = |channel: u8| {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
}

fn ratio(fore: Rgb, back: Rgb) -> f64 {
    let (fore, back) = (luminance(fore), luminance(back));
    (fore.max(back) + 0.05) / (fore.min(back) + 0.05)
}

// The worst of both palettes: each ink on what it sits on there.
fn contrast(foreground: Style, surface: Surface) -> f64 {
    let (fore, back) = (foreground.foreground.unwrap(), surface.0.unwrap());
    ratio(fore.dark, back.dark).min(ratio(fore.light, back.light))
}

#[test]
fn text_roles_hold_their_contrast_on_their_own_terminals() {
    // Lighter dark themes such as One Half Dark (#282C34) drop DIM and FAINT below
    // the floors.
    let dark = [0x0C0C0C, 0x17161F, 0x1E1E1E].map(Rgb::hex);
    let light = [0xFFFFFF, 0xFAFAFA, 0xFDF6E3].map(Rgb::hex);
    let holds = |role: Style, floor: f64| {
        let ink = role.foreground.unwrap();
        for terminal in dark {
            assert!(ratio(ink.dark, terminal) >= floor, "{role:?} {terminal:?}");
        }
        for terminal in light {
            assert!(ratio(ink.light, terminal) >= floor, "{role:?} {terminal:?}");
        }
    };
    for (role, floor) in [
        (STRONG, 13.0),
        (BODY, 11.0),
        (ACCENT, 8.5),
        (MUTED, 6.5),
        (DIM, 5.0),
        (GOOD, 4.5),
        (BAD, 4.5),
        (WARNING, 4.5),
        // Marks meet the 3:1 floor for non-text contrast.
        (FAINT, 3.0),
    ] {
        holds(role, floor);
    }
}

#[test]
fn every_label_holds_its_contrast_on_the_surfaces_it_uses() {
    for (foreground, surface) in [
        (BADGE_TEXT, BADGE_READ),
        (BADGE_TEXT, BADGE_EDIT),
        (BADGE_TEXT, BADGE_SHELL),
        (BADGE_TEXT, BADGE_COORD),
        (BADGE_TEXT, BADGE_BAD),
        (BADGE_TEXT, BADGE_WARN),
        (STRONG, BADGE_IDLE),
        (LIT, BADGE_IDLE),
        (STRONG, USER_SURFACE),
        (STRONG, SELECTED_SURFACE),
        (STRONG, CODE_SURFACE),
        (ACCENT, USER_SURFACE),
        (ACCENT, SELECTED_SURFACE),
        (MUTED, SELECTED_SURFACE),
        (GOOD, SELECTED_SURFACE),
        (WARNING, SELECTED_SURFACE),
        (BAD, SELECTED_SURFACE),
        (MUTED, CODE_SURFACE),
        (CODE_TEXT, CODE_SURFACE),
        (KEYWORD, CODE_SURFACE),
        (CODE_STRING, CODE_SURFACE),
        (CODE_NUMBER, CODE_SURFACE),
        (DIM, DIFF_ADDED_SURFACE),
        (DIM, DIFF_REMOVED_SURFACE),
        (CODE_TEXT, DIFF_ADDED_SURFACE),
        (CODE_TEXT, DIFF_REMOVED_SURFACE),
        (KEYWORD, DIFF_ADDED_SURFACE),
        (KEYWORD, DIFF_REMOVED_SURFACE),
        (CODE_STRING, DIFF_ADDED_SURFACE),
        (CODE_STRING, DIFF_REMOVED_SURFACE),
        (CODE_NUMBER, DIFF_ADDED_SURFACE),
        (CODE_NUMBER, DIFF_REMOVED_SURFACE),
        (GOOD, DIFF_ADDED_SURFACE),
        (GOOD, DIFF_ADDED_STRONG),
        (BAD, DIFF_REMOVED_SURFACE),
        (BAD, DIFF_REMOVED_STRONG),
    ] {
        assert!(
            contrast(foreground, surface) >= 4.5,
            "{foreground:?} on {surface:?}"
        );
    }
    // A running badge rests on dim between sweeps of the band.
    assert!(contrast(DIM, BADGE_IDLE) >= 3.0);
}

fn palette_color(index: u8) -> Rgb {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        16..=231 => {
            let index = usize::from(index - 16);
            Rgb(LEVELS[index / 36], LEVELS[index / 6 % 6], LEVELS[index % 6])
        }
        232..=255 => {
            let grey = 8 + 10 * (index - 232);
            Rgb(grey, grey, grey)
        }
        _ => panic!("the palette mapping must avoid the themeable first 16 colours"),
    }
}

#[test]
fn colors_map_to_the_nearest_stable_palette_entry() {
    for (color, index) in [
        (Rgb(0, 0, 0), 16),
        (Rgb(255, 255, 255), 231),
        (Rgb(135, 95, 255), 99),
        (Rgb(8, 8, 8), 232),
        (Rgb(38, 38, 38), 235),
        (Rgb(238, 238, 238), 255),
    ] {
        assert_eq!(color.nearest_palette_index(), index, "{color:?}");
    }
    assert_eq!(Rgb(199, 175, 255).parameters(38, true), "38;5;183");
    assert_eq!(Rgb(199, 175, 255).parameters(48, false), "48;2;199;175;255");
}

#[test]
fn the_palette_keeps_text_legible_on_every_surface() {
    let mapped = |color: Rgb| palette_color(color.nearest_palette_index());
    let contrast_after = |foreground: Style, surface: Surface| {
        let (fore, back) = (foreground.foreground.unwrap(), surface.0.unwrap());
        ratio(mapped(fore.dark), mapped(back.dark))
            .min(ratio(mapped(fore.light), mapped(back.light)))
    };
    for (foreground, surface) in [
        (BODY, USER_SURFACE),
        (CODE_TEXT, CODE_SURFACE),
        (CODE_TEXT, DIFF_ADDED_SURFACE),
        (CODE_TEXT, DIFF_REMOVED_SURFACE),
        (MUTED, DIFF_ADDED_SURFACE),
        (MUTED, DIFF_REMOVED_SURFACE),
        (GOOD, DIFF_ADDED_STRONG),
        (BAD, DIFF_REMOVED_STRONG),
        (STRONG, SELECTED_SURFACE),
        (LIT, BADGE_IDLE),
        (BADGE_TEXT, BADGE_READ),
        (BADGE_TEXT, BADGE_EDIT),
        (BADGE_TEXT, BADGE_SHELL),
        (BADGE_TEXT, BADGE_COORD),
        (BADGE_TEXT, BADGE_BAD),
        (BADGE_TEXT, BADGE_WARN),
    ] {
        // The palette cannot hold the 24-bit floors exactly: 4 for text, and 3
        // for muted text on an added row.
        let added = foreground == MUTED && surface == DIFF_ADDED_SURFACE;
        let floor = if added { 3.0 } else { 4.0 };
        let ratio = contrast_after(foreground, surface);
        assert!(ratio >= floor, "{foreground:?} on {surface:?}: {ratio:.2}");
    }
    // Rows keep a visible surface instead of falling to the terminal's background.
    for surface in [
        USER_SURFACE,
        CODE_SURFACE,
        SELECTED_SURFACE,
        DIFF_ADDED_SURFACE,
        DIFF_REMOVED_SURFACE,
    ] {
        let ink = surface.0.unwrap();
        assert_ne!(mapped(ink.dark), Rgb(0, 0, 0), "{surface:?}");
        assert_ne!(mapped(ink.light), Rgb(255, 255, 255), "{surface:?}");
    }
    // Colour still separates added from removed, and failure from ordinary tools.
    let ink = |surface: Surface| surface.0.unwrap();
    assert_ne!(
        mapped(ink(DIFF_ADDED_SURFACE).dark),
        mapped(ink(DIFF_REMOVED_SURFACE).dark)
    );
    assert_ne!(
        mapped(ink(DIFF_ADDED_SURFACE).light),
        mapped(ink(DIFF_REMOVED_SURFACE).light)
    );
    assert_ne!(mapped(ink(BADGE_BAD).dark), mapped(ink(BADGE_SHELL).dark));
}

#[test]
fn only_a_declared_terminal_receives_true_color() {
    let declares = |pairs: &[(&str, &str)]| {
        declares_true_color(|name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    };
    for pairs in [
        &[("COLORTERM", "truecolor")][..],
        &[("COLORTERM", "24bit"), ("TERM", "xterm")],
        &[("TERM", "xterm-kitty")],
        &[("TERM", "xterm-direct")],
        &[("TERM_PROGRAM", "iTerm.app")],
        &[("TERM_PROGRAM", "vscode")],
        &[("WT_SESSION", "id")],
        &[("VTE_VERSION", "7600")],
    ] {
        assert!(declares(pairs), "{pairs:?}");
    }
    for pairs in [
        &[][..],
        &[("TERM", "xterm-256color")],
        &[
            ("TERM", "xterm-256color"),
            ("TERM_PROGRAM", "Apple_Terminal"),
        ],
        &[("TERM", "screen-256color")],
        &[("TERM", "linux")],
        &[("COLORTERM", "yes")],
        &[("VTE_VERSION", "3400")],
    ] {
        assert!(!declares(pairs), "{pairs:?}");
    }
}

#[test]
fn changed_fragments_paint_their_own_surface() {
    assert_eq!(DIFF_ADDED_STRONG.to_string(), "48;2;44;74;51");
    assert_eq!(DIFF_REMOVED_STRONG.to_string(), "48;2;82;48;57");
}
