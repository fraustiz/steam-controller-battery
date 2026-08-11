//! Indicateur de batterie de la Steam Controller 2026 dans la zone de
//! notification de Windows.
//!
//! # Pourquoi l'application ne consomme rien quand la manette est absente
//!
//! Le processus se réduit à une fenêtre cachée bloquée dans `GetMessage`, donc
//! ordonnancée zéro fois par seconde. Deux choses seulement peuvent le
//! réveiller :
//!
//! - `WM_DEVICECHANGE`, que Windows diffuse à toute fenêtre de premier niveau
//!   quand l'arborescence des périphériques bouge. Aucun abonnement n'est
//!   nécessaire, et rien ne tourne entre deux branchements ;
//! - un `WM_TIMER`, **armé uniquement tant qu'une manette répond**. Dès qu'elle
//!   disparaît, le minuteur est détruit et l'on retombe à zéro réveil.
//!
//! La lecture elle-même ouvre le périphérique HID, pose sa question et le
//! referme, plutôt que de garder un descripteur ouvert sur un flux de rapports
//! à 270 Hz qu'il faudrait drainer en permanence.

#![windows_subsystem = "windows"]

mod autostart;
mod hid;
mod icon;
mod state;
mod tray;

use std::cell::RefCell;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, PostQuitMessage,
    RegisterClassW, SetTimer, TranslateMessage, MSG, WM_DESTROY, WM_DEVICECHANGE, WM_DPICHANGED,
    WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

use state::App;
use tray::{Tray, ID_QUIT, ID_TOGGLE_AUTOSTART, WM_TRAY};

/// Relevé périodique, tant qu'une manette répond.
const TIMER_POLL: usize = 1;
/// Relevé unique après un changement de périphérique, le temps que Windows
/// termine l'énumération.
const TIMER_SETTLE: usize = 2;

const POLL_INTERVAL_MS: u32 = 30_000;
const SETTLE_DELAY_MS: u32 = 1_500;

/// L'arborescence des périphériques a changé. Non exporté par `windows-sys`.
const DBT_DEVNODES_CHANGED: WPARAM = 0x0007;

struct Ctx {
    app: App,
    tray: Tray,
    polling: bool,
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

/// Interroge la manette, met à jour l'icône, et ajuste le minuteur.
fn refresh(hwnd: HWND) {
    let reading = hid::probe();

    CTX.with(|c| {
        let mut borrow = c.borrow_mut();
        let Some(ctx) = borrow.as_mut() else { return };

        let alert = ctx.app.ingest(reading);
        let hicon = icon::render(ctx.app.display(), icon_size(hwnd));
        ctx.tray.set(hicon, &ctx.app.tooltip());
        if let Some(a) = alert {
            ctx.tray.notify(&a.title, &a.body);
        }

        // Le minuteur tourne dès qu'un périphérique Valve est énuméré, même si
        // la manette est éteinte : allumer une manette déjà appairée ne produit
        // aucun `WM_DEVICECHANGE`, puisque le dongle, lui, n'a pas bougé. Ce
        // n'est qu'en l'absence totale de matériel Valve que l'on peut se
        // permettre de ne rien faire du tout et d'attendre un branchement.
        let want_polling = !ctx.app.is_absent();
        if want_polling && !ctx.polling {
            unsafe { SetTimer(hwnd, TIMER_POLL, POLL_INTERVAL_MS, None) };
            ctx.polling = true;
        } else if !want_polling && ctx.polling {
            unsafe { KillTimer(hwnd, TIMER_POLL) };
            ctx.polling = false;
        }
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wp == TIMER_SETTLE {
                KillTimer(hwnd, TIMER_SETTLE);
            }
            refresh(hwnd);
            0
        }

        // Un branchement ou un débranchement quelque part dans la machine.
        // On ne relève pas immédiatement : l'énumération n'est pas finie.
        WM_DEVICECHANGE => {
            if wp == DBT_DEVNODES_CHANGED || wp == 0 {
                SetTimer(hwnd, TIMER_SETTLE, SETTLE_DELAY_MS, None);
            }
            0
        }

        WM_TRAY => {
            match lp as u32 {
                WM_LBUTTONUP => refresh(hwnd),
                WM_RBUTTONUP => {
                    let on = autostart::is_enabled();
                    let choice = CTX.with(|c| {
                        c.borrow()
                            .as_ref()
                            .map(|ctx| ctx.tray.popup_menu(hwnd, on))
                            .unwrap_or(0)
                    });
                    match choice {
                        ID_TOGGLE_AUTOSTART => {
                            autostart::set_enabled(!on);
                        }
                        ID_QUIT => {
                            PostQuitMessage(0);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            0
        }

        // Changement de densité : l'icône doit être redessinée à la bonne taille.
        WM_DPICHANGED => {
            refresh(hwnd);
            0
        }

        WM_DESTROY => {
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
            *c.borrow_mut() = Some(Ctx {
                app: App::new(),
                tray: Tray::new(hwnd),
                polling: false,
            });
        });

        refresh(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        // Bloquant : tant que rien n'arrive, le processus n'est pas ordonnancé.
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Retire l'icône de la zone de notification avant de rendre la main.
        CTX.with(|c| c.borrow_mut().take());
    }
}
