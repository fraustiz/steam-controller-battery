# Steam Controller 2026 — indicateur de batterie dans la zone de notification

Date : 2026-08-11
Statut : design validé, sonde protocole à faire

## Objectif

Une application Windows qui affiche en permanence le niveau de batterie de la Steam
Controller 2026 dans la zone de notification (system tray), et qui ne consomme
strictement rien quand la manette n'est pas connectée.

Deux contraintes dominent le design, dans cet ordre :

1. **Consommation nulle au repos.** Manette absente signifie zéro timer, zéro handle
   HID ouvert, zéro réveil du processus. Le processus reste résident mais endormi.
2. **Empreinte minimale.** Binaire unique, pas de runtime à installer, quelques Mo de
   RAM.

Tout le reste — l'apparence de l'icône, les notifications, le démarrage automatique —
est subordonné à ces deux points.

## Ce qui est déjà connu du matériel

Le projet voisin `shs-studio` (lecteur MIDI haptique en Rust pour cette même manette)
établit des faits vérifiés sur ce PC :

| Fait | Valeur |
|---|---|
| Vendor ID Valve | `0x28DE` |
| Manette en USB direct | PID `0x1302` |
| Dongle sans fil (« puck ») | PID `0x1304` |
| Interface de contrôle | `usage_page == 0xFF00` |
| Crate HID fonctionnelle sous Windows | `hidapi` 2.6 |
| Longueur des rapports | 64 octets |

Le dongle est actuellement branché et énumère les interfaces `MI_02` à `MI_06` sous
`VID_28DE&PID_1304`. Une interface réelle se distingue d'une interface fantôme du
dongle par le fait qu'elle émet : une lecture avec timeout de 100 ms renvoie des
octets.

### L'inconnue

Le format du paquet de statut batterie du modèle 2026 n'est documenté nulle part.

Le driver noyau Linux `hid-steam.c` décrit celui des modèles précédents :

```
offset  type    signification
0-1     u8[2]   en-tête de protocole (0x01, 0x00)
2       u8      type de message (0x04 = ID_CONTROLLER_STATUS)
3       u8      longueur de charge utile
4-7     u32     numéro de séquence
8-11    u8[4]   toujours 0
12-13   s16     tension en millivolts (little-endian)
14      u8      pourcentage de charge (0-100)
```

La commande `0xB4` (`ID_DONGLE_GET_WIRELESS_STATE`) demande au dongle d'émettre ce
paquet. Les commandes se transmettent en rapport de fonctionnalité (feature report),
identifiant de rapport `0`, tampon de 64 octets, la commande occupant le premier
octet utile.

Rien ne garantit que le modèle 2026 reprenne ce format : le ticket
`ValveSoftware/steam-for-linux#13308` confirme que cette manette n'expose aucune
entrée `/sys/class/power_supply`, contrairement à une DualSense. Le format doit donc
être établi par observation avant d'écrire l'application.

## Architecture

Cinq modules, chacun compréhensible et testable sans les autres.

### `hid.rs` — acquisition

Expose une seule fonction utile et un seul type :

```rust
pub struct BatteryStatus {
    pub percent: u8,
    pub voltage_mv: u16,
    pub charging: bool,
}

pub fn probe() -> Result<BatteryStatus, ProbeError>;
```

`probe()` ouvre l'interface de contrôle, envoie la commande de demande de statut, lit
les rapports entrants jusqu'à en trouver un de type statut (ou jusqu'à expiration
d'un délai de 200 ms), le décode, et referme le device. L'ouverture et la fermeture à
chaque appel sont délibérées : garder le handle ouvert obligerait à drainer un flux
de rapports d'entrée à ~100 Hz, ce qui coûterait du CPU en permanence pour une
donnée qui change toutes les vingt minutes.

Le décodage est isolé dans une fonction pure `parse_status(&[u8]) -> Option<BatteryStatus>`
pour être testable sur des vecteurs d'octets figés, sans matériel.

Ce module ne connaît ni Win32 ni la zone de notification.

### `icon.rs` — rendu

```rust
pub fn render(status: Option<BatteryStatus>, size: u32) -> HICON;
```

Dessin GDI d'une icône carrée : le pourcentage en gros chiffres, sur un fond dont la
teinte va du vert à l'orange puis au rouge selon le niveau. Un éclair se superpose
quand `charging` est vrai. `None` produit une icône grisée portant « ? ».

`size` vient de la mise à l'échelle du système (96 dpi donne 16 px, 144 dpi donne
24 px), pour que l'icône reste nette sur un écran à forte densité.

Le mapping niveau → couleur est une fonction pure, testée séparément du dessin.

