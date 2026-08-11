//! Lecture de l'état de batterie de la Steam Controller 2026.
//!
//! Ce module ne connaît ni Win32 ni la zone de notification : il ouvre le
//! périphérique, lit ce qu'il faut, referme, et rend une valeur.
//!
//! # Le protocole, tel qu'établi par sondage du matériel
//!
//! Le dongle (« puck », PID 0x1304) expose plusieurs interfaces HID en
//! `usage_page` 0xFF00. Celle d'`usage` 0x0002 est l'interface de contrôle ;
//! les autres, d'`usage` 0x0001, sont les emplacements d'appairage, dont une
//! seule émet — celle où la manette est effectivement connectée.
//!
//! Les rapports d'entrée sont *numérotés*. Deux nous intéressent :
//!
//! | identifiant | débit | contenu |
//! |---|---|---|
//! | 0x42 | ~270 Hz | état des boutons, axes, trackpads, gyroscope |
//! | 0x43 | ~0,3 Hz | **état d'alimentation** |
//!
//! Le rapport 0x43 fait quinze octets et se lit ainsi :
//!
//! ```text
//! [0] 0x43        identifiant du rapport
//! [1] état        0x01 sur batterie, 0x04 en charge
//! [2] pourcentage 0 à 100
//! [3] tension     16 bits petit-boutiste, en millivolts
//! [5] ...         seconde tension, rôle non établi
//! ```
//!
//! Ce format a été établi en comparant des relevés à des états connus : sur le
//! puck en charge, l'octet [2] valait 100 et l'octet [1] valait 0x04 ; hors du
//! puck, [2] est tombé à 94 — exactement la valeur rapportée par ailleurs —
//! et [1] à 0x01.
//!
//! # Ce qui a été écarté
//!
//! L'attribut 0x0B, obtenu par la commande 0x83 sur le canal de commande,
//! valait invariablement 4000 quel que soit l'état réel de la batterie, y
//! compris à 100 % en charge et à 94 % sur batterie. C'est une constante de
//! conception, pas une mesure. Les registres lus par la commande 0x89 sont de
//! la configuration, et le rapport 0x7B porte de la télémétrie radio.

use std::time::{Duration, Instant};

const VALVE_VID: u16 = 0x28DE;
const PID_CONTROLLER: u16 = 0x1302;
const PID_DONGLE: u16 = 0x1304;

const USAGE_PAGE_VENDOR: u16 = 0xFF00;
/// Emplacement d'appairage : c'est de là que sortent les rapports d'entrée.
const USAGE_SLOT: u16 = 0x0001;

/// Rapport d'entrée portant l'état d'alimentation.
const RPT_POWER: u8 = 0x43;
/// Longueur utile du rapport 0x43, identifiant compris.
const RPT_POWER_LEN: usize = 15;

/// Seul état observé signalant une charge en cours. Sur batterie, l'octet
/// vaut 0x01 ; les autres valeurs n'ont pas été rencontrées et sont traitées
/// comme « pas en charge ».
const STATE_CHARGING: u8 = 0x04;

/// Le rapport 0x43 arrive environ toutes les trois secondes et demie. Au delà
/// de ce délai, c'est que la manette ne parle plus.
const POWER_REPORT_TIMEOUT: Duration = Duration::from_secs(6);

/// Délai accordé à une interface pour prouver qu'elle émet.
const SLOT_PROBE_TIMEOUT: i32 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// Niveau de charge rapporté par la manette, de 0 à 100.
    pub percent: u8,
    /// Tension de la cellule en millivolts.
    pub voltage_mv: Option<u16>,
    pub charging: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// Aucun périphérique Valve n'est énuméré.
    NoDevice,
    /// Le dongle est là, mais la manette est éteinte, endormie ou hors de portée.
    ControllerOffline,
    /// Le périphérique existe mais refuse de s'ouvrir.
    Busy(String),
    /// `hidapi` n'a pas pu s'initialiser.
    HidUnavailable(String),
}

