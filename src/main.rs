//! Indicateur de batterie de la Steam Controller 2026 dans la zone de
//! notification de Windows.
//!
//! # Comment l'état remonte
//!
//! La manette émet spontanément un rapport d'alimentation toutes les trois
//! secondes et demie environ. Plutôt que d'aller le chercher périodiquement,
//! un fil de lecture reste à l'écoute et transmet chaque rapport à la fenêtre.
//! Poser la manette sur son socle se voit donc en quelques secondes, sans
//! qu'aucun minuteur ne tourne.
//!
//! # Pourquoi l'application ne consomme rien quand la manette est absente
//!
//! Le fil de lecture se termine de lui-même dès qu'il ne reste plus le moindre
//! périphérique Valve énuméré. Il ne subsiste alors qu'une fenêtre cachée
//! bloquée dans `GetMessage`, donc ordonnancée zéro fois par seconde. C'est
//! `WM_DEVICECHANGE`, que Windows diffuse à toute fenêtre de premier niveau
//! quand l'arborescence des périphériques bouge, qui relance tout — aucun
//! abonnement à maintenir, aucun réveil entre deux branchements.

#![windows_subsystem = "windows"]

mod autostart;
mod hid;
mod icon;
mod settings;
mod state;
mod tray;

use std::cell::RefCell;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostMessageW,
    PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage, MSG, WM_APP, WM_DESTROY,
    WM_DEVICECHANGE, WM_DPICHANGED, WM_LBUTTONUP, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER,
    WNDCLASSW, WS_OVERLAPPED,
};

use hid::{BatteryStatus, ProbeError};
use state::{App, IconState};
use tray::{Tray, ID_CHIME, ID_QUIT, ID_TOGGLE_AUTOSTART, ID_TOGGLE_PERCENT, WM_TRAY};

/// Relance de la lecture après un changement de périphérique, le temps que
/// Windows termine son énumération.
const TIMER_SETTLE: usize = 1;
const SETTLE_DELAY_MS: u32 = 1_500;

/// Le fil de lecture a déposé un état.
const WM_STATUS: u32 = WM_APP + 2;

/// L'arborescence des périphériques a changé. Non exporté par `windows-sys`.
const DBT_DEVNODES_CHANGED: WPARAM = 0x0007;

/// Boîte aux lettres entre le fil de lecture et la fenêtre. Seul le dernier
/// état compte : si la fenêtre prend du retard, les relevés intermédiaires
/// n'ont aucun intérêt.
static STATUS: Mutex<Option<Result<BatteryStatus, ProbeError>>> = Mutex::new(None);

/// Vrai tant qu'un fil de lecture est en vie.
static READER_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Demande d'arrêt, à la fermeture.
static READER_STOP: AtomicBool = AtomicBool::new(false);

struct Ctx {
    app: App,
    tray: Tray,
    /// Dernier dessin produit : l'état, et le fait d'y avoir inscrit le
    /// pourcentage. Tant que les deux ne changent pas, il est inutile de
    /// reconstruire une icône.
    drawn: Option<(IconState, bool)>,
}

