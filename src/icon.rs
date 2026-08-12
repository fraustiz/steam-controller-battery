//! Rendu de l'icône, composé à partir des masques de [`crate::icons`].
//!
//! Les formes viennent de Material Symbols, rastérisées hors ligne à chacune
//! des tailles que Windows réclame. Ce module n'a donc plus à dessiner de
//! géométrie : il choisit les pièces, leur donne une couleur, et les empile.
//!
//! La couleur est décidée ici plutôt que figée dans les masques, pour deux
//! raisons. Le cadre et l'éclair doivent suivre le thème du système, sans quoi
//! ils disparaîtraient sur une barre des tâches claire. Et le barreau de niveau
//! doit pouvoir changer de teinte sans qu'on ait à regraver une image.

use std::ffi::c_void;

use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

use crate::icons;
use crate::state::IconState;

/// Tailles auxquelles les masques ont été dessinés.
///
/// Ce sont celles que Windows demande à 100, 125, 150 et 200 % de mise à
/// l'échelle. Toute autre taille est ramenée à la plus proche : mieux vaut une
/// icône nette d'un pixel trop petite qu'une icône redimensionnée et floue.
pub const BAKED_SIZES: [u32; 4] = [16, 20, 24, 32];

/// Couleur du barreau, du plus vide au plus plein.
const BAR_COLOURS: [(u8, u8, u8); 6] = [
    (0xE6, 0x54, 0x38), // rouge
    (0xEF, 0x9A, 0x28), // ambre
    (0xDF, 0xB5, 0x28), // jaune
    (0xA4, 0xBD, 0x32), // vert-jaune
    (0x4E, 0xB4, 0x41), // vert
    (0x3D, 0xA6, 0x38), // vert soutenu
];

/// Bornes hautes de chaque palier.
///
/// Les paliers hauts sont larges parce qu'ils importent peu : entre 95 et 75 %
/// la nuance n'intéresse personne. Les bas sont resserrés, puisque c'est là
/// qu'on regarde l'icône. Le passage au rouge tombe juste avant le premier
/// seuil de notification, de sorte que la couleur prévient avant le message.
const BAR_LIMITS: [u8; 5] = [15, 30, 50, 70, 88];

/// Le biseau de la manette en charge garde une teinte fixe.
///
/// Il ne peut pas représenter un niveau — sa forme est constante — et le teinter
/// selon la charge ferait croire à une mesure qu'il ne porte pas.
const CHARGE_WEDGE: (u8, u8, u8) = (0x3D, 0xA6, 0x38);

/// Le palier correspondant à un niveau.
pub fn bar_index(percent: u8) -> usize {
    BAR_LIMITS.iter().position(|&limit| percent <= limit).unwrap_or(BAR_LIMITS.len())
}

/// Encre des pièces de structure — cadre, téton, prise barrée.
pub fn structure_colour(dark_theme: bool) -> (u8, u8, u8) {
    if dark_theme {
        (0xE3, 0xE3, 0xE3)
    } else {
        (0x3A, 0x3A, 0x3A)
    }
}

/// Encre de l'éclair. Un jaune franc sur fond sombre ; le même, assombri en
/// ocre, sur fond clair, où un jaune vif serait illisible.
pub fn bolt_colour(dark_theme: bool) -> (u8, u8, u8) {
    if dark_theme {
        (0xFF, 0xF5, 0x00)
    } else {
        (0xB3, 0x8F, 0x00)
    }
}

