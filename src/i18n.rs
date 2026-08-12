//! Traductions de l'interface.
//!
//! `Strings` liste tous les libellés en champs : oublier une traduction ne
//! compile pas. Les libellés à paramètre sont des pointeurs de fonction, ce qui
//! laisse chaque langue choisir son ordre de mots et ses accords — « 42 % » ne
//! se place pas au même endroit selon la phrase.
//!
//! Même convention que le projet voisin `shs-studio`, pour que les deux se
//! lisent pareil.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
}

impl Lang {
    #[cfg(test)]
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_ascii_lowercase().as_str() {
            "en" => Some(Lang::En),
            "fr" => Some(Lang::Fr),
            _ => None,
        }
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::En => &EN,
            Lang::Fr => &FR,
        }
    }
}

pub struct Strings {
    // menu contextuel
    pub chime: &'static str,
    pub show_percent: &'static str,
    pub autostart: &'static str,
    pub quit: &'static str,

    // infobulle
    pub controller: &'static str,
    pub charging: &'static str,
    pub charged: &'static str,
    pub reading: &'static str,

    // états sans mesure
    pub no_dongle: &'static str,
    pub controller_offline: &'static str,
    pub controller_docked: &'static str,
    pub controller_busy: &'static str,

    // notification de batterie faible
    pub low_title: &'static str,

    // simulation
    pub sim_prefix: &'static str,
    pub sim_charging: &'static str,
    pub sim_connected: &'static str,
    pub sim_docked: &'static str,
    pub sim_disconnected: &'static str,
    pub sim_minus: &'static str,
    pub sim_plus: &'static str,

    // libellés à paramètre
    pub hid_unavailable: fn(&str) -> String,
    pub low_body: fn(u8) -> String,
    pub sim_level: fn(u8) -> String,
    pub percent_of: fn(u8) -> String,
    pub volts: fn(u16) -> String,
}

const EN: Strings = Strings {
    chime: "Ring the controller",
    show_percent: "Show the percentage",
    autostart: "Start with Windows",
    quit: "Quit",

    controller: "Steam Controller",
    charging: "charging",
    charged: "charged",
    reading: "Steam Controller — reading",

    no_dongle: "No Steam dongle detected",
    controller_offline: "Controller off or out of range",
    controller_docked: "Controller off on its dock — level unknown",
    controller_busy: "Controller held by another program",

    low_title: "Controller battery low",

    sim_prefix: "[simulated] ",
    sim_charging: "Simulate charging",
    sim_connected: "Simulate: connected",
    sim_docked: "Simulate: off on its dock",
    sim_disconnected: "Simulate: nothing connected",
    sim_minus: "One point lower",
    sim_plus: "One point higher",

    hid_unavailable: |e| format!("HID access failed: {e}"),
    low_body: |p| format!("{p}% of charge left."),
    sim_level: |p| format!("Simulate the level  ({p}%)"),
    percent_of: |p| format!("{p}%"),
    volts: |mv| format!(" ({}.{:02} V)", mv / 1000, (mv % 1000) / 10),
};

const FR: Strings = Strings {
    chime: "Faire sonner la manette",
    show_percent: "Afficher le pourcentage",
    autostart: "Démarrer avec Windows",
    quit: "Quitter",

    controller: "Manette Steam",
    charging: "en charge",
    charged: "chargée",
    reading: "Manette Steam — lecture en cours",

    no_dongle: "Aucun dongle Steam détecté",
    controller_offline: "Manette éteinte ou hors de portée",
    controller_docked: "Manette éteinte sur son socle — niveau inconnu",
    controller_busy: "Manette occupée par un autre logiciel",

    low_title: "Batterie de la manette faible",

    sim_prefix: "[simulation] ",
    sim_charging: "Simuler la charge",
    sim_connected: "Simuler : connectée",
    sim_docked: "Simuler : éteinte sur le socle",
    sim_disconnected: "Simuler : rien de connecté",
    sim_minus: "Un point de moins",
    sim_plus: "Un point de plus",

    hid_unavailable: |e| format!("Accès HID impossible : {e}"),
    low_body: |p| format!("Il reste {p} % de charge."),
    sim_level: |p| format!("Simuler le niveau  ({p} %)"),
    percent_of: |p| format!("{p} %"),
    // La virgule décimale et l'espace insécable avant l'unité sont d'usage.
    volts: |mv| format!(" ({},{:02} V)", mv / 1000, (mv % 1000) / 10),
};

