# Batterie de la Steam Controller 2026

Le niveau de batterie de la manette, en permanence, dans la zone de
notification de Windows. Un binaire de 164 Ko, sans runtime, sans installateur,
et qui ne consomme rien quand la manette n'est pas là.

## Utilisation

Lancer `sc-battery.exe`. Une pastille colorée portant le pourcentage apparaît
dans la zone de notification.

- **Survol** — pourcentage, tension, état de charge.
- **Clic gauche** — relève immédiatement, sans attendre le prochain cycle.
- **Clic droit** — démarrage automatique avec Windows, et quitter.

Une notification arrive à 20 % puis à 10 %, une seule fois par décharge. Un
éclair se superpose à l'icône pendant la charge.

## La consommation

C'est la contrainte qui a dicté toute l'architecture. Le processus est une
fenêtre cachée bloquée dans `GetMessage` : tant que rien n'arrive, il n'est pas
ordonnancé du tout.

| Situation | Ce qui tourne | Coût |
|---|---|---|
| Rien de branché | Rien. Aucun minuteur, aucun descripteur HID ouvert. | Zéro réveil |
| Dongle branché | Un relevé toutes les 30 s | 0,10 % d'un cœur |
| Manette connectée | Un relevé toutes les 30 s | 0,10 % d'un cœur |

Mesuré sur 90 secondes, manette connectée : 94 ms de processeur pour trois
relevés, soit une trentaine de millisecondes chacun. L'essentiel de ce temps
part dans l'énumération HID, que `hidapi` refait à chaque appel. La mémoire
privée tient en 2,2 Mo ; les 12 Mo affichés par le gestionnaire des tâches
comptent les DLL système partagées avec le reste de la machine.

Le retour à l'état dormant est déclenché par `WM_DEVICECHANGE`, que Windows
diffuse à toute fenêtre de premier niveau — aucun abonnement à maintenir.

Une nuance assumée : tant que le dongle reste branché, le minuteur continue de
tourner même manette éteinte. Allumer une manette déjà appairée ne produit
aucun événement de périphérique, puisque le dongle, lui, n'a pas bougé ; sans
ce relevé périodique, on ne verrait jamais la manette revenir.

La lecture ouvre le périphérique, attend le rapport d'alimentation, et referme.
Garder le descripteur ouvert obligerait à drainer un flux de rapports d'entrée
à 270 Hz en permanence, pour une donnée qui bouge toutes les vingt minutes.

Ce rapport n'arrivant que toutes les trois secondes et demie environ, un relevé
peut bloquer plusieurs secondes. Il se fait donc sur un fil éphémère qui poste
son résultat à la fenêtre : le menu contextuel reste réactif pendant ce temps.

## Le protocole

Rien de tout cela n'est documenté par Valve ; l'ensemble a été établi en
sondant le matériel. Le détail vit dans l'en-tête de [`src/hid.rs`](src/hid.rs).

Le dongle (PID `0x1304`) expose plusieurs interfaces HID en `usage_page`
`0xFF00`. Celle d'`usage` `0x0002` est l'interface de contrôle ; les autres,
d'`usage` `0x0001`, sont les emplacements d'appairage — une seule émet, celle
où la manette est réellement connectée.

Les rapports d'entrée sont numérotés. Deux comptent :

| identifiant | débit | contenu |
|---|---|---|
| `0x42` | ~270 Hz | boutons, axes, trackpads, gyroscope |
| `0x43` | ~0,3 Hz | **état d'alimentation** |

Le rapport `0x43` fait quinze octets :

```text
[0] 0x43        identifiant du rapport
[1] état        0x01 sur batterie, 0x04 en charge
[2] pourcentage 0 à 100
[3] tension     16 bits petit-boutiste, en millivolts
[5] ...         seconde tension, rôle non établi
```

Le format a été établi en confrontant des relevés à des états connus. Sur le
puck en charge, l'octet [2] valait `100` et l'octet [1] valait `0x04`. Hors du
puck, [2] est tombé à `94` — exactement la valeur rapportée par ailleurs — et
[1] à `0x01`.

### Fausses pistes, pour mémoire

L'attribut `0x0B`, rendu par la commande `0x83` sur le canal de commande, vaut
`4000` quel que soit l'état réel de la batterie : à 100 % en charge comme à
94 % sur batterie, et sans varier d'un millivolt sur quatre minutes
d'observation. C'est une constante de conception, probablement la tension
nominale de la cellule. Elle ressemble suffisamment à une mesure pour avoir
coûté une hypothèse entière.

Écartés de même : les registres lus par la commande `0x89`, qui sont de la
configuration, et le rapport `0x7B`, qui porte de la télémétrie radio.

## Construction

```bash
cargo build --release      # target/release/sc-battery.exe
cargo test                 # série complète, sans matériel
cargo test -- --ignored    # vérification sur manette réelle
```

Deux dépendances : `hidapi` pour le dialogue avec la manette, `windows-sys`
pour Win32. Ni framework graphique, ni runtime.

## Structure

| Fichier | Rôle |
|---|---|
| [`src/hid.rs`](src/hid.rs) | Protocole et décodage. Ne connaît pas Win32. |
| [`src/icon.rs`](src/icon.rs) | Dessin GDI de l'icône. Fonctions pures. |
| [`src/state.rs`](src/state.rs) | Machine à états, seuils de notification. Sans entrée-sortie. |
| [`src/tray.rs`](src/tray.rs) | `Shell_NotifyIcon`, menu, ballons. |
| [`src/autostart.rs`](src/autostart.rs) | Clé `Run` de l'utilisateur courant. |
| [`src/main.rs`](src/main.rs) | Fenêtre cachée, minuteurs, boucle de messages. |

## Limites

- Liaison par dongle 2,4 GHz et USB seulement. Le Bluetooth emploie un autre
  format de rapport et n'est pas géré.
- Manette Steam Controller 2026 uniquement.
- Windows uniquement.

## Licence

MIT.