/// Les pièces disponibles à une taille donnée.
struct Art {
    frame: &'static [u8],
    nub: &'static [u8],
    bars: [&'static [u8]; 6],
    bolt_frame: &'static [u8],
    bolt: &'static [u8],
    wedge: &'static [u8],
    off: [&'static [u8]; 2],
}

/// Construit le jeu de pièces d'une taille. Une branche par taille : les noms
/// des masques sont engendrés, les assembler par macro générative coûterait
/// plus de lisibilité que de lignes économisées.
macro_rules! art_for {
    (16) => {
        Art {
            frame: &icons::FRAME_16,
            nub: &icons::NUB_16,
            bars: [
                &icons::BAR1_16, &icons::BAR2_16, &icons::BAR3_16,
                &icons::BAR4_16, &icons::BAR6_16, &icons::BARFULL_16,
            ],
            bolt_frame: &icons::BOLTFRAME_16,
            bolt: &icons::BOLT_16,
            wedge: &icons::CHARGEWEDGE_16,
            off: [&icons::OFF1_16, &icons::OFF2_16],
        }
    };
    (20) => {
        Art {
            frame: &icons::FRAME_20,
            nub: &icons::NUB_20,
            bars: [
                &icons::BAR1_20, &icons::BAR2_20, &icons::BAR3_20,
                &icons::BAR4_20, &icons::BAR6_20, &icons::BARFULL_20,
            ],
            bolt_frame: &icons::BOLTFRAME_20,
            bolt: &icons::BOLT_20,
            wedge: &icons::CHARGEWEDGE_20,
            off: [&icons::OFF1_20, &icons::OFF2_20],
        }
    };
    (24) => {
        Art {
            frame: &icons::FRAME_24,
            nub: &icons::NUB_24,
            bars: [
                &icons::BAR1_24, &icons::BAR2_24, &icons::BAR3_24,
                &icons::BAR4_24, &icons::BAR6_24, &icons::BARFULL_24,
            ],
            bolt_frame: &icons::BOLTFRAME_24,
            bolt: &icons::BOLT_24,
            wedge: &icons::CHARGEWEDGE_24,
            off: [&icons::OFF1_24, &icons::OFF2_24],
        }
    };
    (32) => {
        Art {
            frame: &icons::FRAME_32,
            nub: &icons::NUB_32,
            bars: [
                &icons::BAR1_32, &icons::BAR2_32, &icons::BAR3_32,
                &icons::BAR4_32, &icons::BAR6_32, &icons::BARFULL_32,
            ],
            bolt_frame: &icons::BOLTFRAME_32,
            bolt: &icons::BOLT_32,
            wedge: &icons::CHARGEWEDGE_32,
            off: [&icons::OFF1_32, &icons::OFF2_32],
        }
    };
}

/// La taille dessinée la plus proche de celle demandée.
pub fn snap_size(requested: u32) -> u32 {
    *BAKED_SIZES
        .iter()
        .min_by_key(|&&s| s.abs_diff(requested))
        .unwrap_or(&16)
}

fn art(size: u32) -> Art {
    match size {
        20 => art_for!(20),
        24 => art_for!(24),
        32 => art_for!(32),
        _ => art_for!(16),
    }
}

/// Composition « source par-dessus », en alpha non prémultiplié.
fn blend(dst: &mut u32, colour: (u8, u8, u8), alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let a = alpha.min(1.0);
    let (dr, dg, db) = (
        ((*dst >> 16) & 0xFF) as f32,
        ((*dst >> 8) & 0xFF) as f32,
        (*dst & 0xFF) as f32,
    );
    let da = ((*dst >> 24) & 0xFF) as f32 / 255.0;
    let out_a = a + da * (1.0 - a);
    if out_a <= 0.0 {
        *dst = 0;
        return;
    }
    let mix = |s: u8, d: f32| {
        ((s as f32 * a + d * da * (1.0 - a)) / out_a)
            .round()
            .clamp(0.0, 255.0) as u32
    };
    *dst = (((out_a * 255.0).round() as u32) << 24)
        | (mix(colour.0, dr) << 16)
        | (mix(colour.1, dg) << 8)
        | mix(colour.2, db);
}

/// Pose une pièce dans la couleur demandée.
fn paint(px: &mut [u32], mask: &[u8], colour: (u8, u8, u8)) {
    for (dst, &a) in px.iter_mut().zip(mask.iter()) {
        if a > 0 {
            blend(dst, colour, a as f32 / 255.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Le nombre, pour l'affichage chiffré
// ---------------------------------------------------------------------------

/// Chiffres, trois pixels de large sur cinq de haut.
///
/// Dessinés à la main plutôt que confiés à GDI : à cette taille, un rendu de
/// texte antialiasé étale chaque trait sur deux pixels gris.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

const DIGIT_W: u32 = 3;
const DIGIT_H: u32 = 5;
const DIGIT_SPACING: u32 = 1;

fn digits_mask(value: u8, scale: u32) -> (Vec<bool>, u32, u32) {
    let text: Vec<u8> = value.to_string().bytes().map(|b| b - b'0').collect();
    let n = text.len() as u32;
    let w = (n * DIGIT_W + (n - 1) * DIGIT_SPACING) * scale;
    let h = DIGIT_H * scale;
    let mut mask = vec![false; (w * h) as usize];

    for (i, d) in text.iter().enumerate() {
        let glyph = &DIGITS[*d as usize % 10];
        let ox = i as u32 * (DIGIT_W + DIGIT_SPACING) * scale;
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..DIGIT_W {
                if bits & (1 << (DIGIT_W - 1 - col)) == 0 {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        let x = ox + col * scale + sx;
                        let y = row as u32 * scale + sy;
                        mask[(y * w + x) as usize] = true;
                    }
                }
            }
        }
    }
    (mask, w, h)
}

/// Dessine le pourcentage seul, aussi gros que l'icône le permet.
///
/// Il remplace l'icône au lieu de s'y ajouter. À seize pixels, le corps d'une
/// batterie ne laisse que quelques pixels utiles, où deux chiffres deviennent
/// des taches ; sans cadre, ils occupent quatre fois la surface. C'est la
/// couleur du nombre qui porte alors le niveau.
fn draw_percent(px: &mut [u32], size: u32, percent: u8, dark_theme: bool) {
    let percent = percent.min(100);
    let digits = match percent {
        0..=9 => 1u32,
        10..=99 => 2,
        _ => 3,
    };
    let margin = (size / 16).max(1);
    let avail = size - 2 * margin;
    let scale = (1..=8u32)
        .rev()
        .find(|&sc| {
            (digits * DIGIT_W + (digits - 1) * DIGIT_SPACING) * sc <= avail && DIGIT_H * sc <= avail
        })
        .unwrap_or(1);

    let (mask, w, h) = digits_mask(percent, scale);
    if w > size || h > size {
        return;
    }
    let (ox, oy) = ((size - w) / 2, (size - h) / 2);

    let colour = {
        let c = BAR_COLOURS[bar_index(percent)];
        if dark_theme {
            c
        } else {
            // Les tons vifs du niveau passent mal sur une barre claire.
            let f = |v: u8| (v as f32 * 0.72).round() as u8;
            (f(c.0), f(c.1), f(c.2))
        }
    };

    for y in 0..h {
        for x in 0..w {
            if mask[(y * w + x) as usize] {
                blend(&mut px[((oy + y) * size + ox + x) as usize], colour, 1.0);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

/// Dessine l'icône dans un tampon de pixels, sans toucher à GDI.
pub fn draw(px: &mut [u32], size: u32, state: IconState, show_percent: bool, dark_theme: bool) {
    // Le mode chiffré remplace tout : voir `draw_percent`.
    if show_percent {
        if let IconState::Battery(s) = state {
            draw_percent(px, size, s.percent, dark_theme);
            return;
        }
    }

    let a = art(size);
    let structure = structure_colour(dark_theme);

    match state {
        IconState::Battery(s) if s.charging => {
            paint(px, a.bolt_frame, structure);
            paint(px, a.wedge, CHARGE_WEDGE);
            paint(px, a.bolt, bolt_colour(dark_theme));
        }
        IconState::Battery(s) => {
            paint(px, a.frame, structure);
            paint(px, a.nub, structure);
            paint(px, a.bars[bar_index(s.percent)], BAR_COLOURS[bar_index(s.percent)]);
        }
        // Éteinte sur son socle : elle charge, mais son niveau est hors
        // d'atteinte. On montre donc l'éclair sans le moindre barreau, plutôt
        // qu'un niveau inventé.
        IconState::Docked => {
            paint(px, a.bolt_frame, structure);
            paint(px, a.bolt, bolt_colour(dark_theme));
        }
        IconState::Disconnected => {
            for piece in a.off {
                paint(px, piece, structure);
            }
        }
    }
}

/// Construit l'icône. Le résultat appartient à l'appelant, qui doit la libérer
/// par `DestroyIcon`.
pub fn render(state: IconState, size: u32, show_percent: bool) -> HICON {
    let size = snap_size(size);
    let n = (size * size) as usize;
    let dark = !system_uses_light_theme();

    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size as i32;
        bmi.bmiHeader.biHeight = -(size as i32); // de haut en bas
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let old = SelectObject(dc, dib as HGDIOBJ);

        let px = std::slice::from_raw_parts_mut(bits as *mut u32, n);
        px.fill(0);
        draw(px, size, state, show_percent, dark);

        // Le masque monochrome n'est pas consulté pour une icône 32 bits avec
        // couche alpha, mais `CreateIconIndirect` en exige un.
        let empty = vec![0u8; (size as usize).div_ceil(8) * size as usize];
        let mono = CreateBitmap(size as i32, size as i32, 1, 1, empty.as_ptr() as *const c_void);

        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mono, hbmColor: dib };
        let icon = CreateIconIndirect(&info);

        SelectObject(dc, old);
        DeleteObject(mono as HGDIOBJ);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(dc);
        icon
    }
}

fn system_uses_light_theme() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    let wide =
        |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect() };
    let path = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("SystemUsesLightTheme");

    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key) != ERROR_SUCCESS {
            return false; // à défaut, on suppose le thème sombre, le plus répandu
        }
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let rc = RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut u8,
            &mut size,
        );
        RegCloseKey(key);
        rc == ERROR_SUCCESS && value == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hid::BatteryStatus;

    fn at(percent: u8, charging: bool) -> BatteryStatus {
        BatteryStatus { percent, voltage_mv: None, charging, full: false }
    }

    fn battery(percent: u8, charging: bool) -> IconState {
        IconState::Battery(at(percent, charging))
    }

    fn buffer(size: u32, state: IconState, percent_mode: bool) -> Vec<u32> {
        let mut px = vec![0u32; (size * size) as usize];
        draw(&mut px, size, state, percent_mode, true);
        px
    }

    fn opaque(px: &[u32]) -> usize {
        px.iter().filter(|p| (*p >> 24) & 0xFF > 40).count()
    }

    /// Planche de contrôle : tous les états, aux deux thèmes, à taille réelle
    /// et agrandis. Le rendu d'une icône de seize pixels ne se juge pas sur des
    /// assertions.
    ///
    /// `cargo test -- --ignored render_preview_sheet`
    #[test]
    #[ignore = "génère un fichier à inspecter"]
    fn render_preview_sheet() {
        let cases: Vec<(&str, IconState, bool)> = vec![
            ("100", battery(100, false), false),
            ("80", battery(80, false), false),
            ("60", battery(60, false), false),
            ("40", battery(40, false), false),
            ("25", battery(25, false), false),
            ("8", battery(8, false), false),
            ("charge", battery(60, true), false),
            ("socle", IconState::Docked, false),
            ("absente", IconState::Disconnected, false),
            ("chiffre", battery(72, false), true),
        ];
        let backgrounds = [((0x20u8, 0x20u8, 0x20u8), true), ((0xF3, 0xF3, 0xF3), false)];

        const ICON: u32 = 16;
        const ZOOM: u32 = 9;
        const PAD: u32 = 8;
        let cell = ICON * ZOOM + PAD * 2;
        let strip = ICON + PAD * 2;
        let w = cell * cases.len() as u32;
        let h = (cell + strip) * backgrounds.len() as u32;
        let mut img = vec![(0u8, 0u8, 0u8); (w * h) as usize];

        let over = |bg: (u8, u8, u8), p: u32| {
            let a = ((p >> 24) & 0xFF) as f32 / 255.0;
            let mix = |s: u32, d: u8| ((s as f32) * a + d as f32 * (1.0 - a)).round() as u8;
            (mix((p >> 16) & 0xFF, bg.0), mix((p >> 8) & 0xFF, bg.1), mix(p & 0xFF, bg.2))
        };

        for (bi, (bg, dark)) in backgrounds.iter().enumerate() {
            let top = bi as u32 * (cell + strip);
            for y in top..(top + cell + strip).min(h) {
                for x in 0..w {
                    img[(y * w + x) as usize] = *bg;
                }
            }
            for (ci, (_, state, pc)) in cases.iter().enumerate() {
                let mut buf = vec![0u32; (ICON * ICON) as usize];
                draw(&mut buf, ICON, *state, *pc, *dark);
                let ox = ci as u32 * cell + PAD;
                for y in 0..ICON * ZOOM {
                    for x in 0..ICON * ZOOM {
                        let p = buf[((y / ZOOM) * ICON + x / ZOOM) as usize];
                        img[((top + PAD + y) * w + ox + x) as usize] = over(*bg, p);
                    }
                }
                for y in 0..ICON {
                    for x in 0..ICON {
                        let p = buf[(y * ICON + x) as usize];
                        img[((top + cell + PAD + y) * w + ox + x) as usize] = over(*bg, p);
                    }
                }
            }
        }

        let row = (w * 3).next_multiple_of(4) as usize;
        let mut out = Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&((54 + row * h as usize) as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(h as i32).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 24]);
        for y in (0..h).rev() {
            let start = out.len();
            for x in 0..w {
                let (r, g, b) = img[(y * w + x) as usize];
                out.extend_from_slice(&[b, g, r]);
            }
            out.resize(start + row, 0);
        }
        let path = std::env::temp_dir().join("sc-battery-preview.bmp");
        std::fs::write(&path, &out).expect("écriture");
        println!("planche écrite : {}", path.display());
    }

    #[test]
    fn every_mask_matches_its_size() {
        for size in BAKED_SIZES {
            let a = art(size);
            let n = (size * size) as usize;
            let mut pieces: Vec<&[u8]> =
                vec![a.frame, a.nub, a.bolt_frame, a.bolt, a.wedge, a.off[0], a.off[1]];
            pieces.extend_from_slice(&a.bars);
            for piece in pieces {
                assert_eq!(piece.len(), n, "pièce mal dimensionnée à {size} px");
            }
        }
    }

    #[test]
    fn no_mask_is_empty() {
        // Une pièce vide passerait inaperçue à l'exécution : l'icône
        // s'afficherait simplement incomplète, sans erreur.
        for size in BAKED_SIZES {
            let a = art(size);
            let mut named: Vec<(&str, &[u8])> = vec![
                ("cadre", a.frame),
                ("téton", a.nub),
                ("cadre éclair", a.bolt_frame),
                ("éclair", a.bolt),
                ("biseau", a.wedge),
                ("prise 1", a.off[0]),
                ("prise 2", a.off[1]),
            ];
            for (i, bar) in a.bars.iter().enumerate() {
                named.push((["b1", "b2", "b3", "b4", "b6", "bfull"][i], bar));
            }
            for (label, mask) in named {
                assert!(
                    mask.iter().any(|&v| v > 20),
                    "pièce « {label} » vide à {size} px"
                );
            }
        }
    }

    #[test]
    fn the_thresholds_cover_the_whole_range_in_order() {
        let mut previous = 0;
        for p in 0..=100u8 {
            let i = bar_index(p);
            assert!(i < BAR_COLOURS.len(), "palier hors bornes à {p} %");
            assert!(i >= previous, "les paliers reculent à {p} %");
            previous = i;
        }
        assert_eq!(bar_index(0), 0, "0 % doit être au palier le plus bas");
        assert_eq!(bar_index(100), 5, "100 % doit être au palier le plus haut");
        // Le rouge doit céder avant la première notification, à 20 %.
        assert!(bar_index(19) > 0, "encore rouge à 19 %, après le seuil d'alerte");
        assert_eq!(bar_index(15), 0, "15 % doit encore être rouge");
    }

    #[test]
    fn the_bar_colours_run_from_red_to_green() {
        let first = BAR_COLOURS[0];
        let last = BAR_COLOURS[BAR_COLOURS.len() - 1];
        assert!(first.0 > first.1, "le palier bas doit tirer sur le rouge");
        assert!(last.1 > last.0, "le palier haut doit tirer sur le vert");
        // Aucune répétition : chaque palier doit se distinguer du précédent.
        for w in BAR_COLOURS.windows(2) {
            assert_ne!(w[0], w[1], "deux paliers de même couleur");
        }
    }

    #[test]
    fn the_four_states_are_visually_distinct() {
        let states = [
            battery(50, false),
            battery(50, true),
            IconState::Docked,
            IconState::Disconnected,
        ];
        for (i, a) in states.iter().enumerate() {
            for b in states.iter().skip(i + 1) {
                assert_ne!(
                    buffer(16, *a, false),
                    buffer(16, *b, false),
                    "deux états se dessinent pareil : {a:?} et {b:?}"
                );
            }
        }
    }

    #[test]
    fn charging_shows_the_bolt_and_docked_shows_no_level() {
        // Sur le socle, le niveau est inconnu : rien ne doit le suggérer.
        let docked = buffer(16, IconState::Docked, false);
        let charging = buffer(16, battery(50, true), false);
        assert!(opaque(&charging) > opaque(&docked), "le biseau doit ajouter de la matière");
    }

    #[test]
    fn each_level_paints_its_own_bar() {
        let mut seen: Vec<Vec<u32>> = Vec::new();
        for p in [5u8, 25, 40, 60, 80, 95] {
            let px = buffer(16, battery(p, false), false);
            assert!(!seen.contains(&px), "deux niveaux dessinent la même icône ({p} %)");
            seen.push(px);
        }
    }

    #[test]
    fn sizes_snap_to_a_baked_one() {
        for requested in 12..=70u32 {
            let s = snap_size(requested);
            assert!(BAKED_SIZES.contains(&s), "{requested} ramené à {s}, non dessiné");
        }
        assert_eq!(snap_size(16), 16);
        assert_eq!(snap_size(24), 24);
        assert_eq!(snap_size(30), 32);
    }

    #[test]
    fn the_theme_changes_the_structure_but_never_the_level() {
        // Le cadre doit s'adapter au fond ; la couleur du niveau est une
        // information, pas une décoration, et ne doit pas bouger.
        assert_ne!(structure_colour(true), structure_colour(false));
        assert_ne!(bolt_colour(true), bolt_colour(false));

        let mut dark = vec![0u32; 16 * 16];
        let mut light = vec![0u32; 16 * 16];
        draw(&mut dark, 16, battery(50, false), false, true);
        draw(&mut light, 16, battery(50, false), false, false);
        assert_ne!(dark, light, "le cadre doit suivre le thème");

        let bar = BAR_COLOURS[bar_index(50)];
        let has_bar = |px: &[u32]| {
            px.iter().any(|p| {
                let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                (r as u8, g as u8, b as u8) == bar
            })
        };
        assert!(has_bar(&dark) && has_bar(&light), "la teinte du niveau doit rester identique");
    }

    #[test]
    fn the_number_replaces_the_icon_and_stays_readable() {
        for percent in [42u8, 99] {
            let px = buffer(16, battery(percent, false), true);
            let painted = opaque(&px);
            assert!(painted > 40, "chiffre trop maigre pour {percent} % : {painted} pixels");
            let rows: Vec<usize> = (0..16)
                .filter(|&y| (0..16).any(|x| (px[y * 16 + x] >> 24) & 0xFF > 40))
                .collect();
            let height = rows.last().unwrap() - rows.first().unwrap() + 1;
            assert!(height >= 9, "chiffre haut de {height} px seulement");
        }
    }

    #[test]
    fn no_number_is_invented_without_a_measurement() {
        for state in [IconState::Docked, IconState::Disconnected] {
            assert_eq!(
                buffer(16, state, false),
                buffer(16, state, true),
                "un chiffre a été inscrit sur un état sans mesure : {state:?}"
            );
        }
    }

    #[test]
    fn every_pixel_is_either_transparent_or_coloured() {
        for state in [battery(70, true), IconState::Docked, IconState::Disconnected] {
            for p in buffer(24, state, false) {
                let (a, rgb) = (p >> 24, p & 0x00FF_FFFF);
                assert!(a == 0 || rgb != 0, "pixel opaque sans couleur");
            }
        }
    }
}
