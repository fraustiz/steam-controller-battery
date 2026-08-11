//! Lecture de l'état de batterie de la Steam Controller 2026.
//!
//! Ce module ne connaît ni Win32 ni la zone de notification : il ouvre le
//! périphérique, pose une question, referme, et rend une valeur.
//!
//! # Le protocole, tel qu'établi par sondage du matériel
//!
//! Le dongle (« puck », PID 0x1304) expose plusieurs interfaces HID en
//! `usage_page` 0xFF00. Celle d'`usage` 0x0002 est l'interface de contrôle ;
//! les autres, d'`usage` 0x0001, sont les emplacements d'appairage, dont une
//! seule émet — celle où la manette est effectivement connectée.
//!
//! Les commandes passent par des *feature reports numérotés* de 64 octets
//! (1 octet d'identifiant + 63 de charge utile) :
//!
//! ```text
//! [id_rapport, commande, longueur_args, args...]
//! ```
//!
//! L'identifiant de rapport choisit le destinataire :
//!
//! | id | destinataire |
//! |----|--------------|
//! | 0x01 | la manette elle-même (PID 0x1302) |
//! | 0x02 | le dongle (PID 0x1304) |
//!
//! La réponse se relit sur le même identifiant, au format
//! `[id_rapport, écho_commande, longueur, charge_utile...]`. Le canal est
//! asynchrone : la première relecture rend souvent la réponse précédente, il
//! faut donc boucler jusqu'à ce que l'écho corresponde.
//!
//! La commande 0x83 (`GET_ATTRIBUTES`) rend une suite de triplets
//! `(identifiant, valeur sur 32 bits en petit-boutiste)`. L'attribut 0x0B ne
//! figure que dans les attributs de la manette, jamais dans ceux du dongle qui
//! n'a pas de batterie.

use std::collections::BTreeMap;
use std::thread::sleep;
use std::time::Duration;

const VALVE_VID: u16 = 0x28DE;
const PID_CONTROLLER: u16 = 0x1302;
const PID_DONGLE: u16 = 0x1304;

const USAGE_PAGE_VENDOR: u16 = 0xFF00;
const USAGE_CONTROL: u16 = 0x0002;

const FEATURE_LEN: usize = 64;

/// Canal de commande vers la manette.
const CH_CONTROLLER: u8 = 0x01;

/// `GET_ATTRIBUTES` : rend les triplets (identifiant, valeur u32).
const CMD_GET_ATTRIBUTES: u8 = 0x83;

/// Identifiant du produit, dans les attributs. Sert à distinguer la manette du dongle.
const ATTR_PRODUCT_ID: u8 = 0x01;

/// Attribut de batterie. Absent des attributs du dongle.
const ATTR_BATTERY: u8 = 0x0B;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// Niveau de charge, de 0 à 100.
    pub percent: u8,
    /// Tension mesurée en millivolts, si le matériel la donne plutôt qu'un pourcentage.
    pub voltage_mv: Option<u16>,
    pub charging: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// Aucun périphérique Valve en `usage_page` 0xFF00 n'est énuméré.
    NoDevice,
    /// Le dongle est là, mais la manette est éteinte ou hors de portée.
    ControllerOffline,
    /// Le périphérique existe mais refuse de s'ouvrir (accaparé par un autre logiciel).
    Busy(String),
    /// La manette a répondu, mais sans attribut de batterie exploitable.
    NoBatteryAttribute,
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
            Self::NoBatteryAttribute => "Niveau de batterie indisponible".into(),
            Self::HidUnavailable(e) => format!("Accès HID impossible : {e}"),
        }
    }
}

/// Convertit une tension de cellule Li-ion en pourcentage de charge.
///
/// La courbe de décharge n'a rien de linéaire : elle s'effondre sous 3,7 V et
/// s'aplatit au-dessus de 4,1 V. Interpolation linéaire entre les points d'une
/// courbe de référence pour cellule unique.
pub fn voltage_to_percent(mv: u16) -> u8 {
    const CURVE: &[(u16, u8)] = &[
        (3000, 0),
        (3300, 5),
        (3450, 10),
        (3550, 20),
        (3650, 35),
        (3750, 50),
        (3850, 65),
        (3950, 75),
        (4050, 85),
        (4150, 95),
        (4200, 100),
    ];

    if mv <= CURVE[0].0 {
        return 0;
    }
    if mv >= CURVE[CURVE.len() - 1].0 {
        return 100;
    }
    for w in CURVE.windows(2) {
        let ((v0, p0), (v1, p1)) = (w[0], w[1]);
        if mv <= v1 {
            let span = (v1 - v0) as u32;
            let into = (mv - v0) as u32;
            let rise = (p1 - p0) as u32;
            return (p0 as u32 + into * rise / span) as u8;
        }
    }
    100
}

