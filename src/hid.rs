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
//! [0] 0x43         identifiant du rapport
//! [1] état         0x01 décharge, 0x02 en charge, 0x04 chargée
//! [2] pourcentage  0 à 100
//! [3] tension      cellule, 16 bits petit-boutiste, en millivolts
//! [5] tension      seconde valeur, rôle non établi
//! [7] alimentation 16 bits petit-boutiste, en millivolts ; nulle hors secteur
//! [9] courant      16 bits petit-boutiste, en milliampères ; nul à pleine charge
//! ```
//!
//! Ce format a été établi en confrontant des relevés à des états connus. Hors
//! du puck, [2] valait 94 — exactement la valeur rapportée par ailleurs.
//!
//! L'octet [1] a d'abord été mal lu. Un unique relevé sur le puck le montrait à
//! 0x04, d'où la conclusion hâtive « 0x04 signifie en charge ». La batterie y
//! était en réalité déjà pleine : 0x04 veut dire « chargée », et c'est 0x02 qui
//! signale une charge en cours. Généraliser depuis un seul point de mesure
//! avait produit un indicateur qui ne s'allumait jamais.
//!
//! De cette correction vient le choix de ne pas faire reposer la détection sur
//! le seul octet d'état : les octets [7] et [9] sont des mesures — tension
//! d'alimentation et courant de charge — et une mesure ment moins qu'un code
//! dont on n'a pas vu toutes les valeurs.
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

/// États observés dans l'octet [1].
const STATE_CHARGING: u8 = 0x02;
const STATE_CHARGED: u8 = 0x04;

/// Tension minimale pour tenir l'alimentation pour réelle. Une source branchée
/// se lit autour de 4800 mV ; débranchée, l'octet tombe franchement à zéro.
const SUPPLY_PRESENT_MV: u16 = 3000;

/// Le rapport 0x43 arrive environ toutes les trois secondes et demie. Au delà
/// de ce délai, c'est que la manette ne parle plus.
#[cfg(test)]
const POWER_REPORT_TIMEOUT: Duration = Duration::from_secs(6);

/// Délai accordé à une interface pour prouver qu'elle émet.
const SLOT_PROBE_TIMEOUT: i32 = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryStatus {
    /// Niveau de charge rapporté par la manette, de 0 à 100.
    pub percent: u8,
    /// Tension de la cellule en millivolts.
    pub voltage_mv: Option<u16>,
    /// La manette est alimentée : posée sur son socle ou reliée par câble.
    pub charging: bool,
    /// Alimentée, mais la charge est terminée — le courant est retombé à zéro.
    pub full: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// Aucun périphérique Valve n'est énuméré.
    NoDevice,
    /// Le dongle est là, mais la manette est éteinte, endormie ou hors de portée.
    ControllerOffline,
    /// La manette est éteinte, mais elle répond encore au dongle : elle est
    /// donc à portée immédiate, c'est-à-dire posée sur son socle. Son niveau de
    /// charge, lui, est hors d'atteinte — il ne circule que dans les rapports
    /// d'entrée, qu'une manette éteinte n'émet pas.
    ControllerDocked,
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
            Self::ControllerDocked => "Manette éteinte sur son socle — niveau inconnu".into(),
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
    let supply_mv = u16::from_le_bytes([data[7], data[8]]);
    let current_ma = u16::from_le_bytes([data[9], data[10]]);
    let state = data[1];

    // La tension d'alimentation prime sur l'octet d'état : c'est une mesure,
    // et elle reste vraie même pour un état que nous n'aurions jamais observé.
    let charging = supply_mv >= SUPPLY_PRESENT_MV
        || matches!(state, STATE_CHARGING | STATE_CHARGED);

    Some(BatteryStatus {
        percent,
        // Une tension nulle ou absurde vaut mieux tue qu'affichée de travers.
        voltage_mv: (2000..=5000).contains(&voltage).then_some(voltage),
        charging,
        // Plus de courant qui entre alors que la source est là : c'est fini.
        full: charging && (state == STATE_CHARGED || current_ma == 0),
    })
}