/// La langue retenue pour cette exécution. Décidée une fois, au premier appel.
pub fn lang() -> Lang {
    static CHOSEN: OnceLock<Lang> = OnceLock::new();
    *CHOSEN.get_or_init(|| from_args().unwrap_or_else(detect))
}

/// Raccourci vers les libellés.
pub fn t() -> &'static Strings {
    lang().strings()
}

/// `--lang fr` ou `--lang=fr`. Un code inconnu est ignoré plutôt que fatal :
/// une faute de frappe ne doit pas empêcher l'application de démarrer.
pub fn from_args() -> Option<Lang> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for (i, arg) in args.iter().enumerate() {
        if let Some(code) = arg.strip_prefix("--lang=") {
            return Lang::from_code(code);
        }
        if arg == "--lang" {
            return args.get(i + 1).and_then(|c| Lang::from_code(c));
        }
    }
    None
}

/// La langue du système, quand elle est lisible. L'anglais est le défaut :
/// c'est la langue du dépôt, et celle qui parle au plus grand nombre.
pub fn detect() -> Lang {
    match os_locale() {
        Some(tag) if tag.to_ascii_lowercase().starts_with("fr") => Lang::Fr,
        _ => Lang::En,
    }
}

fn os_locale() -> Option<String> {
    extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }
    // LOCALE_NAME_MAX_LENGTH
    let mut buf = [0u16; 85];
    let written = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if written <= 0 {
        return None;
    }
    // le compte inclut le zéro terminal
    let end = (written as usize).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_round_trip() {
        for lang in [Lang::En, Lang::Fr] {
            assert_eq!(Lang::from_code(lang.code()), Some(lang));
        }
        assert_eq!(Lang::from_code("FR"), Some(Lang::Fr));
        assert_eq!(Lang::from_code("kl"), None);
        assert_eq!(Lang::from_code(""), None);
    }

    #[test]
    fn no_label_is_left_empty() {
        // Un champ oublié ne compile pas ; un champ rempli d'une chaîne vide,
        // si. C'est le seul trou que la structure ne bouche pas seule.
        for lang in [Lang::En, Lang::Fr] {
            let s = lang.strings();
            let fixed = [
                s.chime, s.show_percent, s.autostart, s.quit,
                s.controller, s.charging, s.charged, s.reading,
                s.no_dongle, s.controller_offline, s.controller_docked, s.controller_busy,
                s.low_title, s.sim_prefix, s.sim_charging, s.sim_connected,
                s.sim_docked, s.sim_disconnected, s.sim_minus, s.sim_plus,
            ];
            for (i, label) in fixed.iter().enumerate() {
                assert!(!label.trim().is_empty(), "libellé {i} vide en {}", lang.code());
            }
        }
    }

    #[test]
    fn parameterised_labels_carry_their_value() {
        for lang in [Lang::En, Lang::Fr] {
            let s = lang.strings();
            assert!((s.low_body)(17).contains("17"), "{}", lang.code());
            assert!((s.sim_level)(42).contains("42"), "{}", lang.code());
            assert!((s.percent_of)(99).contains("99"), "{}", lang.code());
            assert!((s.hid_unavailable)("boum").contains("boum"), "{}", lang.code());
            assert!((s.volts)(4119).contains('V'), "{}", lang.code());
        }
    }

    #[test]
    fn the_two_languages_actually_differ() {
        // Une traduction copiée-collée passerait les autres tests sans bruit.
        let (en, fr) = (Lang::En.strings(), Lang::Fr.strings());
        assert_ne!(en.chime, fr.chime);
        assert_ne!(en.quit, fr.quit);
        assert_ne!(en.controller_docked, fr.controller_docked);
        assert_ne!((en.low_body)(10), (fr.low_body)(10));
    }

    #[test]
    fn the_decimal_separator_follows_the_language() {
        assert!((Lang::En.strings().volts)(4119).contains("4.11"));
        assert!((Lang::Fr.strings().volts)(4119).contains("4,11"));
    }

    #[test]
    fn an_unknown_language_falls_back_rather_than_failing() {
        // `from_args` lit la vraie ligne de commande, qui n'a pas de --lang
        // sous `cargo test` : on vérifie surtout que rien ne panique.
        let _ = from_args();
        assert!(matches!(detect(), Lang::En | Lang::Fr));
    }
}
