//! Mode de simulation, pour éprouver l'affichage sans le matériel.
//!
//! Certains états sont pénibles à provoquer pour de vrai : une batterie à 8 %
//! se mérite, et vérifier le rendu sur barre des tâches claire suppose de
//! changer le thème du système. Ce mode remplace la lecture HID par des
//! valeurs qu'on choisit dans le menu.
//!
//! Il ne s'active que par un paramètre de lancement. Sans lui, rien de tout
//! ceci n'existe : ni menu, ni état, ni branche dans la lecture.

use std::sync::{Mutex, OnceLock};

use crate::hid::{BatteryStatus, ProbeError};

/// Paramètre qui déclenche le mode.
pub const FLAG: &str = "--debug";

/// Situation simulée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Manette allumée et connectée.
    Connected,
    /// Éteinte sur son socle.
    Docked,
    /// Rien de connecté.
    Disconnected,
}

#[derive(Debug, Clone, Copy)]
pub struct Sim {
    pub percent: u8,
    pub charging: bool,
    pub mode: Mode,
}

impl Default for Sim {
    fn default() -> Self {
        Self { percent: 72, charging: false, mode: Mode::Connected }
    }
}

static STATE: Mutex<Sim> = Mutex::new(Sim { percent: 72, charging: false, mode: Mode::Connected });

/// Le mode est-il actif ? Décidé une fois, au premier appel.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::args().skip(1).any(|a| a == FLAG))
}

pub fn get() -> Sim {
    STATE.lock().map(|s| *s).unwrap_or_default()
}

/// Applique une modification. Rend l'état obtenu.
pub fn update(f: impl FnOnce(&mut Sim)) -> Sim {
    match STATE.lock() {
        Ok(mut s) => {
            f(&mut s);
            s.percent = s.percent.min(100);
            *s
        }
        Err(_) => Sim::default(),
    }
}

/// Le relevé correspondant à l'état simulé, dans la forme exacte que rend la
/// lecture réelle — c'est ce qui garantit que l'on éprouve le vrai chemin
/// d'affichage et non une variante.
pub fn reading() -> Result<BatteryStatus, ProbeError> {
    let s = get();
    match s.mode {
        Mode::Connected => Ok(BatteryStatus {
            percent: s.percent,
            voltage_mv: Some(3400 + (s.percent as u16) * 8),
            charging: s.charging,
            full: s.charging && s.percent >= 100,
        }),
        Mode::Docked => Err(ProbeError::ControllerDocked),
        Mode::Disconnected => Err(ProbeError::NoDevice),
    }
}

/// Mention ajoutée à l'infobulle. Sans elle, on finirait par prendre une
/// valeur inventée pour une mesure — exactement l'erreur que ce mode est censé
/// aider à débusquer.
pub const TOOLTIP_PREFIX: &str = "[simulation] ";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_simulated_reading_mirrors_the_real_shape() {
        update(|s| {
            s.mode = Mode::Connected;
            s.percent = 42;
            s.charging = true;
        });
        let r = reading().expect("relevé");
        assert_eq!(r.percent, 42);
        assert!(r.charging);
        assert!(!r.full, "42 % ne peut pas être une charge terminée");
        assert!(r.voltage_mv.is_some_and(|v| (3000..=4500).contains(&v)));
    }

    #[test]
    fn the_modes_map_onto_the_real_outcomes() {
        update(|s| s.mode = Mode::Docked);
        assert_eq!(reading().unwrap_err(), ProbeError::ControllerDocked);
        update(|s| s.mode = Mode::Disconnected);
        assert_eq!(reading().unwrap_err(), ProbeError::NoDevice);
        update(|s| s.mode = Mode::Connected);
        assert!(reading().is_ok());
    }

    #[test]
    fn the_level_stays_within_bounds() {
        assert_eq!(update(|s| s.percent = 250).percent, 100);
        assert_eq!(update(|s| s.percent = 0).percent, 0);
    }

    #[test]
    fn a_full_charge_is_reported_as_finished() {
        update(|s| {
            s.mode = Mode::Connected;
            s.percent = 100;
            s.charging = true;
        });
        assert!(reading().unwrap().full);
    }
}