impl ProbeError {
    /// Texte destiné à l'infobulle de la zone de notification.
    pub fn tooltip(&self) -> String {
        match self {
            Self::NoDevice => "Aucun dongle Steam détecté".into(),
            Self::ControllerOffline => "Manette éteinte ou hors de portée".into(),
            Self::Busy(_) => "Manette occupée par un autre logiciel".into(),
            Self::HidUnavailable(e) => format!("Accès HID impossible : {e}"),
        }
    }
}

/// Décode un rapport 0x43. Fonction pure, testable sur des relevés figés.
///
/// Rend `None` pour tout ce qui n'est pas un rapport d'alimentation complet et
/// cohérent : un pourcentage au-delà de cent trahirait une erreur de cadrage
/// plutôt qu'une batterie trop pleine.
pub fn parse_power_report(data: &[u8]) -> Option<BatteryStatus> {
    if data.len() < RPT_POWER_LEN || data[0] != RPT_POWER {
        return None;
    }
    let percent = data[2];
    if percent > 100 {
        return None;
    }
    let voltage = u16::from_le_bytes([data[3], data[4]]);
    Some(BatteryStatus {
        percent,
        // Une tension nulle ou absurde vaut mieux tue qu'affichée de travers.
        voltage_mv: (2000..=5000).contains(&voltage).then_some(voltage),
        charging: data[1] == STATE_CHARGING,
    })
}

