//! Orchestration : ce que l'application retient d'un relevé à l'autre.
//!
//! Sans état Win32 ni entrée-sortie, pour que toute la logique de décision se
//! teste directement.

use crate::hid::{BatteryStatus, ProbeError};

/// Seuils de notification, du plus haut au plus bas.
pub const ALERT_LEVELS: [u8; 2] = [20, 10];

/// Marge de réarmement. Un niveau qui oscille autour d'un seuil ne doit pas
/// produire une notification à chaque oscillation.
const REARM_MARGIN: u8 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub level: u8,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Default)]
pub struct App {
    /// Dernier relevé réussi, conservé pour l'afficher pendant une panne passagère.
    last_good: Option<BatteryStatus>,
    /// Cause du dernier échec, pour l'infobulle.
    last_error: Option<ProbeError>,
    /// Seuils déjà signalés durant la décharge en cours.
    fired: [bool; ALERT_LEVELS.len()],
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ce que l'icône doit représenter : le dernier état connu, ou rien.
    pub fn display(&self) -> Option<&BatteryStatus> {
        self.last_good.as_ref()
    }

    /// Intègre un relevé et rend la notification à émettre, s'il y a lieu.
    pub fn ingest(&mut self, reading: Result<BatteryStatus, ProbeError>) -> Option<Alert> {
        match reading {
            Ok(s) => {
                self.last_error = None;
                self.last_good = Some(s);
                self.check_thresholds(&s)
            }
            Err(e) => {
                // Un périphérique qui disparaît invalide l'affichage ; une
                // simple indisponibilité passagère laisse la dernière valeur.
                if matches!(e, ProbeError::NoDevice | ProbeError::ControllerOffline) {
                    self.last_good = None;
                    self.fired = [false; ALERT_LEVELS.len()];
                }
                self.last_error = Some(e);
                None
            }
        }
    }

    fn check_thresholds(&mut self, s: &BatteryStatus) -> Option<Alert> {
        // En charge, on ne harcèle personne, et on réarme tout.
        if s.charging {
            self.fired = [false; ALERT_LEVELS.len()];
            return None;
        }

        let mut alert = None;
        for (i, &level) in ALERT_LEVELS.iter().enumerate() {
            if s.percent > level.saturating_add(REARM_MARGIN) {
                self.fired[i] = false;
            } else if s.percent <= level && !self.fired[i] {
                self.fired[i] = true;
                // Les seuils sont ordonnés du plus haut au plus bas ; le
                // dernier franchi est le plus urgent, il l'emporte.
                alert = Some(Alert {
                    level,
                    title: "Batterie de la manette faible".into(),
                    body: format!("Il reste {} % de charge.", s.percent),
                });
            }
        }
        alert
    }

    /// Texte de l'infobulle. Windows tronque au-delà de 127 caractères.
    pub fn tooltip(&self) -> String {
        let text = match (&self.last_good, &self.last_error) {
            (Some(s), _) => {
                let mut t = if s.charging {
                    format!("Manette Steam — {} %, en charge", s.percent)
                } else {
                    format!("Manette Steam — {} %", s.percent)
                };
                if let Some(mv) = s.voltage_mv {
                    t.push_str(&format!(" ({},{:02} V)", mv / 1000, (mv % 1000) / 10));
                }
                t
            }
            (None, Some(e)) => e.tooltip(),
            (None, None) => "Manette Steam — lecture en cours".into(),
        };
        text.chars().take(127).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(percent: u8) -> Result<BatteryStatus, ProbeError> {
        Ok(BatteryStatus { percent, voltage_mv: None, charging: false })
    }

    fn charging_at(percent: u8) -> Result<BatteryStatus, ProbeError> {
        Ok(BatteryStatus { percent, voltage_mv: None, charging: true })
    }

    #[test]
    fn alerts_once_per_threshold_on_the_way_down() {
        let mut app = App::new();
        assert!(app.ingest(at(60)).is_none());
        assert!(app.ingest(at(25)).is_none());
        assert_eq!(app.ingest(at(20)).unwrap().level, 20);
        // Toujours sous le seuil : on ne répète pas.
        assert!(app.ingest(at(18)).is_none());
        assert!(app.ingest(at(15)).is_none());
        assert_eq!(app.ingest(at(10)).unwrap().level, 10);
        assert!(app.ingest(at(9)).is_none());
        assert!(app.ingest(at(2)).is_none());
    }

    #[test]
    fn oscillation_around_a_threshold_does_not_spam() {
        let mut app = App::new();
        app.ingest(at(50));
        assert!(app.ingest(at(20)).is_some());
        for _ in 0..10 {
            assert!(app.ingest(at(21)).is_none(), "réarmement trop précoce");
            assert!(app.ingest(at(20)).is_none(), "répétition sur oscillation");
        }
    }

    #[test]
    fn rearms_after_a_real_recharge() {
        let mut app = App::new();
        app.ingest(at(50));
        assert!(app.ingest(at(20)).is_some());
        app.ingest(at(80)); // rechargée
        assert!(app.ingest(at(20)).is_some(), "doit ré-alerter après recharge");
    }

    #[test]
    fn charging_suppresses_alerts_and_rearms() {
        let mut app = App::new();
        assert!(app.ingest(charging_at(5)).is_none(), "pas d'alerte en charge");
        // Débranchée à un niveau bas : l'alerte doit repartir.
        assert!(app.ingest(at(10)).is_some());
    }

    #[test]
    fn transient_failure_keeps_the_last_known_level() {
        let mut app = App::new();
        app.ingest(at(42));
        app.ingest(Err(ProbeError::Busy("occupé".into())));
        assert_eq!(app.display().map(|s| s.percent), Some(42));
        assert!(app.tooltip().contains("42"));
    }

    #[test]
    fn disconnection_clears_the_display() {
        let mut app = App::new();
        app.ingest(at(42));
        app.ingest(Err(ProbeError::NoDevice));
        assert!(app.display().is_none());
        assert!(app.tooltip().contains("dongle"), "{}", app.tooltip());
    }

    #[test]
    fn reconnecting_can_alert_again() {
        let mut app = App::new();
        app.ingest(at(15));
        assert!(app.ingest(at(10)).is_some());
        app.ingest(Err(ProbeError::NoDevice));
        // Nouvelle session : les seuils sont réarmés.
        assert!(app.ingest(at(10)).is_some());
    }

    #[test]
    fn tooltip_stays_within_the_windows_limit() {
        let mut app = App::new();
        app.ingest(Err(ProbeError::HidUnavailable("e".repeat(400))));
        assert!(app.tooltip().chars().count() <= 127);
    }

    #[test]
    fn tooltip_shows_voltage_when_available() {
        let mut app = App::new();
        app.ingest(Ok(BatteryStatus { percent: 80, voltage_mv: Some(4000), charging: false }));
        let t = app.tooltip();
        assert!(t.contains("80 %"), "{t}");
        assert!(t.contains("4,00 V"), "{t}");
    }

    #[test]
    fn tooltip_marks_charging() {
        let mut app = App::new();
        app.ingest(charging_at(55));
        assert!(app.tooltip().contains("en charge"));
    }
}
