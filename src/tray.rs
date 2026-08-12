//! L'icône dans la zone de notification et son menu.

use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_WARNING, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos, SetForegroundWindow,
    TrackPopupMenu, HICON, MF_CHECKED, MF_ENABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING,
    MF_UNCHECKED, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

/// Message que Windows renvoie à notre fenêtre pour les clics sur l'icône.
pub const WM_TRAY: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;

pub const ID_TOGGLE_AUTOSTART: u32 = 1;
pub const ID_QUIT: u32 = 2;
pub const ID_CHIME: u32 = 3;
pub const ID_TOGGLE_PERCENT: u32 = 4;

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Recopie une chaîne dans un tampon de taille fixe, en la tronquant au besoin
/// et en garantissant le terminateur.
fn fill(dst: &mut [u16], src: &str) {
    let s = wide(src);
    let n = s.len().min(dst.len());
    dst[..n].copy_from_slice(&s[..n]);
    dst[dst.len() - 1] = 0;
    if n == dst.len() {
        dst[n - 1] = 0;
    }
}

pub struct Tray {
    data: NOTIFYICONDATAW,
    icon: HICON,
    added: bool,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        Self { data, icon: std::ptr::null_mut(), added: false }
    }

    /// Installe ou met à jour l'icône et son infobulle. L'ancienne icône est
    /// détruite après la bascule, jamais avant : Windows la lit encore.
    pub fn set(&mut self, icon: HICON, tooltip: &str) {
        let previous = self.icon;
        self.icon = icon;
        self.data.hIcon = icon;
        fill(&mut self.data.szTip, tooltip);

        let op = if self.added { NIM_MODIFY } else { NIM_ADD };
        let ok = unsafe { Shell_NotifyIconW(op, &self.data) } != 0;
        if ok {
            self.added = true;
        }

        if !previous.is_null() {
            unsafe { DestroyIcon(previous) };
        }
    }

    /// Met à jour la seule infobulle. Le niveau exact et la tension changent à
    /// chaque relevé alors que le dessin, lui, ne bouge qu'au pourcent près :
    /// inutile de reconstruire une icône pour ça.
    pub fn set_tooltip(&mut self, tooltip: &str) {
        if !self.added {
            return;
        }
        fill(&mut self.data.szTip, tooltip);
        let mut d = self.data;
        d.uFlags = NIF_TIP;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
    }

    /// Ballon d'avertissement natif. Pas de toast WinRT, donc pas
    /// d'identifiant d'application à enregistrer.
    pub fn notify(&mut self, title: &str, body: &str) {
        if !self.added {
            return;
        }
        let mut d = self.data;
        d.uFlags = NIF_INFO;
        d.dwInfoFlags = NIIF_WARNING;
        fill(&mut d.szInfoTitle, title);
        fill(&mut d.szInfo, body);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &d) };
    }
}

/// Menu contextuel. Rend l'identifiant choisi, ou zéro si l'utilisateur a
/// cliqué à côté.
///
/// `can_chime` est faux quand la manette est éteinte : ses actionneurs ne
/// reçoivent alors rien, et proposer une action sans effet vaut moins que la
/// montrer indisponible.
///
/// Délibérément une fonction libre, et non une méthode de `Tray`.
/// `TrackPopupMenu` fait tourner sa propre boucle de messages : tant que le
/// menu est ouvert, la fenêtre continue de recevoir et de traiter des messages.
/// Une méthode obligerait l'appelant à tenir un emprunt de l'état de
/// l'application pendant tout ce temps, et le premier relevé qui arriverait
/// voudrait le modifier — emprunt mutable sur emprunt partagé, panique, et
/// arrêt immédiat du processus puisque le profil release abandonne au lieu de
/// dérouler la pile. Ne rien avoir à emprunter supprime le problème à la
/// racine plutôt que de le rendre improbable.
pub fn popup_menu(hwnd: HWND, autostart_on: bool, can_chime: bool, percent_on: bool) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        AppendMenuW(
            menu,
            MF_STRING | if can_chime { MF_ENABLED } else { MF_GRAYED },
            ID_CHIME as usize,
            wide("Faire sonner la manette").as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());

        AppendMenuW(
            menu,
            MF_STRING | if percent_on { MF_CHECKED } else { MF_UNCHECKED },
            ID_TOGGLE_PERCENT as usize,
            wide("Afficher le pourcentage").as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING | if autostart_on { MF_CHECKED } else { MF_UNCHECKED },
            ID_TOGGLE_AUTOSTART as usize,
            wide("Démarrer avec Windows").as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, ID_QUIT as usize, wide("Quitter").as_ptr());

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        // Sans cet appel, le menu ne se referme pas quand on clique ailleurs.
        SetForegroundWindow(hwnd);
        let choice = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_BOTTOMALIGN,
            pt.x,
            pt.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(menu);
        choice as u32
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            if self.added {
                Shell_NotifyIconW(NIM_DELETE, &self.data);
            }
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_truncates_without_overflowing() {
        let mut buf = [0u16; 8];
        fill(&mut buf, "abcdefghijklmnop");
        assert_eq!(buf[buf.len() - 1], 0, "terminateur absent");
        assert_eq!(buf[0], b'a' as u16);
    }

    #[test]
    fn fill_handles_exact_and_short_strings() {
        let mut buf = [0u16; 8];
        fill(&mut buf, "abc");
        assert_eq!(&buf[..4], &[97, 98, 99, 0]);

        let mut buf = [0u16; 4];
        fill(&mut buf, "");
        assert_eq!(buf[0], 0);
    }

    #[test]
    fn tooltip_buffer_is_the_size_windows_expects() {
        let d: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        assert_eq!(d.szTip.len(), 128);
        assert_eq!(d.szInfo.len(), 256);
        assert_eq!(d.szInfoTitle.len(), 64);
    }
}
