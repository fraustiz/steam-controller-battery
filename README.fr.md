# Batterie de la Steam Controller 2026

*[English version](README.md)*

Le niveau de batterie de la Steam Controller 2026, en permanence, dans la zone
de notification de Windows. Un binaire unique d'environ 250 Ko — sans runtime,
sans installateur — qui ne coûte rigoureusement rien quand la manette n'est pas
là.

Rust natif, deux dépendances : `hidapi` pour dialoguer avec la manette,
`windows-sys` pour Win32. Aucun framework graphique.

## Ce que dit l'icône

| Situation | Icône |
|---|---|
| Manette connectée | Batterie remplie et colorée selon le niveau, en huit paliers |
| En charge | Le même niveau, plus un éclair |
| **Éteinte sur son socle** | Un cadre marqué d'un point d'interrogation |
| Rien de connecté | Une prise barrée |

Le socle mérite sa propre icône parce que le niveau y est réellement inconnu :
une manette éteinte n'émet rien. Dessiner une batterie vide se lirait « 0 % »,
ce qui est une tout autre affirmation.

- **Survol** — niveau, tension de la cellule, état de charge.
- **Clic gauche** — relance la lecture si elle s'était arrêtée faute de matériel.
- **Clic droit** — le menu ci-dessous.

Un ballon prévient à 20 % puis à 10 %, une seule fois par décharge.

## Menu

**Faire sonner la manette** joue une courte mélodie sur les actionneurs
haptiques, pour la retrouver dans une pièce ou savoir de laquelle il s'agit.
L'entrée est grisée tant que la manette est éteinte : ses actionneurs ne
reçoivent alors rien.

La mélodie vient d'un fichier MIDI, converti **hors ligne** en table figée par
un outil séparé. Embarquer un analyseur MIDI pour rejouer une seconde et demie
de musique aurait coûté une dépendance entière contre quelques centaines
d'octets de table.

**Afficher le pourcentage** remplace la batterie par le nombre lui-même, coloré
selon le niveau. Ce remplacement est une contrainte, pas une préférence : dans
un cadre de batterie, seize pixels ne laissent la place qu'à des chiffres de
3×5, à la limite du déchiffrable. Sans cadre, les mêmes chiffres occupent 6×10
— quatre fois la surface. Les chiffres sont une police bitmap dessinée à la
main ; confiés à GDI, ils s'étaleraient sur deux pixels gris et redeviendraient
illisibles.

**Démarrer avec Windows** écrit une valeur sous la clé `Run` de l'utilisateur
courant. Rien de plus : ni service, ni tâche planifiée, ni élévation.

## Langue

Anglais et français. La locale du système décide, l'anglais servant de repli.

```bash
sc-battery.exe --lang fr
```

force la langue pour cette exécution.

## Mode de simulation

```bash
sc-battery.exe --debug
```

Le menu reçoit alors un niveau au choix, la charge, le socle et la déconnexion.
Le matériel n'est plus interrogé du tout : le fil de lecture écraserait les
valeurs choisies.

Il existe parce que certains états se méritent : une batterie à 8 % ne se
provoque pas sur commande, et vérifier le rendu sur une barre des tâches claire
suppose de changer le thème du système.

L'infobulle est préfixée de `[simulation]`. Sans cette mention, on finirait par
prendre une valeur inventée pour une mesure — l'erreur exacte que ce mode sert
à débusquer.

## La consommation

C'est la contrainte qui a dicté l'architecture. Le processus est une fenêtre
cachée bloquée dans `GetMessage` : tant que rien n'arrive, il n'est pas
ordonnancé du tout.

| Situation | Ce qui tourne | Coût |
|---|---|---|
| Rien de branché | Rien. Le fil de lecture s'est terminé. | Zéro réveil |
| Dongle branché, manette éteinte | Une tentative toutes les 5 s | négligeable |
| Manette connectée | Un fil à l'écoute du flux | 0,31 % d'un cœur |

La mémoire privée tient autour de 2,2 Mo ; les 12 Mo affichés par le
gestionnaire des tâches comptent les DLL système partagées avec le reste de la
machine.

Le retour à l'état dormant est déclenché par `WM_DEVICECHANGE`, que Windows
diffuse à toute fenêtre de premier niveau — aucun abonnement à maintenir.