/// Lit les rapports entrants jusqu'à en trouver un d'alimentation.
///
/// L'application, elle, écoute en continu par `run_reader` ; ce chemin ne sert
/// qu'à la vérification ponctuelle sur matériel.
///
/// Le flux 0x42 arrive à environ 270 Hz et se traverse à vide : il ne coûte
/// qu'une comparaison d'octet par rapport.
#[cfg(test)]
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

/// Ouvre l'emplacement du dongle où une manette émet réellement.
///
/// Le dongle expose un emplacement par appairage possible ; seul celui d'une
/// manette allumée produit quoi que ce soit. On les départage par une lecture
/// courte. Rend aussi le premier rapport lu, qui est parfois déjà le bon.
fn open_emitting_slot(
    api: &hidapi::HidApi,
) -> Result<(hidapi::HidDevice, Option<BatteryStatus>), ProbeError> {
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
        if matches!(dev.read_timeout(&mut buf, SLOT_PROBE_TIMEOUT), Ok(n) if n > 0) {
            let first = parse_power_report(&buf);
            return Ok((dev, first));
        }
    }
    Err(busy.unwrap_or(ProbeError::ControllerOffline))
}

/// Un relevé ponctuel, pour la vérification sur matériel réel.
#[cfg(test)]
pub fn probe() -> Result<BatteryStatus, ProbeError> {
    let api = hidapi::HidApi::new().map_err(|e| ProbeError::HidUnavailable(e.to_string()))?;
    let (dev, first) = open_emitting_slot(&api)?;
    if let Some(s) = first {
        return Ok(s);
    }
    read_power_report(&dev).ok_or(ProbeError::ControllerOffline)
}

/// Canal de commande visant la manette dans les rapports de fonctionnalité.
const CH_CONTROLLER: u8 = 0x01;
/// Lecture des attributs. Sert ici de simple « es-tu là ? ».
const CMD_GET_ATTRIBUTES: u8 = 0x83;
/// Longueur d'un rapport de fonctionnalité, identifiant compris.
const FEATURE_LEN: usize = 64;

