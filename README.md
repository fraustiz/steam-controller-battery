# Batterie de la Steam Controller 2026

Le niveau de batterie de la manette, en permanence, dans la zone de
notification de Windows. Un binaire de 187 Ko, sans runtime, sans installateur,
et qui ne consomme rien quand la manette n'est pas là.

## Utilisation

Lancer `sc-battery.exe`. Une batterie verticale apparaît dans la zone de
notification : le remplissage suit le niveau, du vert au rouge en passant par
l'ambre. Le contour prend le ton qui contraste avec la barre des tâches, clair
ou sombre, et suit vos changements de thème.

L'icône a trois visages :

| Situation | Icône |
|---|---|
| Manette connectée | Batterie remplie et colorée selon le niveau, éclair si elle charge |
| Éteinte **sur son socle** | Batterie atténuée avec éclair : en charge, niveau inconnu |
| Rien de connecté | Une prise |

Une batterie vide se lirait « 0 % » : c'est pourquoi l'absence de mesure ne se
dessine jamais ainsi. Et l'état « sur le socle » est atténué plutôt que plein,
faute de quoi il serait le dessin exact d'une batterie mesurée à 0 % en charge
— état parfaitement réel, qu'une manette à plat sur son socle produit.

Le clic droit propose **« Afficher le pourcentage »**. Le nombre remplace alors
entièrement la batterie, et prend sa couleur du niveau.

Ce remplacement n'est pas un choix esthétique mais une contrainte de place. Une
première version inscrivait le chiffre *dans* la batterie : le contour ne
laissait que huit pixels utiles, donc des chiffres de 3×5, à la limite du
déchiffrable. Sans cadre, les mêmes chiffres occupent 6×10 — quatre fois la
surface. Le niveau reste lisible d'un coup d'œil puisqu'il donne sa couleur au
nombre, et la charge passe dans l'infobulle.

Les chiffres sont une police bitmap dessinée à la main, alignée au pixel :
confiés à GDI, ils s'étaleraient sur deux pixels gris et redeviendraient
illisibles.

Le clic droit propose aussi **« Faire sonner la manette »** : une sonnerie jouée sur
ses actionneurs haptiques, pour la retrouver ou savoir de laquelle il s'agit.
L'entrée est grisée quand la manette est éteinte, ses actionneurs ne recevant
alors rien.