Une nuance assumée : tant que le dongle reste branché, le fil de lecture retente
toutes les cinq secondes, même manette éteinte. Allumer une manette déjà
appairée ne produit aucun événement de périphérique, puisque le dongle, lui, n'a
pas bougé ; sans cette tentative répétée, on ne la verrait jamais revenir.

### Le compromis de la réactivité

La première version interrogeait la manette toutes les trente secondes. Poser la
manette sur son socle mettait donc jusqu'à une demi-minute à se voir, ce qui est
trop long pour un geste dont on attend un retour immédiat.

Or la manette émet son état d'alimentation d'elle-même, toutes les trois
secondes et demie. Un fil reste désormais à l'écoute et transmet chaque
rapport : la charge apparaît en quelques secondes, et plus aucun minuteur ne
tourne.

Le prix en est de traverser le flux d'entrée à 270 Hz qui arrive sur la même
interface. Traverser n'est pas traiter : chaque rapport coûte une comparaison
d'octet, et le noyau les reçoit de toute façon, que nous les lisions ou non.

Ce prix a été mesuré : **0,31 % d'un cœur**, contre 0,035 % pour un sondage
périodique. Neuf fois plus, pour passer d'une demi-minute à quelques secondes de
latence. En valeur absolue cela reste 0,3 % d'un seul cœur, et uniquement tant
qu'une manette est connectée — mais c'est un arbitrage, pas un repas gratuit.

## Le protocole

Rien de tout cela n'est documenté par Valve ; l'ensemble a été établi en sondant
le matériel. Le détail vit dans l'en-tête de [`src/hid.rs`](src/hid.rs).

Le dongle (PID `0x1304`) expose plusieurs interfaces HID en `usage_page`
`0xFF00`. Celle d'`usage` `0x0002` est l'interface de contrôle ; les autres,
d'`usage` `0x0001`, sont les emplacements d'appairage — une seule émet, celle où
une manette est effectivement connectée.

Les rapports d'entrée sont numérotés. Deux comptent :

| identifiant | débit | contenu |
|---|---|---|
| `0x42` | ~270 Hz | boutons, axes, trackpads, gyroscope |
| `0x43` | ~0,3 Hz | **état d'alimentation** |

Le rapport `0x43` fait quinze octets :

```text
[0] 0x43         identifiant du rapport
[1] état         0x01 décharge, 0x02 en charge, 0x04 chargée
[2] pourcentage  0 à 100
[3] tension      cellule, 16 bits petit-boutiste, en millivolts
[5] tension      seconde valeur, rôle non établi
[7] alimentation 16 bits petit-boutiste, en millivolts ; nulle hors secteur
[9] courant      16 bits petit-boutiste, en milliampères ; nul à pleine charge
```

Trois états, relevés sur le matériel :

| Situation | [1] | [7..9] | [9..11] |
|---|---|---|---|
| Hors du socle, 94 % | `0x01` | 0 | 0 |
| En charge, 96 % | `0x02` | 4800 mV | 175 mA |
| Sur le socle, 100 % | `0x04` | 4860 mV | 0 mA |

### Détecter une manette éteinte sur son socle

Le canal `0x01` change d'interface selon l'état de la manette, et c'est ce qui
rend la détection possible :

| Manette | emplacement (`usage 0x0001`) | contrôle (`usage 0x0002`) |
|---|---|---|
| allumée | répond | refusé |
| éteinte, **sur le socle** | refusé | **répond** |
| éteinte, à côté du PC hors socle | refusé | refusé |
| éteinte, éloignée | refusé | refusé |

Les deux derniers cas sont ce qui donne sa valeur au test. Une manette éteinte
posée à trente centimètres du dongle reste muette : ce n'est donc ni un
appairage mémorisé, ni de la simple portée radio. Seul le contact du socle —
celui-là même qui la recharge — ouvre le canal. Le signal est par conséquent
exact : il dit « sur le socle », pas « quelque part à proximité ».

Le niveau, lui, reste hors d'atteinte : il ne circule que dans les rapports
d'entrée, qu'une manette éteinte n'émet pas.

### Deux erreurs qui méritent d'être consignées

