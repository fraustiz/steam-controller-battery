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

/// Nom réservé aux vérifications.
///
/// Elles ne doivent jamais toucher à [`VALUE_NAME`]. Une première version le
/// faisait, en restaurant l'état de départ à la fin — mais restaurer passait
/// par `set_enabled(true)`, qui écrit `current_exe()`. Sous `cargo test`, c'est
/// l'exécutable de tests. Chaque exécution de la série remplaçait donc
/// l'entrée de l'utilisateur par le chemin d'un binaire de test, et Windows
/// lançait la suite de tests à l'ouverture de session au lieu de
/// l'application.
#[cfg(test)]
const TEST_VALUE_NAME: &str = "SteamControllerBatteryTest";

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn open(access: u32) -> Option<HKEY> {
    let mut key: HKEY = std::ptr::null_mut();
    let path = wide(RUN_KEY);
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, access, &mut key) };
    (rc == ERROR_SUCCESS).then_some(key)
}

/// La ligne de commande à réexécuter à l'ouverture de session.
///
/// Le chemin est mis entre guillemets : sans eux, un chemin contenant une
/// espace serait coupé au lancement.
///
/// Les paramètres de langue sont reconduits, faute de quoi activer le
/// démarrage automatique depuis une instance lancée en `--lang fr` la ferait
/// repartir dans la langue du système. `--debug` ne l'est pas : personne ne
/// veut retrouver un mode de simulation à chaque ouverture de session.
fn command_line() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let mut line = format!("\"{}\"", exe.display());
    for arg in forwarded_args() {
        line.push(' ');
        line.push_str(&arg);
    }
    Some(line)
}

fn forwarded_args() -> Vec<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut kept = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg.starts_with("--lang=") {
            kept.push(arg.clone());
        } else if arg == "--lang" {
            if let Some(code) = args.get(i + 1) {
                kept.push(format!("--lang={code}"));
                i += 1;
            }
        }
        i += 1;
    }
    kept
}

pub fn is_enabled() -> bool {
    has_value(VALUE_NAME)
}

fn has_value(value_name: &str) -> bool {
    let Some(key) = open(KEY_READ) else { return false };
    let name = wide(value_name);
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
    set_value(VALUE_NAME, enable)
}

fn set_value(value_name: &str, enable: bool) -> bool {
    let Some(key) = open(KEY_WRITE) else { return false };
    let name = wide(value_name);

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
        assert!(cmd.starts_with('"'), "{cmd}");
        assert!(cmd.contains(":\\"), "chemin non absolu : {cmd}");
        // Le chemin lui-même doit être clos, même quand des paramètres suivent.
        assert!(cmd[1..].contains('"'), "chemin non fermé : {cmd}");
    }

    #[test]
    fn only_the_language_survives_into_the_startup_entry() {
        // `forwarded_args` lit la vraie ligne de commande, qui sous
        // `cargo test` ne contient aucun de nos paramètres : on vérifie donc
        // la règle, pas un cas particulier.
        for arg in forwarded_args() {
            assert!(arg.starts_with("--lang="), "paramètre reconduit à tort : {arg}");
        }
    }

    #[test]
    fn wide_strings_are_nul_terminated() {
        assert_eq!(wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
        assert_eq!(wide(""), vec![0]);
    }

    /// Le va-et-vient touche le vrai registre, mais sous un nom réservé : le
    /// démarrage automatique de l'utilisateur n'est jamais lu ni écrit.
    #[test]
    fn toggling_round_trips() {
        let untouched = is_enabled();

        assert!(set_value(TEST_VALUE_NAME, true));
        assert!(has_value(TEST_VALUE_NAME), "activation non prise en compte");
        assert!(set_value(TEST_VALUE_NAME, false));
        assert!(!has_value(TEST_VALUE_NAME), "désactivation non prise en compte");

        assert_eq!(
            is_enabled(),
            untouched,
            "la vérification a touché au démarrage automatique de l'utilisateur"
        );
    }

    #[test]
    fn the_real_entry_is_never_written_by_the_tests() {
        // Un nom partagé suffirait à reproduire le défaut d'origine.
        assert_ne!(VALUE_NAME, TEST_VALUE_NAME);
    }
}