/// Interprète l'attribut de batterie, qui peut porter soit un pourcentage
/// direct, soit une tension en millivolts. Les deux domaines ne se recouvrent
/// pas, ce qui rend la distinction sûre.
pub fn interpret_battery(raw: u32) -> Option<(u8, Option<u16>)> {
    match raw {
        0..=100 => Some((raw as u8, None)),
        2500..=4500 => {
            let mv = raw as u16;
            Some((voltage_to_percent(mv), Some(mv)))
        }
        _ => None,
    }
}

/// Décode la charge utile de `GET_ATTRIBUTES` : des triplets de 5 octets.
pub fn parse_attributes(payload: &[u8]) -> BTreeMap<u8, u32> {
    payload
        .chunks_exact(5)
        .map(|c| (c[0], u32::from_le_bytes([c[1], c[2], c[3], c[4]])))
        .collect()
}

/// Construit l'état de batterie à partir des attributs bruts de la manette.
///
/// Séparée de toute entrée-sortie pour être testable sur des relevés figés.
pub fn status_from_attributes(attrs: &BTreeMap<u8, u32>) -> Result<BatteryStatus, ProbeError> {
    if attrs.get(&ATTR_PRODUCT_ID) != Some(&(PID_CONTROLLER as u32)) {
        return Err(ProbeError::ControllerOffline);
    }
    let raw = attrs.get(&ATTR_BATTERY).ok_or(ProbeError::NoBatteryAttribute)?;
    let (percent, voltage_mv) = interpret_battery(*raw).ok_or(ProbeError::NoBatteryAttribute)?;
    Ok(BatteryStatus {
        percent,
        voltage_mv,
        // La charge se déduit d'une tension au-delà du repos : une cellule au
        // repos ne dépasse pas 4,2 V, une cellule en charge est tirée plus haut.
        charging: voltage_mv.is_some_and(|mv| mv > 4200),
    })
}

/// Émet une commande sur un canal et attend la réponse dont l'écho correspond.
fn command(dev: &hidapi::HidDevice, channel: u8, cmd: u8, args: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![0u8; FEATURE_LEN];
    out[0] = channel;
    out[1] = cmd;
    out[2] = args.len() as u8;
    out[3..3 + args.len()].copy_from_slice(args);
    dev.send_feature_report(&out).ok()?;

    // Le canal est asynchrone : on relit jusqu'à voir l'écho de *notre* commande.
    for _ in 0..25 {
        let mut buf = vec![0u8; FEATURE_LEN];
        buf[0] = channel;
        if let Ok(n) = dev.get_feature_report(&mut buf) {
            if n > 3 && buf[1] == cmd {
                let len = (buf[2] as usize).min(n - 3);
                return Some(buf[3..3 + len].to_vec());
            }
        }
        sleep(Duration::from_millis(8));
    }
    None
}

