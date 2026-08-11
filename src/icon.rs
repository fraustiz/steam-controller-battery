//! Rendu de l'icône de la zone de notification, en GDI.
//!
//! Une icône Windows se compose d'une image couleur et d'un masque. On dessine
//! dans une section DIB 32 bits, puis on impose la couche alpha nous-mêmes :
//! GDI écrit le texte sans toucher à l'alpha, si bien que des pixels dessinés
//! resteraient parfaitement transparents sans cette reprise.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC,
    DeleteObject, DrawTextW, Polygon, SelectObject, SetBkMode, SetTextColor, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE,
    DT_VCENTER, FF_DONTCARE, FW_BOLD, HGDIOBJ, TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

use crate::hid::BatteryStatus;

/// Couleur du fond selon le niveau : vert franc en haut, ambre au milieu,
/// rouge en bas. Les paliers sont choisis pour que la bascule vers l'ambre
/// coïncide avec le premier seuil de notification.
pub fn level_color(percent: u8) -> (u8, u8, u8) {
    const STOPS: &[(u8, (u8, u8, u8))] = &[
        (0, (0xD1, 0x3A, 0x2F)),   // rouge
        (10, (0xD9, 0x4B, 0x2B)),  // rouge orangé
        (20, (0xE0, 0x8A, 0x1E)),  // ambre
        (45, (0xC9, 0xB0, 0x1E)),  // or
        (70, (0x4C, 0xA6, 0x3A)),  // vert
        (100, (0x35, 0x96, 0x30)), // vert soutenu
    ];
    let p = percent.min(100);
    for w in STOPS.windows(2) {
        let ((p0, c0), (p1, c1)) = (w[0], w[1]);
        if p <= p1 {
            let span = (p1 - p0) as u32;
            let into = (p - p0) as u32;
            let lerp = |a: u8, b: u8| {
                (a as i32 + (b as i32 - a as i32) * into as i32 / span.max(1) as i32) as u8
            };
            return (lerp(c0.0, c1.0), lerp(c0.1, c1.1), lerp(c0.2, c1.2));
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// Le texte à inscrire dans l'icône. Trois caractères au maximum.
pub fn label(status: Option<&BatteryStatus>) -> String {
    match status {
        Some(s) => s.percent.min(100).to_string(),
        None => "?".into(),
    }
}

/// Masque en carré à coins arrondis. Rend `true` pour les pixels de l'icône.
fn rounded_mask(size: u32) -> Vec<bool> {
    let s = size as i32;
    let r = (s / 4).max(1);
    let mut m = vec![false; (size * size) as usize];
    for y in 0..s {
        for x in 0..s {
            // Distance au centre du coin le plus proche, uniquement dans les coins.
            let dx = if x < r { r - x } else if x >= s - r { x - (s - r - 1) } else { 0 };
            let dy = if y < r { r - y } else if y >= s - r { y - (s - r - 1) } else { 0 };
            let inside = if dx > 0 && dy > 0 {
                dx * dx + dy * dy <= r * r
            } else {
                true
            };
            m[(y * s + x) as usize] = inside;
        }
    }
    m
}

/// Construit l'icône. Le résultat est la propriété de l'appelant, qui doit la
/// libérer par `DestroyIcon`.
pub fn render(status: Option<&BatteryStatus>, size: u32) -> HICON {
    let size = size.clamp(16, 64);
    let n = (size * size) as usize;
    let mask = rounded_mask(size);

    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = size as i32;
        bmi.bmiHeader.biHeight = -(size as i32); // orientation de haut en bas
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;

        let mut bits: *mut c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        let old = SelectObject(dc, dib as HGDIOBJ);

        let px = std::slice::from_raw_parts_mut(bits as *mut u32, n);

        // Fond.
        let (r, g, b) = match status {
            Some(s) => level_color(s.percent),
            None => (0x6B, 0x6B, 0x6B), // gris : état inconnu
        };
        let bg = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        for (i, p) in px.iter_mut().enumerate() {
            *p = if mask[i] { bg } else { 0 };
        }

        // Éclair de charge, dessiné avant le texte pour rester en arrière-plan.
        if status.is_some_and(|s| s.charging) {
            draw_bolt(dc, size);
        }

        // Texte.
        let text = label(status);
        let wide: Vec<u16> = text.encode_utf16().collect();
        let height = if wide.len() >= 3 { size * 58 / 100 } else { size * 76 / 100 };
        let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let font = CreateFontW(
            height as i32,
            0,
            0,
            0,
            FW_BOLD as i32,
            0,
            0,
            0,
            0,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            FF_DONTCARE as u32,
            face.as_ptr(),
        );
        let old_font = SelectObject(dc, font as HGDIOBJ);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00FF_FFFF); // blanc, en BGR
        let mut rect = RECT { left: 0, top: 0, right: size as i32, bottom: size as i32 };
        DrawTextW(
            dc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(dc, old_font);
        DeleteObject(font as HGDIOBJ);

        // GDI a écrit le texte avec une alpha nulle : on impose l'opacité sur
        // toute la forme, et la transparence stricte au dehors.
        for (i, p) in px.iter_mut().enumerate() {
            *p = if mask[i] { *p | 0xFF00_0000 } else { 0 };
        }

        // Le masque monochrome n'est pas consulté pour une icône 32 bits avec
        // alpha, mais `CreateIconIndirect` en exige un.
        let empty = vec![0u8; (size as usize).div_ceil(8) * size as usize];
        let mono = CreateBitmap(size as i32, size as i32, 1, 1, empty.as_ptr() as *const c_void);

        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mono,
            hbmColor: dib,
        };
        let icon = CreateIconIndirect(&info);

        SelectObject(dc, old);
        DeleteObject(mono as HGDIOBJ);
        DeleteObject(dib as HGDIOBJ);
        DeleteDC(dc);
        icon
    }
}

/// Un éclair stylisé dans le coin supérieur droit.
unsafe fn draw_bolt(dc: windows_sys::Win32::Graphics::Gdi::HDC, size: u32) {
    use windows_sys::Win32::Foundation::POINT;

    let s = size as i32;
    // Boîte de l'éclair : quart supérieur droit, avec une marge.
    let (ox, oy, w, h) = (s * 58 / 100, s * 6 / 100, s * 34 / 100, s * 40 / 100);
    let pt = |fx: i32, fy: i32| POINT {
        x: ox + w * fx / 100,
        y: oy + h * fy / 100,
    };
    let pts = [
        pt(55, 0),
        pt(0, 55),
        pt(42, 55),
        pt(30, 100),
        pt(100, 40),
        pt(58, 40),
    ];

    let brush = CreateSolidBrush(0x0033_F0FF); // jaune vif, en BGR
    let old = SelectObject(dc, brush as HGDIOBJ);
    Polygon(dc, pts.as_ptr(), pts.len() as i32);
    SelectObject(dc, old);
    DeleteObject(brush as HGDIOBJ);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_goes_from_red_through_amber_to_green() {
        let (r0, g0, _) = level_color(0);
        let (r100, g100, _) = level_color(100);
        assert!(r0 > g0, "0 % doit tirer sur le rouge");
        assert!(g100 > r100, "100 % doit tirer sur le vert");
    }

    #[test]
    fn colour_is_defined_over_the_whole_range() {
        for p in 0..=120u8 {
            let (r, g, b) = level_color(p);
            assert!(r as u32 + g as u32 + b as u32 > 0, "couleur nulle à {p}");
        }
    }

    #[test]
    fn colour_transitions_are_gradual() {
        // Aucun saut brutal : l'icône ne doit pas clignoter d'une couleur à
        // l'autre pour un point de pourcentage.
        for p in 0..100u8 {
            let a = level_color(p);
            let b = level_color(p + 1);
            let step = (a.0 as i32 - b.0 as i32).abs()
                + (a.1 as i32 - b.1 as i32).abs()
                + (a.2 as i32 - b.2 as i32).abs();
            assert!(step < 40, "saut de couleur entre {p} et {}", p + 1);
        }
    }

    #[test]
    fn label_never_exceeds_three_characters() {
        for p in 0..=255u8 {
            let s = BatteryStatus { percent: p, voltage_mv: None, charging: false };
            assert!(label(Some(&s)).len() <= 3, "étiquette trop longue pour {p}");
        }
        assert_eq!(label(None), "?");
    }

    #[test]
    fn mask_is_square_and_carves_the_corners() {
        for size in [16u32, 20, 24, 32] {
            let m = rounded_mask(size);
            assert_eq!(m.len(), (size * size) as usize);
            assert!(!m[0], "le coin supérieur gauche doit être vide");
            let centre = (size / 2 * size + size / 2) as usize;
            assert!(m[centre], "le centre doit être plein");
            let edge = (size / 2 * size) as usize;
            assert!(m[edge], "le milieu du bord gauche doit être plein");
        }
    }
}