**L'octet d'état a menti une fois.** Un unique relevé pris sur le socle le
montrait à `0x04`, d'où la conclusion hâtive que `0x04` signifiait « en
charge ». La batterie y était en réalité déjà pleine : `0x04` veut dire
*chargée*, et c'est `0x02` qui signale une charge en cours. Généraliser depuis
un seul point de mesure avait produit un indicateur qui ne pouvait jamais
s'allumer.

Rien dans les tests ne pouvait le rattraper : ils rejouaient fidèlement le
relevé mal interprété. Il a fallu une observation contradictoire — tension et
niveau qui montent alors que `charging` vaut faux — pour le faire tomber.

**L'attribut `0x0B` n'est pas une jauge, et pas une constante non plus.** Il
valait `4000` sans bouger sur quatre minutes, d'où « constante de conception ».
Ces quatre minutes avaient toutes été prises manette éveillée ; interrogé
éteinte sur son socle, il vaut `64000`. Trente minutes de surveillance pendant
une charge ont tranché : il ne prend que ces deux valeurs, bascule avec l'état,
et ne progresse jamais. Son écriture hexadécimale suggère la raison — `0x0FA0`
contre `0xFA00`, les mêmes chiffres décalés d'un quartet.

La leçon vaut plus que le détail : un champ observé dans un seul état n'est pas
un champ observé.

Écartés de même : les registres lus par la commande `0x89`, qui sont de la
configuration, et le rapport `0x7B`, qui porte de la télémétrie radio.

## Les icônes

Les formes viennent de [Material Symbols](https://fonts.google.com/icons),
retouchées, puis rastérisées **hors ligne** à chacune des quatre tailles que
Windows réclame selon la densité d'écran — 16, 20, 24 et 32 pixels. Dessinées à
chaque taille plutôt que redimensionnées : réduire une grille de 24 dp vers
16 px place les traits à cheval sur deux pixels et les rend gris.

Les masques engendrés ne portent que l'opacité. La couleur est décidée à
l'affichage, ce qui permet au cadre et à l'éclair de suivre le thème du système
— un cadre clair disparaîtrait sur une barre des tâches claire — tandis que les
teintes de niveau restent fixes, puisqu'elles portent une information et non une
décoration.

## Construction

```bash
cargo build --release      # target/release/sc-battery.exe
cargo test                 # série complète, sans matériel
cargo test -- --ignored    # vérifications sur manette réelle, et planche des icônes
```

## Structure

| Fichier | Rôle |
|---|---|
| [`src/hid.rs`](src/hid.rs) | Protocole et décodage. Ne connaît pas Win32. |
| [`src/icon.rs`](src/icon.rs) | Composition de l'icône. Fonctions pures. |
| [`src/icons.rs`](src/icons.rs) | Masques des icônes, engendrés hors ligne. Ne pas modifier à la main. |
| [`src/state.rs`](src/state.rs) | Machine à états, seuils de notification. Sans entrée-sortie. |
| [`src/tray.rs`](src/tray.rs) | `Shell_NotifyIcon`, menu, ballons. |
| [`src/settings.rs`](src/settings.rs) | Préférences, dans notre propre branche du registre. |
| [`src/autostart.rs`](src/autostart.rs) | Clé `Run` de l'utilisateur courant. |
| [`src/i18n.rs`](src/i18n.rs) | Traductions. En oublier une ne compile pas. |
| [`src/debug.rs`](src/debug.rs) | Mode de simulation, derrière `--debug`. |
| [`src/main.rs`](src/main.rs) | Fenêtre cachée, fil de lecture, boucle de messages. |

## Limites

- Liaison par dongle 2,4 GHz et USB seulement. Le Bluetooth emploie un autre
  format de rapport et n'est pas géré.
- Steam Controller 2026 uniquement.
- Windows uniquement.

## Crédits et licences

Ce projet est sous licence MIT. Il incorpore des travaux tiers :

- **Les formes des icônes** dérivent de
  [Material Symbols](https://fonts.google.com/icons) de Google, sous licence
  Apache 2.0.
- **Les tables de fréquences haptiques** viennent de
  [SteamHapticsSinger](https://github.com/CrazyCritic89/SteamHapticsSinger) de
  CrazyCritic89, d'après SteamControllerSinger de Pila, sous BSD-3-Clause — par
  l'intermédiaire du projet voisin
  [shs-studio](https://github.com/fraustiz/steam-haptics-studio).
