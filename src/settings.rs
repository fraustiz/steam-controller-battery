//! Préférences de l'utilisateur, dans sa propre branche du registre.
//!
//! Volontairement séparé de [`crate::autostart`] : celui-ci écrit dans une clé
//! qui appartient à Windows, celle des programmes lancés à l'ouverture de
//! session. Nos préférences, elles, nous appartiennent et se suppriment sans
//! conséquence.

use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
};

const KEY_PATH: &str = r"Software\SteamControllerBattery";
const VALUE_SHOW_PERCENT: &str = "ShowPercent";

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Ouvre la clé, en la créant au besoin. Rien à signaler si elle n'existe pas
/// encore : c'est le cas au premier lancement.
fn open(access: u32) -> Option<HKEY> {
    let mut key: HKEY = std::ptr::null_mut();
    let path = wide(KEY_PATH);
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            access,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    (rc == ERROR_SUCCESS).then_some(key)
}

fn read_flag(name: &str, default: bool) -> bool {
    let Some(key) = open(KEY_READ) else { return default };
    let name = wide(name);
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let rc = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut value as *mut u32 as *mut u8,
            &mut size,
        )
    };
    unsafe { RegCloseKey(key) };
    if rc == ERROR_SUCCESS {
        value != 0
    } else {
        default
    }
}

fn write_flag(name: &str, on: bool) -> bool {
    let Some(key) = open(KEY_WRITE) else { return false };
    let name = wide(name);
    let value: u32 = on as u32;
    let rc = unsafe {
        RegSetValueExW(
            key,
            name.as_ptr(),
            0,
            REG_DWORD,
            &value as *const u32 as *const u8,
            std::mem::size_of::<u32>() as u32,
        )
    };
    unsafe { RegCloseKey(key) };
    rc == ERROR_SUCCESS
}

/// Le pourcentage est-il inscrit dans l'icône ?
///
/// Éteint par défaut : à seize pixels, deux chiffres occupent la moitié de
/// l'icône, et le niveau se lit déjà au remplissage. C'est un ajout pour qui
/// veut le chiffre exact d'un coup d'œil, pas la présentation naturelle.
pub fn show_percent() -> bool {
    read_flag(VALUE_SHOW_PERCENT, false)
}

pub fn set_show_percent(on: bool) -> bool {
    write_flag(VALUE_SHOW_PERCENT, on)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touche le vrai registre de l'utilisateur ; restaure l'état de départ
    /// quoi qu'il advienne.
    #[test]
    fn the_preference_round_trips() {
        let original = show_percent();

        assert!(set_show_percent(true));
        assert!(show_percent(), "activation non retenue");
        assert!(set_show_percent(false));
        assert!(!show_percent(), "désactivation non retenue");

        set_show_percent(original);
        assert_eq!(show_percent(), original, "état de départ non restauré");
    }

    #[test]
    fn an_unknown_value_falls_back_to_its_default() {
        assert!(!read_flag("ValeurQuiNExistePas", false));
        assert!(read_flag("ValeurQuiNExistePas", true));
    }
}
