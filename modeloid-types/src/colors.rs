//! Class colours: each dataset gives its classes distinct colours, taken in
//! turn from a 64-colour palette running once round the hue circle (big
//! steps, so neighbours differ a lot), starting at a colour picked per
//! dataset. The colour picker offers every fourth palette colour (`presets`). Kept by class name, so
//! reordering classes doesn't change them; the user can change them in
//! Classes & Merge.

use std::collections::{BTreeMap, BTreeSet};

/// Colour (`#rrggbb`) by class name.
pub type ClassColors = BTreeMap<String, String>;

/// Shown for a class without a colour (never stored).
pub const FALLBACK: &str = "#888888";

const GOLDEN_ANGLE: f64 = 137.507_764;

/// Colours in `palette`.
pub const PALETTE_SIZE: usize = 64;
/// Colours the picker offers (`presets`).
pub const PRESET_COUNT: usize = 16;
/// Palette steps between two classes' colours: about 0.38 of the circle,
/// and coprime with 64, so 64 classes use every colour once.
const PALETTE_STEP: usize = 25;

/// 64 colours from red round the hue circle back towards red, evenly spaced.
pub fn palette() -> Vec<String> {
    (0..PALETTE_SIZE).map(|i| hsl_hex(i as f64 * 360.0 / PALETTE_SIZE as f64, 0.75, 0.60)).collect()
}

/// The picker's 16 colours: every fourth palette colour.
pub fn presets() -> Vec<String> {
    palette().into_iter().step_by(PALETTE_SIZE / PRESET_COUNT).collect()
}

/// A seed from a name (FNV-1a), e.g. the dataset's, so its start hue looks
/// random but stays the same.
pub fn seed_of(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Colour number `step` of the sequence for `seed`: palette colours first
/// (from a start picked by the seed), then, past 64 classes, hues between them.
pub fn spectrum(seed: u64, step: usize) -> String {
    let start = splitmix(seed) as usize % PALETTE_SIZE;
    if step < PALETTE_SIZE {
        let i = (start + step * PALETTE_STEP) % PALETTE_SIZE;
        // Bright and saturated, so boxes show on photos in every theme.
        return hsl_hex(i as f64 * 360.0 / PALETTE_SIZE as f64, 0.75, 0.60);
    }
    let hue = (start as f64 * 360.0 / PALETTE_SIZE as f64 + 2.8125 + step as f64 * GOLDEN_ANGLE) % 360.0;
    hsl_hex(hue, 0.75, 0.52)
}

/// Colours for `classes`: their colour in `existing` when they have a valid
/// one, otherwise the next colours of the sequence not used yet. Classes
/// not in `classes` are left out.
pub fn assign(classes: &[String], existing: &ClassColors, seed: u64) -> ClassColors {
    let mut out = ClassColors::new();
    for class in classes {
        if let Some(color) = existing.get(class).and_then(|c| normalize(c)) {
            out.insert(class.clone(), color);
        }
    }
    let mut used: BTreeSet<String> = out.values().cloned().collect();
    let mut step = 0;
    for class in classes {
        if out.contains_key(class) {
            continue;
        }
        let color = loop {
            let color = spectrum(seed, step);
            step += 1;
            if !used.contains(&color) {
                break color;
            }
        };
        used.insert(color.clone());
        out.insert(class.clone(), color);
    }
    out
}

/// The colours of `classes` in their order.
pub fn in_order(classes: &[String], colors: &ClassColors) -> Vec<String> {
    classes.iter().map(|c| colors.get(c).cloned().unwrap_or_else(|| FALLBACK.to_string())).collect()
}

/// `#rrggbb` in lower case, or `None` when `color` isn't one.
pub fn normalize(color: &str) -> Option<String> {
    let color = color.trim();
    let hex = color.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", hex.to_ascii_lowercase()))
}

fn splitmix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn hsl_hex(hue: f64, saturation: f64, lightness: f64) -> String {
    let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let m = lightness - c / 2.0;
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn colours_are_distinct_and_stable() {
        let classes: Vec<String> = (0..60).map(|i| format!("c{i}")).collect();
        let colors = assign(&classes, &ClassColors::new(), 7);
        let distinct: BTreeSet<&String> = colors.values().collect();
        assert_eq!(distinct.len(), 60);
        assert!(colors.values().all(|c| normalize(c).as_deref() == Some(c.as_str())));
        assert_eq!(assign(&classes, &ClassColors::new(), 7), colors, "same seed, same colours");
        assert_ne!(assign(&classes, &ClassColors::new(), 8), colors, "another seed starts elsewhere");
    }

    #[test]
    fn keeps_existing_and_skips_used() {
        let first = spectrum(3, 0);
        let existing = ClassColors::from([("dog".to_string(), first.clone()), ("gone".to_string(), "#123456".to_string())]);
        let colors = assign(&names(&["cat", "dog"]), &existing, 3);
        assert_eq!(colors["dog"], first);
        assert_eq!(colors["cat"], spectrum(3, 1), "the dog's colour is taken");
        assert!(!colors.contains_key("gone"));
        assert_eq!(in_order(&names(&["dog", "x"]), &colors), vec![first, FALLBACK.to_string()]);
    }

    #[test]
    fn palette_and_presets() {
        let palette = palette();
        assert_eq!(palette.len(), PALETTE_SIZE);
        assert_eq!(palette.iter().collect::<BTreeSet<_>>().len(), PALETTE_SIZE, "all different");
        let presets = presets();
        assert_eq!(presets.len(), PRESET_COUNT);
        assert_eq!((&presets[0], &presets[1]), (&palette[0], &palette[4]));
        // The first 64 classes get palette colours, each once.
        let used: BTreeSet<String> = (0..PALETTE_SIZE).map(|s| spectrum(9, s)).collect();
        assert_eq!(used, palette.into_iter().collect());
        assert!(!used.contains(&spectrum(9, PALETTE_SIZE)), "the 65th is a new colour");
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize(" #AbC123 ").as_deref(), Some("#abc123"));
        assert_eq!(normalize("abc123"), None);
        assert_eq!(normalize("#abc"), None);
        assert_eq!(hsl_hex(0.0, 1.0, 0.5), "#ff0000");
        assert_eq!(hsl_hex(240.0, 1.0, 0.5), "#0000ff");
    }
}