thread_local! {
    static CTX: RefCell<Option<Ctx>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Taille d'icône adaptée à la densité de l'écran : 16 px à 96 ppp.
fn icon_size(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi == 0 { 96 } else { dpi };
    (16 * dpi / 96).clamp(16, 64)
}

/// Reconstruit l'icône, que son apparence ait changé ou non.
fn repaint(hwnd: HWND, ctx: &mut Ctx) {
    let state = ctx.app.icon_state();
    let with_percent = settings::show_percent();
    let hicon = icon::render(state, icon_size(hwnd), with_percent);
    ctx.tray.set(hicon, &ctx.app.tooltip());
    ctx.drawn = Some((state, with_percent));
}

/// Démarre le fil de lecture s'il n'y en a pas déjà un.
fn ensure_reader(hwnd: HWND) {
    if READER_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    // `HWND` est un pointeur brut, donc non transférable entre fils tel quel.
    let target = hwnd as isize;
    std::thread::spawn(move || {
        hid::run_reader(&READER_STOP, |status| {
            if let Ok(mut slot) = STATUS.lock() {
                *slot = Some(status);
            }
            unsafe { PostMessageW(target as HWND, WM_STATUS, 0, 0) };
        });
        READER_ACTIVE.store(false, Ordering::SeqCst);
    });
}

/// Intègre l'état déposé par le fil de lecture.
fn on_status(hwnd: HWND) {
    CTX.with(|c| {
        // Emprunt prudent. Windows peut nous rappeler depuis une boucle de
        // messages imbriquée alors que l'état est déjà emprunté ailleurs ;
        // paniquer y coûterait le processus entier. Le relevé reste alors dans
        // sa boîte aux lettres et sera traité au message suivant.
        let Ok(mut borrow) = c.try_borrow_mut() else { return };
        let Some(ctx) = borrow.as_mut() else { return };

        let Some(status) = STATUS.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };

        let alert = ctx.app.ingest(status);

        // L'infobulle suit chaque relevé ; l'icône, seulement les changements
        // visibles. Sans cette distinction on reconstruirait une icône toutes
        // les trois secondes et demie pour rien.
        if ctx.drawn != Some((ctx.app.icon_state(), settings::show_percent())) {
            repaint(hwnd, ctx);
        } else {
            let tip = ctx.app.tooltip();
            ctx.tray.set_tooltip(&tip);
        }

        if let Some(a) = alert {
            ctx.tray.notify(&a.title, &a.body);
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_STATUS => {
            on_status(hwnd);
            0
        }

        WM_TIMER => {
            if wp == TIMER_SETTLE {
                KillTimer(hwnd, TIMER_SETTLE);
                ensure_reader(hwnd);
            }
            0
        }

        // Un branchement ou un débranchement quelque part dans la machine.
        // On ne relance pas immédiatement : l'énumération n'est pas finie.
        WM_DEVICECHANGE => {
            if wp == DBT_DEVNODES_CHANGED || wp == 0 {
                SetTimer(hwnd, TIMER_SETTLE, SETTLE_DELAY_MS, None);
            }
            0
        }

        WM_TRAY => {
            match lp as u32 {
                // Un clic gauche réveille la lecture si elle s'était arrêtée
                // faute de matériel.
                WM_LBUTTONUP => ensure_reader(hwnd),
                WM_RBUTTONUP => {
                    let on = autostart::is_enabled();
                    // On extrait ce dont le menu a besoin, puis on relâche
                    // l'emprunt AVANT d'ouvrir le menu : `TrackPopupMenu` fait
                    // tourner sa propre boucle de messages, et un relevé arrivé
                    // entre-temps voudrait modifier ce que nous tiendrions.
                    let awake = CTX.with(|c| {
                        c.borrow()
                            .as_ref()
                            .is_some_and(|ctx| matches!(ctx.app.icon_state(), IconState::Battery(_)))
                    });
                    let percent_on = settings::show_percent();
                    let choice = tray::popup_menu(hwnd, on, awake, percent_on);
                    match choice {
                        ID_TOGGLE_AUTOSTART => {
                            autostart::set_enabled(!on);
                        }
                        ID_TOGGLE_PERCENT => {
                            settings::set_show_percent(!percent_on);
                            // Redessin immédiat : l'utilisateur vient de le
                            // demander, il ne doit pas attendre le prochain relevé.
                            CTX.with(|c| {
                                if let Ok(mut b) = c.try_borrow_mut() {
                                    if let Some(ctx) = b.as_mut() {
                                        repaint(hwnd, ctx);
                                    }
                                }
                            });
                        }
                        ID_CHIME => {
                            // Le carillon dure deux secondes : il n'a rien à
                            // faire sur le fil qui traite les messages.
                            std::thread::spawn(|| {
                                let _ = hid::play_locator_chime();
                            });
                        }
                        ID_QUIT => {
                            READER_STOP.store(true, Ordering::SeqCst);
                            PostQuitMessage(0);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            0
        }

        // Changement de densité, ou bascule entre thème clair et sombre : le
        // contour de l'icône doit reprendre la couleur qui contraste.
        WM_DPICHANGED | WM_SETTINGCHANGE => {
            CTX.with(|c| {
                // Même prudence qu'ailleurs : ces messages peuvent survenir
                // depuis une boucle imbriquée. Un redessin manqué se rattrape
                // au relevé suivant ; une panique, non.
                if let Ok(mut borrow) = c.try_borrow_mut() {
                    if let Some(ctx) = borrow.as_mut() {
                        repaint(hwnd, ctx);
                    }
                }
            });
            DefWindowProcW(hwnd, msg, wp, lp)
        }

        WM_DESTROY => {
            READER_STOP.store(true, Ordering::SeqCst);
            PostQuitMessage(0);
            0
        }

        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Rend `false` si une autre instance tourne déjà. Sans cela, un lancement en
/// double poserait deux icônes dans la zone de notification.
fn claim_single_instance() -> bool {
    const ERROR_ALREADY_EXISTS: u32 = 183;
    let name = wide("Local\\SteamControllerBattery.instance");
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        // Le descripteur est volontairement fuité : il doit vivre aussi
        // longtemps que le processus.
        !h.is_null() && windows_sys::Win32::Foundation::GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn main() {
    if !claim_single_instance() {
        return;
    }

    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("SteamControllerBatteryWindow");

        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = instance;
        wc.lpszClassName = class_name.as_ptr();
        RegisterClassW(&wc);

        // Fenêtre de premier niveau, jamais affichée. Une fenêtre
        // « message-only » serait plus légère mais ne recevrait pas les
        // diffusions de `WM_DEVICECHANGE`, dont dépend tout le mécanisme de
        // réveil.
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("Batterie manette Steam").as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }

        CTX.with(|c| {
            *c.borrow_mut() = Some(Ctx { app: App::new(), tray: Tray::new(hwnd), drawn: None });
        });

        // L'icône apparaît tout de suite, coque vide, le temps du premier relevé.
        CTX.with(|c| {
            if let Some(ctx) = c.borrow_mut().as_mut() {
                repaint(hwnd, ctx);
            }
        });
        ensure_reader(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        // Bloquant : tant que rien n'arrive, le processus n'est pas ordonnancé.
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        READER_STOP.store(true, Ordering::SeqCst);
        // Retire l'icône de la zone de notification avant de rendre la main.
        CTX.with(|c| c.borrow_mut().take());
    }
}