/// Interroge la manette et referme immédiatement le périphérique.
///
/// L'ouverture et la fermeture à chaque appel sont délibérées : garder le
/// descripteur ouvert obligerait à drainer un flux de rapports d'entrée à
/// environ 270 Hz, ce qui coûterait du processeur en permanence pour une
/// donnée qui bouge toutes les vingt minutes.
pub fn probe() -> Result<BatteryStatus, ProbeError> {
    let api = hidapi::HidApi::new().map_err(|e| ProbeError::HidUnavailable(e.to_string()))?;

    let paths: Vec<_> = api
        .device_list()
        .filter(|i| {
            i.vendor_id() == VALVE_VID
                && i.usage_page() == USAGE_PAGE_VENDOR
                && (i.usage() == USAGE_CONTROL || i.product_id() == PID_CONTROLLER)
        })
        .map(|i| i.path().to_owned())
        .collect();

    if paths.is_empty() {
        // Un dongle sans interface de contrôle reste un dongle : on distingue
        // « rien de branché » de « branché mais muet ».
        let any_valve = api.device_list().any(|i| {
            i.vendor_id() == VALVE_VID && matches!(i.product_id(), PID_CONTROLLER | PID_DONGLE)
        });
        return Err(if any_valve {
            ProbeError::ControllerOffline
        } else {
            ProbeError::NoDevice
        });
    }

    let mut last = ProbeError::ControllerOffline;
    for path in &paths {
        let dev = match api.open_path(path) {
            Ok(d) => d,
            Err(e) => {
                last = ProbeError::Busy(e.to_string());
                continue;
            }
        };
        let Some(payload) = command(&dev, CH_CONTROLLER, CMD_GET_ATTRIBUTES, &[]) else {
            continue;
        };
        match status_from_attributes(&parse_attributes(&payload)) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relevé réel de la manette, capturé par la sonde : la charge utile de
    /// `GET_ATTRIBUTES` sur le canal 0x01.
    const REAL_PAYLOAD: &[u8] = &[
        0x01, 0x02, 0x13, 0x00, 0x00, // product id = 0x1302
        0x02, 0x00, 0x00, 0x00, 0x00, // capabilities = 0
        0x0A, 0x2E, 0xF9, 0xD2, 0x68, // bootloader build time
        0x04, 0xE3, 0x85, 0x4D, 0x6A, // firmware build time
        0x09, 0x49, 0x00, 0x00, 0x00, // board revision = 73
        0x0B, 0xA0, 0x0F, 0x00, 0x00, // batterie = 4000
    ];

    /// Relevé réel du dongle : mêmes attributs, sans le 0x0B.
    const DONGLE_PAYLOAD: &[u8] = &[
        0x01, 0x04, 0x13, 0x00, 0x00, // product id = 0x1304
        0x02, 0x00, 0x00, 0x00, 0x00,
        0x0A, 0xF2, 0xF9, 0xD2, 0x68,
        0x04, 0xDE, 0x23, 0x44, 0x6A,
        0x09, 0x47, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn parses_the_real_controller_reading() {
        let attrs = parse_attributes(REAL_PAYLOAD);
        assert_eq!(attrs[&ATTR_PRODUCT_ID], 0x1302);
        assert_eq!(attrs[&ATTR_BATTERY], 4000);
    }

    #[test]
    fn real_reading_yields_a_plausible_level() {
        let s = status_from_attributes(&parse_attributes(REAL_PAYLOAD)).unwrap();
        assert_eq!(s.voltage_mv, Some(4000));
        assert_eq!(s.percent, voltage_to_percent(4000));
        assert!(!s.charging);
    }

    #[test]
    fn dongle_is_not_mistaken_for_the_controller() {
        let e = status_from_attributes(&parse_attributes(DONGLE_PAYLOAD)).unwrap_err();
        assert_eq!(e, ProbeError::ControllerOffline);
    }

    #[test]
    fn controller_without_battery_attribute_is_reported_as_such() {
        let attrs = parse_attributes(&REAL_PAYLOAD[..25]); // les cinq premiers triplets
        assert_eq!(
            status_from_attributes(&attrs).unwrap_err(),
            ProbeError::NoBatteryAttribute
        );
    }

    #[test]
    fn truncated_payload_does_not_panic() {
        for n in 0..REAL_PAYLOAD.len() {
            let _ = status_from_attributes(&parse_attributes(&REAL_PAYLOAD[..n]));
        }
    }

    #[test]
    fn battery_attribute_reads_as_percent_or_millivolts() {
        assert_eq!(interpret_battery(87), Some((87, None)));
        assert_eq!(interpret_battery(100), Some((100, None)));
        assert_eq!(interpret_battery(4000), Some((voltage_to_percent(4000), Some(4000))));
        // Entre les deux domaines : ni un pourcentage, ni une tension de cellule.
        assert_eq!(interpret_battery(1500), None);
        assert_eq!(interpret_battery(0xFFFF_FFFF), None);
    }

    #[test]
    fn voltage_curve_is_monotonic_and_bounded() {
        let mut prev = 0;
        for mv in 2800..=4400u16 {
            let p = voltage_to_percent(mv);
            assert!(p >= prev, "recul à {mv} mV : {prev} -> {p}");
            assert!(p <= 100);
            prev = p;
        }
        assert_eq!(voltage_to_percent(2800), 0);
        assert_eq!(voltage_to_percent(4400), 100);
    }

    #[test]
    fn voltage_curve_hits_its_reference_points() {
        assert_eq!(voltage_to_percent(3750), 50);
        assert_eq!(voltage_to_percent(4200), 100);
        assert_eq!(voltage_to_percent(3000), 0);
    }

    /// Vérification sur matériel réel. Exclue de la série normale, puisqu'elle
    /// exige une manette allumée :
    /// `cargo test -- --ignored --nocapture`
    #[test]
    #[ignore = "exige une Steam Controller 2026 connectée"]
    fn reads_the_actual_hardware() {
        match probe() {
            Ok(s) => {
                println!("relevé matériel : {s:?}");
                assert!(s.percent <= 100);
            }
            Err(e) => panic!("lecture impossible : {e:?} — {}", e.tooltip()),
        }
    }

    #[test]
    fn charging_is_inferred_above_resting_voltage() {
        let charging = parse_attributes(&[0x01, 0x02, 0x13, 0x00, 0x00, 0x0B, 0x50, 0x11, 0x00, 0x00]);
        assert_eq!(charging[&ATTR_BATTERY], 4432);
        assert!(status_from_attributes(&charging).unwrap().charging);
    }
}
