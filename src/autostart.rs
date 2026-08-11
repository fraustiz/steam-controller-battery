//! Démarrage automatique, par la clé `Run` de l'utilisateur courant.
//!
//! Ni service, ni tâche planifiée, ni élévation de privilèges : une valeur
//! dans `HKEY_CURRENT_USER`, que l'utilisateur peut retirer à la main.

use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "SteamControllerBattery";

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn open(access: u32) -> Option<HKEY> {
    let mut key: HKEY = std::ptr::null_mut();
    let path = wide(RUN_KEY);
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, access, &mut key) };
    (rc == ERROR_SUCCESS).then_some(key)
}

/// Chemin complet de l'exécutable courant, entre guillemets : sans eux, un
/// chemin contenant une espace serait coupé au lancement.
fn command_line() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

pub fn is_enabled() -> bool {
    let Some(key) = open(KEY_READ) else { return false };
    let name = wide(VALUE_NAME);
    let mut size: u32 = 0;
    let rc = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    unsafe { RegCloseKey(key) };
    rc == ERROR_SUCCESS
}

/// Écrit ou retire la valeur. Rend `false` si le registre a refusé.
pub fn set_enabled(enable: bool) -> bool {
    let Some(key) = open(KEY_WRITE) else { return false };
    let name = wide(VALUE_NAME);

    let rc = if enable {
        let Some(cmd) = command_line() else {
            unsafe { RegCloseKey(key) };
            return false;
        };
        let value = wide(&cmd);
        unsafe {
            RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as *const u8,
                (value.len() * 2) as u32, // en octets, terminateur compris
            )
        }
    } else {
        unsafe { RegDeleteValueW(key, name.as_ptr()) }
    };

    unsafe { RegCloseKey(key) };
    rc == ERROR_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_is_quoted_and_absolute() {
        let cmd = command_line().expect("chemin de l'exécutable");
        assert!(cmd.starts_with('"') && cmd.ends_with('"'), "{cmd}");
        assert!(cmd.contains(":\\"), "chemin non absolu : {cmd}");
    }

    #[test]
    fn wide_strings_are_nul_terminated() {
        assert_eq!(wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
        assert_eq!(wide(""), vec![0]);
    }

    /// Le va-et-vient complet touche le vrai registre de l'utilisateur ; on
    /// restaure l'état de départ quoi qu'il arrive.
    #[test]
    fn toggling_round_trips() {
        let original = is_enabled();

        assert!(set_enabled(true));
        assert!(is_enabled(), "activation non prise en compte");
        assert!(set_enabled(false));
        assert!(!is_enabled(), "désactivation non prise en compte");

        if original {
            set_enabled(true);
        }
        assert_eq!(is_enabled(), original, "état de départ non restauré");
    }
}