/// Lit les rapports entrants jusqu'à en trouver un d'alimentation.
///
/// Le flux 0x42 arrive à environ 270 Hz et se traverse à vide : il ne coûte
/// qu'une comparaison d'octet par rapport.
fn read_power_report(dev: &hidapi::HidDevice) -> Option<BatteryStatus> {
    let deadline = Instant::now() + POWER_REPORT_TIMEOUT;
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        match dev.read_timeout(&mut buf, 100) {
            Ok(n) if n > 0 => {
                if let Some(s) = parse_power_report(&buf[..n]) {
                    return Some(s);
                }
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

/// Interroge la manette et referme immédiatement le périphérique.
///
/// L'appel peut bloquer plusieurs secondes, le temps qu'un rapport
/// d'alimentation se présente : il n'a rien à faire sur un fil chargé de
/// traiter des messages de fenêtre.
pub fn probe() -> Result<BatteryStatus, ProbeError> {
    let api = hidapi::HidApi::new().map_err(|e| ProbeError::HidUnavailable(e.to_string()))?;

    let slots: Vec<_> = api
        .device_list()
        .filter(|i| {
            i.vendor_id() == VALVE_VID
                && i.usage_page() == USAGE_PAGE_VENDOR
                && (i.usage() == USAGE_SLOT || i.product_id() == PID_CONTROLLER)
        })
        .map(|i| i.path().to_owned())
        .collect();

    if slots.is_empty() {
        let any_valve = api.device_list().any(|i| {
            i.vendor_id() == VALVE_VID && matches!(i.product_id(), PID_CONTROLLER | PID_DONGLE)
        });
        return Err(if any_valve {
            ProbeError::ControllerOffline
        } else {
            ProbeError::NoDevice
        });
    }

    // Le dongle expose un emplacement par appairage possible ; seul celui d'une
    // manette allumée émet quoi que ce soit. On les départage par une lecture
    // courte avant de s'engager dans l'attente longue du rapport 0x43.
    let mut busy = None;
    for path in &slots {
        let dev = match api.open_path(path) {
            Ok(d) => d,
            Err(e) => {
                busy = Some(ProbeError::Busy(e.to_string()));
                continue;
            }
        };
        let mut buf = [0u8; 64];
        let emits = matches!(dev.read_timeout(&mut buf, SLOT_PROBE_TIMEOUT), Ok(n) if n > 0);
        if !emits {
            continue;
        }
        // Le tout premier rapport lu est parfois déjà le bon.
        if let Some(s) = parse_power_report(&buf) {
            return Ok(s);
        }
        if let Some(s) = read_power_report(&dev) {
            return Ok(s);
        }
    }

    Err(busy.unwrap_or(ProbeError::ControllerOffline))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relevé réel, manette sur le puck : 100 % et en charge.
    const ON_PUCK: &[u8] = &[
        0x43, 0x04, 0x64, 0x2F, 0x10, 0x40, 0x10, 0xFC, 0x12, 0x00, 0x00, 0x85, 0x00, 0xB6, 0x7A,
    ];

    /// Relevé réel, manette hors du puck : 94 %, valeur confirmée par ailleurs.
    const ON_BATTERY: &[u8] = &[
        0x43, 0x01, 0x5E, 0x17, 0x10, 0x2C, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xB6, 0x7A,
    ];

    /// Relevé réel du flux d'état, qui ne doit jamais être pris pour un
    /// rapport d'alimentation.
    const STATE_STREAM: &[u8] = &[
        0x42, 0x7D, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6D, 0x00, 0x34, 0xFE, 0x19,
        0xFF, 0x5E, 0x02,
    ];

    #[test]
    fn reads_the_confirmed_ninety_four_percent() {
        let s = parse_power_report(ON_BATTERY).expect("rapport valide");
        assert_eq!(s.percent, 94);
        assert_eq!(s.voltage_mv, Some(4119));
        assert!(!s.charging);
    }

    #[test]
    fn reads_the_charging_reading() {
        let s = parse_power_report(ON_PUCK).expect("rapport valide");
        assert_eq!(s.percent, 100);
        assert_eq!(s.voltage_mv, Some(4143));
        assert!(s.charging, "l'octet d'état 0x04 signale la charge");
    }

    #[test]
    fn the_state_stream_is_not_mistaken_for_a_power_report() {
        assert!(parse_power_report(STATE_STREAM).is_none());
    }

    #[test]
    fn truncated_reports_are_rejected_without_panicking() {
        for n in 0..ON_BATTERY.len() {
            assert!(parse_power_report(&ON_BATTERY[..n]).is_none(), "accepté à {n} octets");
        }
        assert!(parse_power_report(&[]).is_none());
    }

    #[test]
    fn an_impossible_percentage_is_refused() {
        let mut bad = ON_BATTERY.to_vec();
        bad[2] = 101;
        assert!(parse_power_report(&bad).is_none());
        bad[2] = 255;
        assert!(parse_power_report(&bad).is_none());
    }

    #[test]
    fn an_absurd_voltage_is_dropped_but_the_level_is_kept() {
        let mut odd = ON_BATTERY.to_vec();
        odd[3] = 0x00;
        odd[4] = 0x00;
        let s = parse_power_report(&odd).expect("le niveau reste lisible");
        assert_eq!(s.percent, 94);
        assert_eq!(s.voltage_mv, None, "une tension nulle ne doit pas s'afficher");
    }

    #[test]
    fn only_the_documented_state_means_charging() {
        for state in 0u8..=255 {
            let mut r = ON_BATTERY.to_vec();
            r[1] = state;
            let s = parse_power_report(&r).unwrap();
            assert_eq!(
                s.charging,
                state == STATE_CHARGING,
                "état {state:#04X} mal interprété"
            );
        }
        assert!(!parse_power_report(ON_BATTERY).unwrap().charging);
    }

    #[test]
    fn extra_trailing_bytes_are_tolerated() {
        // hidapi rend parfois un tampon plus long que le rapport déclaré.
        let mut padded = ON_BATTERY.to_vec();
        padded.extend_from_slice(&[0xB6, 0x7A, 0xFE, 0x00, 0x00]);
        assert_eq!(parse_power_report(&padded).unwrap().percent, 94);
    }

    /// Vérification sur matériel réel :
    /// `cargo test -- --ignored --nocapture`
    #[test]
    #[ignore = "exige une Steam Controller 2026 allumée"]
    fn reads_the_actual_hardware() {
        match probe() {
            Ok(s) => {
                println!("relevé matériel : {s:?}");
                assert!(s.percent <= 100);
            }
            Err(e) => panic!("lecture impossible : {e:?} — {}", e.tooltip()),
        }
    }
}