La mélodie vient d'un fichier MIDI, converti **hors ligne** en table figée par
un petit outil séparé. Embarquer un analyseur MIDI pour rejouer une seconde et
demie de musique aurait coûté une dépendance entière contre quelques centaines
d'octets de table. Le principe et les tables de fréquences viennent de
[shs-studio](https://github.com/fraustiz/steam-haptics-studio), lui-même porté
de SteamHapticsSinger.

- **Survol** — pourcentage, tension, état de charge.
- **Clic gauche** — relance la lecture si elle s'était arrêtée faute de matériel.
- **Clic droit** — démarrage automatique avec Windows, et quitter.

Une notification arrive à 20 % puis à 10 %, une seule fois par décharge. Un
éclair se découpe dans l'icône pendant la charge.

Le pourcentage exact ne figure pas dans l'icône : à seize pixels, trois
chiffres sont illisibles et alourdissent la barre. Il est dans l'infobulle.

## Mode de simulation

```bash
sc-battery.exe --debug
```

Le menu contextuel reçoit alors une section supplémentaire : niveau au choix,
charge, socle, déconnexion. Le matériel n'est plus interrogé du tout — le fil
de lecture écraserait les valeurs choisies.

Il existe parce que certains états se méritent : une batterie à 8 % ne se
provoque pas sur commande, et vérifier le rendu sur barre des tâches claire
suppose de changer le thème du système.

L'infobulle est préfixée de `[simulation]`. Sans cette mention, on finirait par
prendre une valeur inventée pour une mesure — l'erreur exacte que ce mode
sert à débusquer.

## La consommation

C'est la contrainte qui a dicté toute l'architecture. Le processus est une
fenêtre cachée bloquée dans `GetMessage` : tant que rien n'arrive, il n'est pas
ordonnancé du tout.

| Situation | Ce qui tourne | Coût |
|---|---|---|
| Rien de branché | Rien. Le fil de lecture s'est terminé. | Zéro réveil |
| Dongle branché, manette éteinte | Une tentative toutes les 5 s | négligeable |
| Manette connectée | Un fil à l'écoute du flux | 0,31 % d'un cœur |

La mémoire privée tient en 2,2 Mo ; les 12 Mo affichés par le gestionnaire des
tâches comptent les DLL système partagées avec le reste de la machine.

Le retour à l'état dormant est déclenché par `WM_DEVICECHANGE`, que Windows
diffuse à toute fenêtre de premier niveau — aucun abonnement à maintenir.

Une nuance assumée : tant que le dongle reste branché, le fil de lecture
retente toutes les cinq secondes, même manette éteinte. Allumer une manette
déjà appairée ne produit aucun événement de périphérique, puisque le dongle,
lui, n'a pas bougé ; sans cette tentative répétée, on ne la verrait jamais
revenir.

### Le compromis de la réactivité

La première version interrogeait la manette toutes les trente secondes. Poser
la manette sur son socle mettait donc jusqu'à une demi-minute à se voir, ce qui
est trop long pour un geste dont on attend un retour immédiat.

Or la manette émet son état d'alimentation d'elle-même, toutes les trois
secondes et demie. Un fil reste donc à l'écoute et transmet chaque rapport : la
charge apparaît en quelques secondes, et plus aucun minuteur ne tourne.

Le prix en est de traverser le flux d'entrée à 270 Hz, qui arrive sur la même
interface. Traverser ne veut pas dire traiter : chaque rapport coûte une
comparaison d'octet, et le noyau les reçoit de toute façon, que nous les
lisions ou non.

Ce prix se mesure : **0,31 % d'un cœur**, contre 0,035 % pour le sondage
périodique. Neuf fois plus, donc, pour passer d'une demi-minute à quelques
secondes de latence. En valeur absolue cela reste 0,3 % d'un seul cœur, et
uniquement tant qu'une manette est connectée — mais c'est un arbitrage, pas un
repas gratuit. Qui préférerait l'inverse n'a qu'à remplacer l'écoute par un
appel périodique à `probe()` : `run_reader` est la seule fonction concernée.

L'infobulle suit chaque relevé, mais l'icône n'est reconstruite que lorsque son
dessin change réellement — sans quoi on fabriquerait une icône toutes les trois
secondes et demie pour un résultat identique.

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
[0] 0x43         identifiant du rapport
[1] état         0x01 décharge, 0x02 en charge, 0x04 chargée
[2] pourcentage  0 à 100
[3] tension      cellule, 16 bits petit-boutiste, en millivolts
[5] tension      seconde valeur, rôle non établi
[7] alimentation 16 bits petit-boutiste, en millivolts ; nulle hors secteur
[9] courant      16 bits petit-boutiste, en milliampères ; nul à pleine charge
```

Les trois états, relevés sur le matériel :

| Situation | [1] | [7..9] | [9..11] |
|---|---|---|---|
| Hors du puck, 94 % | `0x01` | 0 | 0 |
| En charge, 96 % | `0x02` | 4800 mV | 175 mA |
| Sur le puck, 100 % | `0x04` | 4860 mV | 0 mA |

### L'octet d'état a menti une fois

L'octet [1] a d'abord été lu de travers. Un unique relevé sur le puck le
montrait à `0x04`, d'où la conclusion « `0x04` signifie en charge ». La
batterie y était en réalité déjà pleine : `0x04` veut dire *chargée*, et c'est
`0x02` qui signale une charge en cours. L'indicateur ne s'allumait donc jamais,
et rien dans les tests ne pouvait le révéler — ils rejouaient fidèlement le
relevé mal interprété.

C'est de là que vient le choix de ne pas faire reposer la détection sur le seul
octet d'état. Les octets [7] et [9] sont des mesures — tension d'alimentation
et courant de charge — et une mesure ment moins qu'un code dont on n'a pas
observé toutes les valeurs.

### Détecter une manette éteinte sur son socle

Le routage du canal `0x01` s'inverse selon l'état de la manette, et c'est ce qui
rend la détection possible :

| Manette | emplacement (`usage 0x0001`) | contrôle (`usage 0x0002`) |
|---|---|---|
| allumée | répond | refusé |
| éteinte, **sur le socle** | refusé | **répond** |
| éteinte, à côté du PC hors socle | refusé | refusé |
| éteinte, éloignée | refusé | refusé |

Les deux derniers cas sont ce qui donne sa valeur au test. Une manette éteinte
posée à trente centimètres du dongle reste muette : ce n'est donc ni un
appairage mémorisé, ni de la portée radio. Seul le contact du socle — celui-là
même qui la recharge — ouvre le canal. Le signal est par conséquent exact :
il dit « sur le socle », pas « quelque part à proximité ».

Le niveau de charge, lui, reste hors d'atteinte : il ne circule que dans les
rapports d'entrée, qu'une manette éteinte n'émet pas.

### Fausses pistes, pour mémoire

L'attribut `0x0B` ne porte pas le niveau, mais il a fallu deux erreurs pour
l'établir.

Il valait `4000` quel que soit l'état réel de la batterie, sans varier d'un
millivolt sur quatre minutes. J'en ai conclu à une constante de conception. Ces
quatre minutes se passaient toutes **manette éveillée** : interrogé éteinte sur
son socle, il vaut `64000`.

Une surveillance de trente minutes pendant une charge a tranché : il ne prend
que ces deux valeurs, bascule avec l'état, et ne progresse jamais. Ce n'est donc
ni une jauge, ni une constante — plutôt un champ dont le cadrage change selon
l'état, ce que suggère l'écriture hexadécimale : `0x0FA0` contre `0xFA00`, les
mêmes chiffres décalés d'un quartet.

La leçon vaut plus que le détail : conclure « c'est constant » depuis des
relevés pris dans un seul état, c'est la même faute que conclure « 0x04 signifie
en charge » depuis un unique échantillon.

Écartés de même : les registres lus par la commande `0x89`, qui sont de la
configuration, et le rapport `0x7B`, qui porte de la télémétrie radio.

## Construction

```bash
cargo build --release      # target/release/sc-battery.exe
cargo test                 # série complète, sans matériel
cargo test -- --ignored    # relevé sur manette réelle, et planche de contrôle de l'icône
```

Deux dépendances : `hidapi` pour le dialogue avec la manette, `windows-sys`
pour Win32. Ni framework graphique, ni runtime.

## Structure

| Fichier | Rôle |
|---|---|
| [`src/hid.rs`](src/hid.rs) | Protocole et décodage. Ne connaît pas Win32. |
| [`src/icon.rs`](src/icon.rs) | Dessin de l'icône, pixel par pixel. Fonctions pures. |
| [`src/state.rs`](src/state.rs) | Machine à états, seuils de notification. Sans entrée-sortie. |
| [`src/tray.rs`](src/tray.rs) | `Shell_NotifyIcon`, menu, ballons. |
| [`src/autostart.rs`](src/autostart.rs) | Clé `Run` de l'utilisateur courant. |
| [`src/settings.rs`](src/settings.rs) | Préférences, dans notre propre branche du registre. |
| [`src/icons.rs`](src/icons.rs) | Masques des icônes, engendrés hors ligne. Ne pas modifier à la main. |
| [`src/debug.rs`](src/debug.rs) | Mode de simulation, derrière `--debug`. |
| [`src/main.rs`](src/main.rs) | Fenêtre cachée, fil de lecture, boucle de messages. |

## Limites

- Liaison par dongle 2,4 GHz et USB seulement. Le Bluetooth emploie un autre
  format de rapport et n'est pas géré.
- Manette Steam Controller 2026 uniquement.
- Windows uniquement.

## Licence

MIT.
