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

La lecture ouvre le périphérique, pose sa question et le referme. Garder le
descripteur ouvert obligerait à drainer un flux de rapports d'entrée à 270 Hz,
ce qui coûterait du processeur en permanence pour une donnée qui bouge toutes
les vingt minutes.

## Le protocole

Rien de tout cela n'est documenté par Valve ; l'ensemble a été établi en
sondant le matériel. Le détail vit dans l'en-tête de [`src/hid.rs`](src/hid.rs).

Le dongle (PID `0x1304`) expose plusieurs interfaces HID en `usage_page`
`0xFF00`. Celle d'`usage` `0x0002` est l'interface de contrôle. Les commandes
passent par des *feature reports numérotés* de 64 octets, dont l'identifiant
choisit le destinataire : `0x01` la manette (PID `0x1302`), `0x02` le dongle.

```text
[id_rapport, commande, longueur_args, args...]
```

La commande `0x83` rend les attributs sous forme de triplets
`(identifiant, valeur sur 32 bits)`. L'attribut `0x0B` porte la mesure de
batterie, et n'apparaît que chez la manette — jamais chez le dongle, qui n'a
pas de batterie.

### Ce qui reste incertain

L'attribut `0x0B` vaut `4000` et n'a pas varié d'un millivolt sur quatre
minutes d'observation. C'est cohérent avec une tension de cellule Li-ion au
repos, convertie ici en pourcentage par une courbe de décharge de référence
(4000 mV donnent 80 %). Mais la confirmation définitive demande de voir la
valeur monter pendant une charge — un test qui n'a pas encore pu être fait.

Si `0x0B` se révélait être une constante et non une mesure, seule la fonction
`status_from_attributes` serait à revoir : le reste de l'application ne connaît
rien du format.

Les autres pistes ont été écartées par l'observation : les rapports d'entrée
`0x43` et `0x7B` portent de la télémétrie radio (puissance de réception,
pertes), et les registres lus par la commande `0x89` sont de la configuration.

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