/// La manette est-elle éteinte et posée sur son socle ?
///
/// Le routage du canal 0x01 s'inverse selon l'état de la manette :
///
/// | | emplacement (usage 0x0001) | contrôle (usage 0x0002) |
/// |---|---|---|
/// | allumée | répond | refusé |
/// | éteinte, sur le socle | refusé | **répond** |
/// | éteinte, à côté du PC hors socle | refusé | refusé |
/// | éteinte, éloignée | refusé | refusé |
///
/// Les deux derniers cas sont ce qui donne sa valeur au test. Une manette
/// éteinte posée à trente centimètres du dongle reste muette : ce n'est donc ni
/// un appairage mémorisé, ni de la simple portée radio. Seul le contact du
/// socle — celui-là même qui la recharge — ouvre le canal.
///
/// Le signal est par conséquent exact et non approximatif : il dit « sur le
/// socle », pas « quelque part à proximité ».
fn controller_answers_while_off(api: &hidapi::HidApi) -> bool {
    let Some(info) = api
        .device_list()
        .find(|i| i.vendor_id() == VALVE_VID && i.usage_page() == USAGE_PAGE_VENDOR && i.usage() == USAGE_CONTROL)
        .cloned()
    else {
        return false;
    };
    let Ok(dev) = info.open_device(api) else {
        return false;
    };

    let mut out = vec![0u8; FEATURE_LEN];
    out[0] = CH_CONTROLLER;
    out[1] = CMD_GET_ATTRIBUTES;
    if dev.send_feature_report(&out).is_err() {
        return false;
    }

    // Les premières relectures échouent régulièrement : la réponse fait un
    // aller-retour radio et n'est pas prête tout de suite.
    for _ in 0..20 {
        let mut b = vec![0u8; FEATURE_LEN];
        b[0] = CH_CONTROLLER;
        if let Ok(n) = dev.get_feature_report(&mut b) {
            if n > 3 && b[1] == CMD_GET_ATTRIBUTES {
                let len = (b[2] as usize).min(n - 3);
                return b[3..3 + len]
                    .chunks_exact(5)
                    .any(|c| c[0] == 0x01 && u16::from_le_bytes([c[1], c[2]]) == PID_CONTROLLER);
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Interface de contrôle du dongle.
const USAGE_CONTROL: u16 = 0x0002;

/// Délai au-delà duquel un silence signifie que la manette a décroché.
const SILENCE_TIMEOUT: Duration = Duration::from_secs(12);
/// Attente entre deux tentatives quand le dongle est là mais pas la manette.
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// Rapport de sortie pilotant les actionneurs haptiques.
const RPT_HAPTIC: u8 = 0x83;

/// Sonnerie, convertie depuis `persona_3_reload_phone.mid`.
///
/// Chaque entrée est `(attente en millisecondes avant l'événement,
/// actionneur, fréquence)`. Une fréquence nulle arrête l'actionneur.
const RINGTONE: &[(u16, u8, u16)] = &[
    (174, 3, 1008),
    (23, 0, 1105),
    (47, 3, 0),
    (11, 3, 2992),
    (0, 4, 1510),
    (12, 0, 0),
    (58, 3, 0),
    (23, 4, 0),
    (81, 4, 1008),
    (70, 4, 0),
    (24, 4, 1510),
    (81, 4, 0),
    (174, 4, 1897),
    (46, 3, 1008),
    (35, 4, 0),
    (23, 4, 1131),
    (12, 3, 0),
    (35, 3, 1510),
    (23, 4, 0),
    (58, 3, 0),
    (82, 3, 1008),
    (46, 4, 1131),
    (23, 3, 0),
    (12, 3, 1510),
    (34, 4, 0),
    (70, 3, 0),
];

/// Nombre de passages. Un seul : une sonnerie qui se répète devient vite
/// pénible, et celle-ci sert à identifier une manette autant qu'à la retrouver.
const RINGTONE_REPEATS: usize = 1;

/// Silence entre deux passages. Sans effet à un seul passage, mais la boucle
/// reste écrite pour en accepter davantage.
const RINGTONE_GAP: Duration = Duration::from_millis(450);

/// Tous les actionneurs de la manette. Sert à garantir le silence final, quels
/// que soient ceux que la mélodie a réellement employés.
const ALL_ACTUATORS: [u8; 5] = [0, 1, 2, 3, 4];

/// Amplitude maximale. L'octet est signé : 127 est le plus fort.
const LOCATOR_GAIN: u8 = 127;

/// Démarre une note sur un actionneur.
///
/// Le découpage de la fréquence en deux octets se fait modulo 255 et non 256.
/// C'est l'arithmétique du projet d'origine, vérifiée sur le matériel ; la
/// « corriger » en décalage de bits produit une note fausse.
fn haptic_on(actuator: u8, hz: u16) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = RPT_HAPTIC;
    b[1] = actuator;
    b[2] = LOCATOR_GAIN;
    b[3] = (hz % 0xFF) as u8;
    b[4] = (hz / 0xFF) as u8;
    b[5] = 0xFF;
    b[6] = 0x7F;
    b
}

fn haptic_off(actuator: u8) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[0] = RPT_HAPTIC;
    b[1] = actuator;
    b[2] = 0x80;
    b[6] = 0x80;
    b
}

/// Fait sonner la manette, pour la retrouver ou savoir de laquelle il s'agit.
///
/// La mélodie passe par les actionneurs haptiques, seule chose que cette
/// manette sache faire vibrer ou sonner. Les fréquences ont été converties
/// hors ligne depuis le fichier MIDI : embarquer un analyseur pour rejouer une
/// seconde et demie de musique aurait coûté une dépendance entière pour
/// quelques centaines d'octets de table.
///
/// Exige une manette allumée : les actionneurs d'une manette éteinte ne
/// reçoivent rien, même posée sur son socle.
pub fn play_locator_chime() -> Result<(), ProbeError> {
    let api = hidapi::HidApi::new().map_err(|e| ProbeError::HidUnavailable(e.to_string()))?;
    let (dev, _) = open_emitting_slot(&api)?;

    for _ in 0..RINGTONE_REPEATS {
        for &(wait_ms, actuator, freq) in RINGTONE {
            if wait_ms > 0 {
                std::thread::sleep(Duration::from_millis(wait_ms as u64));
            }
            let report = if freq == 0 {
                haptic_off(actuator)
            } else {
                haptic_on(actuator, freq)
            };
            let _ = dev.write(&report);
        }
        std::thread::sleep(RINGTONE_GAP);
    }

    // Silence garanti, même si une écriture s'est perdue en route : une manette
    // laissée en vibration continue serait pire que pas de sonnerie du tout.
    for a in ALL_ACTUATORS {
        let _ = dev.write(&haptic_off(a));
    }
    Ok(())
}

/// Écoute les rapports d'alimentation au fil de l'eau et les transmet à mesure.
///
/// C'est ce qui donne à l'application sa réactivité : plutôt que d'aller
/// chercher l'état toutes les trente secondes, on reste à l'écoute et la
/// manette nous prévient d'elle-même toutes les trois secondes et demie. Poser
/// la manette sur son socle se voit donc presque aussitôt.
///
/// La fonction ne rend la main que sur demande d'arrêt, ou lorsqu'il n'y a plus
/// le moindre périphérique Valve — auquel cas l'appelant n'a plus qu'à dormir
/// jusqu'au prochain branchement.
pub fn run_reader(
    should_stop: &std::sync::atomic::AtomicBool,
    mut on_status: impl FnMut(Result<BatteryStatus, ProbeError>),
) {
    use std::sync::atomic::Ordering;

    let stopping = || should_stop.load(Ordering::Relaxed);

    /// Sommeil découpé, pour rester réactif à une demande d'arrêt.
    fn nap(should_stop: &std::sync::atomic::AtomicBool, total: Duration) {
        let deadline = Instant::now() + total;
        while Instant::now() < deadline && !should_stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    while !stopping() {
        // L'énumération est refaite à chaque tour : c'est ainsi qu'on voit
        // apparaître un dongle branché entre-temps.
        let api = match hidapi::HidApi::new() {
            Ok(a) => a,
            Err(e) => {
                on_status(Err(ProbeError::HidUnavailable(e.to_string())));
                nap(should_stop, RETRY_DELAY);
                continue;
            }
        };

        let dev = match open_emitting_slot(&api) {
            Ok((dev, first)) => {
                if let Some(s) = first {
                    on_status(Ok(s));
                }
                dev
            }
            Err(ProbeError::NoDevice) => {
                // Plus rien de branché : inutile de garder un fil en vie.
                on_status(Err(ProbeError::NoDevice));
                return;
            }
            Err(ProbeError::ControllerOffline) => {
                // Muette, mais peut-être seulement éteinte sur son socle : le
                // canal du socle répond là où la radio s'est tue.
                let docked = controller_answers_while_off(&api);
                on_status(Err(if docked {
                    ProbeError::ControllerDocked
                } else {
                    ProbeError::ControllerOffline
                }));
                nap(should_stop, RETRY_DELAY);
                continue;
            }
            Err(e) => {
                on_status(Err(e));
                nap(should_stop, RETRY_DELAY);
                continue;
            }
        };

        // Écoute du flux. Le rapport 0x42 arrive à environ 270 Hz et se
        // traverse à vide ; seul 0x43 nous intéresse.
        let mut last_seen = Instant::now();
        let mut buf = [0u8; 64];
        while !stopping() {
            match dev.read_timeout(&mut buf, 500) {
                Ok(n) if n > 0 => {
                    if let Some(s) = parse_power_report(&buf[..n]) {
                        last_seen = Instant::now();
                        on_status(Ok(s));
                    }
                }
                Ok(_) => {
                    if last_seen.elapsed() > SILENCE_TIMEOUT {
                        break; // la manette s'est tue : on repart en découverte
                    }
                }
                Err(_) => break,
            }
        }
        if !stopping() {
            on_status(Err(ProbeError::ControllerOffline));
            nap(should_stop, RETRY_DELAY);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relevé réel, manette sur le puck, batterie pleine : alimentée, mais le
    /// courant de charge est retombé à zéro.
    const FULL_ON_PUCK: &[u8] = &[
        0x43, 0x04, 0x64, 0x2F, 0x10, 0x40, 0x10, 0xFC, 0x12, 0x00, 0x00, 0x85, 0x00, 0xB6, 0x7A,
    ];

    /// Relevé réel, charge réellement en cours : 96 %, alimentation à 4800 mV,
    /// courant de 175 mA.
    const CHARGING: &[u8] = &[
        0x43, 0x02, 0x60, 0x3F, 0x10, 0x68, 0x10, 0xC0, 0x12, 0xAF, 0x00, 0x31, 0x01, 0xCD, 0x78,
    ];

    /// Relevé réel, manette hors du puck : 94 %, valeur confirmée par ailleurs.
    const OFF_PUCK: &[u8] = &[
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
        let s = parse_power_report(OFF_PUCK).expect("rapport valide");
        assert_eq!(s.percent, 94);
        assert_eq!(s.voltage_mv, Some(4119));
        assert!(!s.charging);
    }

    #[test]
    fn charging_is_detected_while_actually_charging() {
        let s = parse_power_report(CHARGING).expect("rapport valide");
        assert_eq!(s.percent, 96);
        assert_eq!(s.voltage_mv, Some(4159));
        assert!(s.charging, "une charge en cours doit être signalée");
        assert!(!s.full, "96 % n'est pas une batterie pleine");
    }

    #[test]
    fn a_full_battery_on_power_is_reported_as_charged() {
        let s = parse_power_report(FULL_ON_PUCK).expect("rapport valide");
        assert_eq!(s.percent, 100);
        assert!(s.charging, "posée sur son socle, elle reste alimentée");
        assert!(s.full, "courant nul à 100 % : la charge est terminée");
    }

    #[test]
    fn the_three_power_states_are_distinguished() {
        let off = parse_power_report(OFF_PUCK).unwrap();
        let charging = parse_power_report(CHARGING).unwrap();
        let full = parse_power_report(FULL_ON_PUCK).unwrap();
        assert_eq!((off.charging, off.full), (false, false));
        assert_eq!((charging.charging, charging.full), (true, false));
        assert_eq!((full.charging, full.full), (true, true));
    }

    #[test]
    fn the_state_stream_is_not_mistaken_for_a_power_report() {
        assert!(parse_power_report(STATE_STREAM).is_none());
    }

    #[test]
    fn truncated_reports_are_rejected_without_panicking() {
        for n in 0..OFF_PUCK.len() {
            assert!(parse_power_report(&OFF_PUCK[..n]).is_none(), "accepté à {n} octets");
        }
        assert!(parse_power_report(&[]).is_none());
    }

    #[test]
    fn an_impossible_percentage_is_refused() {
        let mut bad = OFF_PUCK.to_vec();
        bad[2] = 101;
        assert!(parse_power_report(&bad).is_none());
        bad[2] = 255;
        assert!(parse_power_report(&bad).is_none());
    }

    #[test]
    fn an_absurd_voltage_is_dropped_but_the_level_is_kept() {
        let mut odd = OFF_PUCK.to_vec();
        odd[3] = 0x00;
        odd[4] = 0x00;
        let s = parse_power_report(&odd).expect("le niveau reste lisible");
        assert_eq!(s.percent, 94);
        assert_eq!(s.voltage_mv, None, "une tension nulle ne doit pas s'afficher");
    }

    #[test]
    fn charging_rests_on_the_measured_supply_not_on_the_state_byte_alone() {
        // L'octet d'état a déjà menti une fois : 0x04 voulait dire « chargée »
        // et non « en charge ». La détection s'appuie donc d'abord sur la
        // tension d'alimentation, qui est une mesure et non un code.
        for state in 0u8..=255 {
            let mut r = CHARGING.to_vec();
            r[1] = state;
            assert!(
                parse_power_report(&r).unwrap().charging,
                "alimentation présente ignorée pour l'état {state:#04X}"
            );

            let mut unplugged = OFF_PUCK.to_vec();
            unplugged[1] = state;
            let s = parse_power_report(&unplugged).unwrap();
            assert_eq!(
                s.charging,
                matches!(state, STATE_CHARGING | STATE_CHARGED),
                "sans alimentation mesurée, seul l'état peut trancher ({state:#04X})"
            );
        }
    }

    #[test]
    fn extra_trailing_bytes_are_tolerated() {
        // hidapi rend parfois un tampon plus long que le rapport déclaré.
        let mut padded = OFF_PUCK.to_vec();
        padded.extend_from_slice(&[0xB6, 0x7A, 0xFE, 0x00, 0x00]);
        assert_eq!(parse_power_report(&padded).unwrap().percent, 94);
    }

    #[test]
    fn the_chime_note_splits_frequency_the_way_the_hardware_expects() {
        // Le découpage se fait modulo 255, pas 256. Une note à 1174 Hz donne
        // 1174 % 255 = 154 et 1174 / 255 = 4 ; un découpage en octets aurait
        // donné 150 et 4, et la manette jouerait faux.
        let r = haptic_on(1, 1174);
        assert_eq!(r[0], RPT_HAPTIC);
        assert_eq!(r[1], 1);
        assert_eq!(r[2], LOCATOR_GAIN);
        assert_eq!(r[3], 154);
        assert_eq!(r[4], 4);
        assert_ne!(r[3], (1174 & 0xFF) as u8, "découpage en octets : note fausse");
        assert_eq!(r[5], 0xFF);
        assert_eq!(r[6], 0x7F);
    }

    #[test]
    fn the_ringtone_never_leaves_an_actuator_running() {
        // Le pire défaut possible : une manette qu'on fait sonner et qui
        // continue de vibrer indéfiniment. Chaque note allumée doit être
        // éteinte, et aucune ne doit en écraser une autre déjà en cours.
        let mut running = [false; 8];
        for &(_, actuator, freq) in RINGTONE {
            let a = actuator as usize;
            assert!(a < running.len(), "actionneur hors bornes : {actuator}");
            if freq == 0 {
                assert!(running[a], "extinction d'un actionneur déjà au repos : {actuator}");
                running[a] = false;
            } else {
                assert!(!running[a], "note posée sur un actionneur occupé : {actuator}");
                running[a] = true;
            }
        }
        assert!(running.iter().all(|r| !r), "la sonnerie laisse un actionneur en marche");
    }

    #[test]
    fn the_ringtone_is_short_enough_not_to_annoy() {
        let once: u32 = RINGTONE.iter().map(|e| e.0 as u32).sum();
        let total = (once + RINGTONE_GAP.as_millis() as u32) * RINGTONE_REPEATS as u32;
        assert!(total > 500, "sonnerie inaudible : {total} ms");
        assert!(total < 5_000, "trop long : {total} ms");
    }

    #[test]
    fn the_chime_silences_every_actuator_it_touches() {
        for a in ALL_ACTUATORS {
            let off = haptic_off(a);
            assert_eq!(off[0], RPT_HAPTIC);
            assert_eq!(off[1], a);
            assert_eq!(off[2], 0x80);
            assert_eq!(off[6], 0x80);
            // Aucune fréquence résiduelle : la note doit vraiment s'arrêter.
            assert_eq!(off[3], 0);
            assert_eq!(off[4], 0);
        }
    }

    #[test]
    fn a_docked_controller_is_not_reported_as_absent() {
        // Les trois situations doivent rester distinctes jusque dans l'infobulle.
        assert_ne!(
            ProbeError::ControllerDocked.tooltip(),
            ProbeError::ControllerOffline.tooltip()
        );
        assert!(ProbeError::ControllerDocked.tooltip().contains("socle"));
    }

    /// Fait réellement sonner la manette. À lancer manette allumée :
    /// `cargo test -- --ignored plays_the_ringtone`
    #[test]
    #[ignore = "fait sonner une manette allumée"]
    fn plays_the_ringtone_on_real_hardware() {
        match play_locator_chime() {
            Ok(()) => println!("sonnerie jouée"),
            Err(e) => panic!("impossible de jouer : {e:?} — {}", e.tooltip()),
        }
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