### `tray.rs` — présentation

Crée une fenêtre cachée (`message-only window`), y attache l'icône via
`Shell_NotifyIcon`, et traite :

- les clics droit → menu contextuel (démarrage automatique, quitter) ;
- `WM_DEVICECHANGE` → transitions d'état ;
- `WM_TIMER` → rafraîchissement ;
- `WM_DPICHANGED` → régénération de l'icône à la bonne taille.

Les notifications de batterie faible passent par le ballon natif du tray (`NIF_INFO`),
et non par un toast WinRT : le rendu est identique sous Windows 11, sans dépendance
supplémentaire ni enregistrement d'un AppUserModelID.

### `state.rs` — orchestration

Machine à états à deux positions.

**Absent** — aucun handle HID, aucun timer. Le processus est bloqué dans `GetMessage`,
donc ordonnancé zéro fois par seconde. Un abonnement `CM_Register_Notification` sur la
classe d'interface HID est le seul lien avec le monde extérieur ; c'est Windows qui
réveille le processus à l'arrivée d'un périphérique.

**Connecté** — un `SetTimer` à 30 s. Chaque tick appelle `hid::probe()` puis met à jour
l'icône. Trente secondes suffisent largement : une batterie annoncée pour 35 h ne perd
pas un point de pourcentage en moins de vingt minutes.

Le retrait du périphérique détruit le timer et ramène à l'état Absent.

Ce module tient aussi les seuils de notification : un ballon à 20 % et un à 10 %,
armés une seule fois par cycle de décharge et réarmés dès que le niveau remonte.

### `autostart.rs` — persistance

Lecture et écriture de la valeur `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
Deux fonctions, `is_enabled()` et `set_enabled(bool)`, appelées par l'élément à cocher
du menu contextuel.

## Flux de données

```
Windows (WM_DEVICECHANGE)  ─┐
                            ├─→ state.rs ─→ hid::probe() ─→ BatteryStatus
Timer 30 s (WM_TIMER)      ─┘                  │
                                               ↓
                                   icon::render() ─→ HICON
                                               ↓
                                   Shell_NotifyIcon (icône + infobulle)
                                               │
                                    seuil franchi ? ─→ ballon
```

## Gestion des erreurs

Aucune condition d'erreur ne doit produire de fenêtre de dialogue ni terminer le
processus. Une application de zone de notification qui interrompt l'utilisateur pour
signaler qu'elle n'a pas pu lire un octet est une nuisance.

| Situation | Comportement |
|---|---|
| Aucun périphérique Valve énuméré | État Absent, icône grisée |
| Périphérique présent, ouverture refusée (accaparé) | Icône grisée, infobulle explicite, nouvel essai au tick suivant |
| Dongle présent mais manette éteinte | Icône grisée « ? » |
| Rapport reçu mais non décodable | Conservation de la dernière valeur connue, infobulle datée |
| `hidapi` indisponible | Icône grisée, infobulle avec le message d'erreur |

## Tests

Testable sans matériel, donc automatisé :

- `parse_status` sur des vecteurs d'octets figés capturés par la sonde, y compris des
  cas dégradés (rapport tronqué, type de message inattendu, pourcentage hors bornes) ;
- le mapping niveau → couleur, aux bornes et aux points de bascule ;
- la logique de seuil de notification : déclenchement unique à la descente,
  réarmement à la remontée.

Vérifié à l'œil, car dépendant de l'environnement graphique : le rendu GDI de l'icône,
le menu contextuel, l'apparence du ballon, la transition d'état au débranchement
physique du dongle.

## Étape préalable : sonde du protocole

Avant d'écrire l'application, un binaire jetable établit le format réel :

1. énumérer les interfaces `0x28DE` et retenir celles en `usage_page 0xFF00` qui
   émettent ;
2. envoyer les commandes candidates (`0xB4`, `0xA1`) en rapport de fonctionnalité ;
3. dumper en hexadécimal les rapports entrants, groupés par type de message ;
4. répéter à deux niveaux de charge différents pour identifier l'octet qui suit la
   décharge.

La sortie de cette sonde fixe les constantes de `hid.rs` et fournit les vecteurs de
test de `parse_status`. Elle ne fait pas partie du livrable.

## Hors périmètre

Volontairement écartés pour tenir les deux contraintes de tête :

- la prise en charge du Bluetooth, dont le format de rapport diffère — à ajouter si le
  besoin apparaît ;
- les manettes autres que la Steam Controller 2026 ;
- l'historique de décharge, les graphiques, la fenêtre de préférences ;
- toute forme de télémétrie ou d'accès réseau.
