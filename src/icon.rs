//! Rendu de l'icône : une batterie verticale, dessinée pixel par pixel.
//!
//! Tout est tracé à la main plutôt que par GDI. À seize pixels de côté, GDI
//! place ses traits à la demi-unité près et rend un contour flou ; ici chaque
//! forme est décrite analytiquement puis échantillonnée seize fois par pixel,
//! ce qui donne un bord net à toute densité d'écran.

use std::ffi::c_void;

use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

use crate::hid::BatteryStatus;
use crate::state::IconState;

/// Opacité du dessin lorsque le niveau est inconnu.
const DIM_UNKNOWN: f32 = 0.5;

/// Racine du nombre d'échantillons par pixel. Quatre par quatre suffisent à
/// lisser un contour d'un pixel d'épaisseur.
const SUPERSAMPLE: u32 = 4;

/// Couleur du remplissage selon le niveau : vert franc en haut, ambre au
/// milieu, rouge en bas. Les paliers coïncident avec les seuils de notification.
pub fn level_color(percent: u8) -> (u8, u8, u8) {
    const STOPS: &[(u8, (u8, u8, u8))] = &[
        (0, (0xE5, 0x48, 0x3C)),   // rouge
        (10, (0xE8, 0x5C, 0x35)),  // rouge orangé
        (20, (0xEF, 0x9A, 0x28)),  // ambre
        (45, (0xD8, 0xC0, 0x28)),  // or
        (70, (0x54, 0xB8, 0x43)),  // vert
        (100, (0x3D, 0xA6, 0x38)), // vert soutenu
    ];
    let p = percent.min(100);
    for w in STOPS.windows(2) {
        let ((p0, c0), (p1, c1)) = (w[0], w[1]);
        if p <= p1 {
            let span = (p1 - p0).max(1) as i32;
            let into = (p - p0) as i32;
            let lerp = |a: u8, b: u8| (a as i32 + (b as i32 - a as i32) * into / span) as u8;
            return (lerp(c0.0, c1.0), lerp(c0.1, c1.1), lerp(c0.2, c1.2));
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// Le contour doit contraster avec la barre des tâches, dont le ton suit le
/// thème du système. Une icône blanche sur une barre claire serait invisible.
pub fn outline_color() -> (u8, u8, u8) {
    if system_uses_light_theme() {
        (0x3A, 0x3A, 0x3A)
    } else {
        // Un blanc franc écraserait le remplissage : le contour doit cadrer la
        // couleur, pas lui disputer l'attention.
        (0xDC, 0xDC, 0xDC)
    }
}

fn system_uses_light_theme() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegQueryValueExW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    let wide = |s: &str| -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
    };
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

/// Géométrie de la batterie, en unités de la taille de l'icône.
///
/// Les proportions sont pensées pour que le dessin remplisse le carré : à
/// seize pixels, le corps occupe neuf pixels de large sur treize de haut, ce
/// qui laisse dix pixels utiles de remplissage — assez pour que chaque dizaine
/// de pourcents se distingue.
struct Geometry {
    body: (f32, f32, f32, f32), // x0, y0, x1, y1
    nub: (f32, f32, f32, f32),
    inner: (f32, f32, f32, f32),
    radius: f32,
    stroke: f32,
}

impl Geometry {
    fn new(size: f32) -> Self {
        let stroke = (size / 16.0).max(1.0);
        // Marge franche tout autour : un dessin qui affleure le bord du carré
        // paraît rogné une fois posé dans la barre des tâches.
        let margin = (size / 16.0).clamp(1.0, 2.0);
        let nub_h = (size * 0.075).max(1.0);
        let nub_y = margin;
        // Un corps plus étroit que haut : c'est le rapport qui fait lire
        // « batterie » plutôt que « gélule ».
        let body = (size * 0.25, nub_y + nub_h * 0.7, size * 0.75, size - margin);
        let nub = (size * 0.385, nub_y, size * 0.615, nub_y + nub_h + stroke);
        // Espace minime entre le contour et le remplissage : au-delà, le
        // contour prend un poids visuel qu'il ne doit pas avoir.
        let gap = stroke * 0.3;
        let inner = (
            body.0 + stroke + gap,
            body.1 + stroke + gap,
            body.2 - stroke - gap,
            body.3 - stroke - gap,
        );
        Self { body, nub, inner, radius: size * 0.10, stroke }
    }

    /// Hauteur du remplissage pour un niveau donné. Un niveau non nul garde
    /// toujours au moins un filet visible : une batterie à 1 % ne doit pas
    /// s'afficher vide.
    fn fill_top(&self, percent: u8) -> f32 {
        let h = self.inner.3 - self.inner.1;
        let frac = percent.min(100) as f32 / 100.0;
        let filled = if percent == 0 { 0.0 } else { (h * frac).max(self.stroke) };
        self.inner.3 - filled
    }
}

fn in_rect(x: f32, y: f32, r: (f32, f32, f32, f32)) -> bool {
    x >= r.0 && x < r.2 && y >= r.1 && y < r.3
}

/// Rectangle à coins arrondis : hors des quatre quarts de cercle, on retombe
/// sur un rectangle simple.
fn in_rounded_rect(x: f32, y: f32, r: (f32, f32, f32, f32), radius: f32) -> bool {
    if !in_rect(x, y, r) {
        return false;
    }
    let radius = radius.min((r.2 - r.0) / 2.0).min((r.3 - r.1) / 2.0);
    let cx = x.clamp(r.0 + radius, r.2 - radius);
    let cy = y.clamp(r.1 + radius, r.3 - radius);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= radius * radius
}

/// Test d'appartenance à un polygone, par lancer de rayon.
fn in_polygon(x: f32, y: f32, pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = pts[i];
        let (xj, yj) = pts[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Distance d'un point à un segment.
fn segment_distance(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((px - cx).powi(2) + (py - cy).powi(2)).sqrt()
}

/// Le polygone, épaissi d'une largeur constante.
///
/// Dilater les sommets autour du centre aurait grossi la forme
/// proportionnellement à sa taille, jusqu'à la faire sortir du cadre ; épaissir
/// par la distance au tracé ajoute la même épaisseur partout, quelle que soit
/// l'échelle.
fn near_polygon(x: f32, y: f32, pts: &[(f32, f32)], width: f32) -> bool {
    if in_polygon(x, y, pts) {
        return true;
    }
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        if segment_distance(x, y, pts[j], pts[i]) <= width {
            return true;
        }
        j = i;
    }
    false
}

/// L'éclair de charge, inscrit dans le corps de la batterie.
fn bolt_points(g: &Geometry) -> Vec<(f32, f32)> {
    const SHAPE: &[(f32, f32)] = &[
        (0.66, 0.00),
        (0.06, 0.58),
        (0.42, 0.58),
        (0.34, 1.00),
        (0.94, 0.40),
        (0.58, 0.40),
    ];
    // L'éclair prend appui sur le corps entier, pas sur l'aire de remplissage :
    // à seize pixels, une forme confinée à l'intérieur devient une tache.
    let (x0, y0, x1, y1) = g.body;
    let (w, h) = (x1 - x0, y1 - y0);
    let inset = h * 0.06;
    SHAPE
        .iter()
        .map(|(fx, fy)| (x0 + w * fx, y0 + inset + (h - 2.0 * inset) * fy))
        .collect()
}

/// Proportion d'un pixel couverte par une forme, par suréchantillonnage.
fn coverage(px: u32, py: u32, inside: impl Fn(f32, f32) -> bool) -> f32 {
    let n = SUPERSAMPLE;
    let step = 1.0 / n as f32;
    let mut hits = 0u32;
    for sy in 0..n {
        for sx in 0..n {
            let x = px as f32 + (sx as f32 + 0.5) * step;
            let y = py as f32 + (sy as f32 + 0.5) * step;
            if inside(x, y) {
                hits += 1;
            }
        }
    }
    hits as f32 / (n * n) as f32
}

/// Composition « source par-dessus », en alpha non prémultiplié.
fn blend(dst: &mut u32, color: (u8, u8, u8), alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let a = alpha.min(1.0);
    let (dr, dg, db) = (((*dst >> 16) & 0xFF) as f32, ((*dst >> 8) & 0xFF) as f32, (*dst & 0xFF) as f32);
    let da = ((*dst >> 24) & 0xFF) as f32 / 255.0;

    let out_a = a + da * (1.0 - a);
    if out_a <= 0.0 {
        *dst = 0;
        return;
    }
    let mix = |s: u8, d: f32| ((s as f32 * a + d * da * (1.0 - a)) / out_a).round().clamp(0.0, 255.0) as u32;
    *dst = (((out_a * 255.0).round() as u32) << 24)
        | (mix(color.0, dr) << 16)
        | (mix(color.1, dg) << 8)
        | mix(color.2, db);
}

/// Retire de la matière : sert à creuser le liseré autour de l'éclair.
fn erase(dst: &mut u32, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let keep = (1.0 - amount.min(1.0)).max(0.0);
    let a = ((*dst >> 24) & 0xFF) as f32 * keep;
    *dst = (*dst & 0x00FF_FFFF) | ((a.round() as u32) << 24);
}

/// Dessine l'icône dans un tampon de pixels. Séparé de toute création d'objet
/// GDI, donc vérifiable directement.
pub fn draw(px: &mut [u32], size: u32, state: IconState) {
    draw_themed(px, size, state, outline_color())
}

/// Variante à contour imposé, pour pouvoir contrôler le rendu sur les deux
/// thèmes sans dépendre de celui de la machine.
pub fn draw_themed(px: &mut [u32], size: u32, state: IconState, outline: (u8, u8, u8)) {
    match state {
        IconState::Battery(s) => draw_battery(px, size, &s, outline),
        // Sur le socle et éteinte : elle charge, mais son niveau est hors
        // d'atteinte. La coque et l'éclair disent exactement cela — en charge,
        // niveau inconnu — là où un remplissage inventerait une mesure.
        IconState::Docked => {
            let g = Geometry::new(size as f32);
            draw_shell(px, size, &g, outline);
            draw_bolt(px, size, &g, outline);
            // Atténuation. Sans elle, ce dessin serait rigoureusement celui
            // d'une batterie mesurée à 0 % en charge — état parfaitement réel,
            // qu'une manette à plat posée sur son socle produit. Le ton éteint
            // dit « je ne sais pas », là où le plein dirait « je sais, et c'est
            // vide ».
            for p in px.iter_mut() {
                erase(p, 1.0 - DIM_UNKNOWN);
            }
        }
        // Sans relevé, dessiner une batterie vide serait un contresens : on la
        // lirait comme une batterie à plat. Une prise dit « pas de liaison ».
        IconState::Disconnected => draw_plug(px, size, outline),
    }
}

/// Une prise électrique : deux broches, un corps, un début de cordon.
///
/// Les formes sont pleines et non détourées : à seize pixels, un contour d'un
/// pixel autour d'un objet aussi petit se referme sur lui-même et devient une
/// tache.
fn draw_plug(px: &mut [u32], size: u32, color: (u8, u8, u8)) {
    let s = size as f32;
    let r = s * 0.07;
    let prong_left = (s * 0.30, s * 0.10, s * 0.42, s * 0.40);
    let prong_right = (s * 0.58, s * 0.10, s * 0.70, s * 0.40);
    let body = (s * 0.20, s * 0.33, s * 0.80, s * 0.66);
    let cord = (s * 0.43, s * 0.62, s * 0.57, s * 0.90);

    for y in 0..size {
        for x in 0..size {
            let c = coverage(x, y, |fx, fy| {
                in_rounded_rect(fx, fy, prong_left, r * 0.6)
                    || in_rounded_rect(fx, fy, prong_right, r * 0.6)
                    || in_rounded_rect(fx, fy, body, r)
                    || in_rounded_rect(fx, fy, cord, r * 0.5)
            });
            blend(&mut px[(y * size + x) as usize], color, c);
        }
    }
}

/// Le contour du corps et son téton.
fn draw_shell(px: &mut [u32], size: u32, g: &Geometry, outline: (u8, u8, u8)) {
    for y in 0..size {
        for x in 0..size {
            let c = coverage(x, y, |fx, fy| {
                (in_rounded_rect(fx, fy, g.body, g.radius)
                    && !in_rounded_rect(fx, fy, g.inner, g.radius * 0.5))
                    || in_rounded_rect(fx, fy, g.nub, g.radius * 0.4)
            });
            blend(&mut px[(y * size + x) as usize], outline, c);
        }
    }
}

/// L'éclair, en deux temps : on creuse d'abord un liseré, puis on trace dedans.
/// Le vide ainsi dégagé le détache du remplissage quelle qu'en soit la couleur.
fn draw_bolt(px: &mut [u32], size: u32, g: &Geometry, outline: (u8, u8, u8)) {
    let bolt = bolt_points(g);
    let halo = g.stroke * 0.85;
    for y in 0..size {
        for x in 0..size {
            let i = (y * size + x) as usize;
            erase(&mut px[i], coverage(x, y, |fx, fy| near_polygon(fx, fy, &bolt, halo)));
            let c = coverage(x, y, |fx, fy| in_polygon(fx, fy, &bolt));
            blend(&mut px[i], outline, c);
        }
    }
}

fn draw_battery(px: &mut [u32], size: u32, status: &BatteryStatus, outline: (u8, u8, u8)) {
    let g = Geometry::new(size as f32);
    let fill_color = level_color(status.percent);
    let fill_top = g.fill_top(status.percent);

    draw_shell(px, size, &g, outline);
    for y in 0..size {
        for x in 0..size {
            let fill = coverage(x, y, |fx, fy| {
                fy >= fill_top && in_rounded_rect(fx, fy, g.inner, g.radius * 0.5)
            });
            blend(&mut px[(y * size + x) as usize], fill_color, fill);
        }
    }
    if status.charging {
        draw_bolt(px, size, &g, outline);
    }
}

/// Construit l'icône. Le résultat appartient à l'appelant, qui doit la libérer
/// par `DestroyIcon`.
pub fn render(state: IconState, size: u32) -> HICON {
    let size = size.clamp(16, 64);
    let n = (size * size) as usize;

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
        px.fill(0);
        draw(px, size, state);

        // Le masque monochrome n'est pas consulté pour une icône 32 bits avec
        // couche alpha, mais `CreateIconIndirect` en exige un.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(percent: u8, charging: bool) -> BatteryStatus {
        BatteryStatus { percent, voltage_mv: None, charging, full: false }
    }

    fn buffer(size: u32, state: IconState) -> Vec<u32> {
        let mut px = vec![0u32; (size * size) as usize];
        draw(&mut px, size, state);
        px
    }

    fn battery(percent: u8, charging: bool) -> IconState {
        IconState::Battery(at(percent, charging))
    }

    fn opaque_pixels(px: &[u32]) -> usize {
        px.iter().filter(|p| (*p >> 24) & 0xFF > 40).count()
    }

    #[test]
    fn colour_goes_from_red_through_amber_to_green() {
        let (r0, g0, _) = level_color(0);
        let (r100, g100, _) = level_color(100);
        assert!(r0 > g0, "0 % doit tirer sur le rouge");
        assert!(g100 > r100, "100 % doit tirer sur le vert");
    }

    #[test]
    fn colour_transitions_are_gradual() {
        for p in 0..100u8 {
            let (a, b) = (level_color(p), level_color(p + 1));
            let step = (a.0 as i32 - b.0 as i32).abs()
                + (a.1 as i32 - b.1 as i32).abs()
                + (a.2 as i32 - b.2 as i32).abs();
            assert!(step < 40, "saut de couleur entre {p} et {}", p + 1);
        }
    }

    #[test]
    fn fill_height_grows_with_the_level() {
        let g = Geometry::new(16.0);
        let mut previous = f32::MAX;
        for p in 0..=100u8 {
            let top = g.fill_top(p);
            assert!(top <= previous + 0.001, "le remplissage recule à {p} %");
            assert!(top >= g.inner.1 - 0.001, "le remplissage déborde en haut à {p} %");
            assert!(top <= g.inner.3 + 0.001, "le remplissage déborde en bas à {p} %");
            previous = top;
        }
    }

    #[test]
    fn an_empty_battery_is_empty_and_a_full_one_is_full() {
        let g = Geometry::new(16.0);
        assert_eq!(g.fill_top(0), g.inner.3, "0 % doit ne rien remplir");
        assert!((g.fill_top(100) - g.inner.1).abs() < 0.001, "100 % doit tout remplir");
    }

    #[test]
    fn a_low_but_nonzero_level_stays_visible() {
        let g = Geometry::new(16.0);
        assert!(
            g.inner.3 - g.fill_top(1) >= g.stroke,
            "1 % doit laisser un filet visible"
        );
    }

    #[test]
    fn the_drawing_stays_inside_the_canvas_at_every_size() {
        for size in [16u32, 20, 24, 32, 48, 64] {
            let px = buffer(size, battery(50, false));
            assert_eq!(px.len(), (size * size) as usize);
            // Le pourtour du carré doit rester vide : rien ne doit toucher le bord.
            for i in 0..size {
                for &p in &[
                    px[i as usize],
                    px[((size - 1) * size + i) as usize],
                    px[(i * size) as usize],
                    px[(i * size + size - 1) as usize],
                ] {
                    assert_eq!(p >> 24, 0, "le dessin touche le bord à la taille {size}");
                }
            }
        }
    }

    #[test]
    fn a_fuller_battery_paints_more_pixels() {
        let empty = opaque_pixels(&buffer(32, battery(0, false)));
        let half = opaque_pixels(&buffer(32, battery(50, false)));
        let full = opaque_pixels(&buffer(32, battery(100, false)));
        assert!(half > empty, "50 % doit peindre plus que 0 %");
        assert!(full > half, "100 % doit peindre plus que 50 %");
    }

    #[test]
    fn the_three_states_are_visually_distinct() {
        // Confondre « en charge sur le socle » avec « rien de connecté » ou
        // avec une batterie mesurée annulerait tout l'intérêt de les avoir
        // distingués dans le code.
        let docked = buffer(32, IconState::Docked);
        let absent = buffer(32, IconState::Disconnected);
        let measured = buffer(32, battery(0, true));
        assert_ne!(docked, absent);
        assert_ne!(docked, measured);
        assert_ne!(absent, measured);
    }

    #[test]
    fn the_docked_icon_is_dimmer_than_a_measured_one() {
        // Le cas piégeux : une batterie mesurée à 0 % en charge dessine elle
        // aussi une coque et un éclair, sans remplissage. Seule l'atténuation
        // les sépare.
        let docked = buffer(32, IconState::Docked);
        let flat_charging = buffer(32, battery(0, true));
        assert_ne!(docked, flat_charging);

        let alpha = |px: &[u32]| px.iter().map(|p| (p >> 24) & 0xFF).sum::<u32>();
        let (a, b) = (alpha(&docked), alpha(&flat_charging));
        assert!(a < b, "l'état inconnu doit être plus discret : {a} contre {b}");
        assert!(a > b / 4, "atténué, mais pas au point de disparaître");
    }

    #[test]
    fn a_disconnected_controller_draws_a_plug_not_an_empty_battery() {
        let disconnected = buffer(32, IconState::Disconnected);
        let flat = buffer(32, battery(0, false));
        assert!(opaque_pixels(&disconnected) > 0, "il faut bien dessiner quelque chose");
        // Le point de tout ceci : une batterie vide se lirait « 0 % », ce qui
        // est un contresens quand on n'a aucune mesure.
        assert_ne!(disconnected, flat, "la prise doit se distinguer d'une batterie à plat");

        // La prise est une forme pleine, la batterie vide un simple contour :
        // la première doit peindre nettement plus de matière.
        assert!(
            opaque_pixels(&disconnected) > opaque_pixels(&flat),
            "la prise doit être plus dense qu'un contour vide"
        );
    }

    #[test]
    fn the_plug_is_a_single_connected_shape() {
        // Une prise dont les broches flotteraient loin du corps se lirait comme
        // trois taches sans rapport. On vérifie qu'aucune colonne peinte n'est
        // isolée : le dessin doit tenir d'un seul tenant horizontalement.
        let px = buffer(32, IconState::Disconnected);
        let painted: Vec<usize> = (0..32)
            .filter(|&x| (0..32).any(|y| (px[y * 32 + x] >> 24) & 0xFF > 40))
            .collect();
        assert!(!painted.is_empty());
        for w in painted.windows(2) {
            assert_eq!(w[1] - w[0], 1, "colonne vide au milieu de la prise");
        }
    }

    #[test]
    fn charging_adds_the_bolt() {
        let plain = buffer(32, battery(50, false));
        let charging = buffer(32, battery(50, true));
        assert_ne!(plain, charging, "l'éclair doit modifier le dessin");
    }

    #[test]
    fn every_pixel_is_either_transparent_or_coloured() {
        // Un pixel opaque totalement noir trahirait une composition ratée.
        for p in buffer(32, battery(70, true)) {
            let (a, rgb) = (p >> 24, p & 0x00FF_FFFF);
            assert!(a == 0 || rgb != 0, "pixel opaque sans couleur");
        }
    }

    /// Compose l'icône sur un fond donné, comme le ferait la barre des tâches.
    fn over(background: (u8, u8, u8), px: u32) -> (u8, u8, u8) {
        let a = ((px >> 24) & 0xFF) as f32 / 255.0;
        let src = (((px >> 16) & 0xFF) as f32, ((px >> 8) & 0xFF) as f32, (px & 0xFF) as f32);
        let mix = |s: f32, d: u8| (s * a + d as f32 * (1.0 - a)).round() as u8;
        (mix(src.0, background.0), mix(src.1, background.1), mix(src.2, background.2))
    }

    /// Écrit une planche de contrôle visuelle. Le rendu d'une icône de seize
    /// pixels ne se juge pas sur des assertions : il faut le regarder, sur
    /// fond clair comme sur fond sombre.
    ///
    /// `cargo test -- --ignored render_preview_sheet`
    #[test]
    #[ignore = "génère un fichier à inspecter à l'œil"]
    fn render_preview_sheet() {
        let cases: &[(&str, IconState)] = &[
            ("100", battery(100, false)),
            ("70", battery(70, false)),
            ("45", battery(45, false)),
            ("15", battery(15, false)),
            ("5", battery(5, false)),
            ("charge", battery(60, true)),
            ("socle", IconState::Docked),
            ("absente", IconState::Disconnected),
        ];
        // Fond de barre des tâches et contour associé, pour contrôler les deux
        // thèmes sans dépendre de celui de la machine.
        let backgrounds: &[((u8, u8, u8), (u8, u8, u8))] = &[
            ((0x20, 0x20, 0x20), (0xDC, 0xDC, 0xDC)),
            ((0xF3, 0xF3, 0xF3), (0x3A, 0x3A, 0x3A)),
        ];

        const ICON: u32 = 16;
        const ZOOM: u32 = 10;
        const PAD: u32 = 6;
        let cell = ICON * ZOOM + PAD * 2;
        let strip = ICON + PAD * 2;
        let w = cell * cases.len() as u32;
        let h = (cell + strip) * backgrounds.len() as u32;

        let mut img = vec![(0u8, 0u8, 0u8); (w * h) as usize];
        for (bi, &(bg, outline)) in backgrounds.iter().enumerate() {
            let band_top = bi as u32 * (cell + strip);
            for y in band_top..(band_top + cell + strip).min(h) {
                for x in 0..w {
                    img[(y * w + x) as usize] = bg;
                }
            }
            for (ci, (_, state)) in cases.iter().enumerate() {
                let mut px = vec![0u32; (ICON * ICON) as usize];
                draw_themed(&mut px, ICON, *state, outline);
                let ox = ci as u32 * cell + PAD;

                // Version agrandie, pour juger le tracé.
                for y in 0..ICON * ZOOM {
                    for x in 0..ICON * ZOOM {
                        let p = px[((y / ZOOM) * ICON + x / ZOOM) as usize];
                        img[((band_top + PAD + y) * w + ox + x) as usize] = over(bg, p);
                    }
                }
                // Version à taille réelle, pour juger la lisibilité.
                for y in 0..ICON {
                    for x in 0..ICON {
                        let p = px[(y * ICON + x) as usize];
                        img[((band_top + cell + PAD + y) * w + ox + x) as usize] = over(bg, p);
                    }
                }
            }
        }

        // BMP 24 bits, lignes de bas en haut, chaque ligne alignée sur 4 octets.
        let row = (w * 3).next_multiple_of(4) as usize;
        let pixel_data = row * h as usize;
        let mut out = Vec::with_capacity(54 + pixel_data);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&((54 + pixel_data) as u32).to_le_bytes());
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
        std::fs::write(&path, &out).expect("écriture de la planche");
        println!("planche écrite : {}", path.display());
    }

    #[test]
    fn coverage_is_bounded_and_meaningful() {
        assert_eq!(coverage(0, 0, |_, _| false), 0.0);
        assert_eq!(coverage(0, 0, |_, _| true), 1.0);
        let half = coverage(0, 0, |x, _| x < 0.5);
        assert!((half - 0.5).abs() < 0.01, "couverture d'un demi-pixel : {half}");
    }

    #[test]
    fn rounded_corners_are_actually_carved() {
        let r = (0.0, 0.0, 10.0, 10.0);
        assert!(!in_rounded_rect(0.1, 0.1, r, 3.0), "le coin doit être vide");
        assert!(in_rounded_rect(5.0, 5.0, r, 3.0), "le centre doit être plein");
        assert!(in_rounded_rect(0.1, 5.0, r, 3.0), "le milieu du bord doit être plein");
    }

    #[test]
    fn the_bolt_polygon_is_closed_and_inside_the_body() {
        let g = Geometry::new(32.0);
        let pts = bolt_points(&g);
        assert!(pts.len() >= 6);
        for (x, y) in &pts {
            assert!(*x >= g.body.0 - 0.01 && *x <= g.body.2 + 0.01, "éclair hors du corps");
            assert!(*y >= g.body.1 - 0.01 && *y <= g.body.3 + 0.01, "éclair hors du corps");
        }
        // Le liseré, d'épaisseur constante, ne doit pas non plus sortir du carré.
        for px in 0..32u32 {
            for py in 0..32u32 {
                if near_polygon(px as f32, py as f32, &pts, g.stroke * 0.85) {
                    assert!((1..31).contains(&px) && (1..31).contains(&py), "liseré hors cadre");
                }
            }
        }
        // Le centre géométrique doit tomber dans le polygone.
        assert!(in_polygon(
            pts.iter().map(|p| p.0).sum::<f32>() / pts.len() as f32,
            pts.iter().map(|p| p.1).sum::<f32>() / pts.len() as f32,
            &pts
        ));
    }
}
