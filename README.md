# My Magic Adventure

> *Un RPG voxel d'aventure et de magie, inspiré de Minecraft — choisis ta race, maîtrise les éléments, explore un monde infini.*

**Alpha v0.1** · Rust + Bevy 0.15

![Aperçu](assets/blocks_preview.png)

---

## Le pitch

Cinq races, six éléments de magie, un monde voxel infini. Tu incarnes un humanoïde né dans un monde sculpté par les forces élémentaires. Selon ta race, ton rapport à la magie, à la nuit, au sol ou à l'air change. À toi de trouver ton style — bâtisseur patient, mage offensif, ombre silencieuse…

## Fonctionnalités de l'Alpha

### Les 5 races jouables
Chaque race apporte un passif permanent, une compétence active dédiée (touche **F**) et une affinité élémentaire qui influence le gameplay.

| Race | Affinités | Style |
|------|-----------|-------|
| **Sylvaris** — les Éveillés de la Forêt | Terre & Eau | Régénération près de la verdure, enracinement des ennemis |
| **Ignaar** — les Forgés du Magma | Feu & Terre | Immunité à la lave, éruption volcanique |
| **Aethyn** — les Tisseurs du Vent | Air & Foudre | Vitesse accrue, dash aérien, chutes ralenties |
| **Vorkai** — les Écailles du Crépuscule | Ombre & Feu | Vision nocturne, voile d'invisibilité |
| **Crysthari** — les Enfants du Prisme | Lumière & Eau | Pool de mana étendu, rayon prismatique |

### 12 sorts répartis sur 6 éléments
Boule de Feu, Éclat de Glace, Mur de Terre, Dash du Vent, Soin Lumière, Éclat Aveuglant, Voile d'Ombre, Drain d'Ombre, Bouclier d'Eau, Nova de Feu, Lame de Vent, Pic de Pierre.

### Monde & gameplay
- Génération procédurale infinie (chunks 16×16×256, biomes plaine/forêt/montagne)
- Destruction et placement de blocs au raycast
- Inventaire style Minecraft avec hotbar et drag & drop
- Crafting de blocs et d'outils
- Combat mêlée + magie contre mobs passifs (Vache, Poule, Cochon) et hostiles (Slime, Loup, Humanoïdes)
- Cycle jour/nuit dynamique
- Sauvegarde / chargement multi-slot
- Écran de mort / respawn

## Installation

### Pré-requis
- [Rust](https://www.rust-lang.org/tools/install) (edition 2024)
- Une carte graphique compatible Vulkan / DX12 / Metal

### Build et lancement

```bash
git clone https://github.com/Djibzi/my-magic-adventure.git
cd my-magic-adventure
cargo run --release
```

La première compilation est longue (Bevy), ensuite c'est rapide.

## Contrôles

| Touche | Action |
|--------|--------|
| **ZQSD** (AZERTY) / WASD | Déplacement |
| **Espace** | Saut |
| **Souris** | Regarder |
| **Clic gauche** | Casser un bloc / frapper |
| **Clic droit** | Poser un bloc |
| **1–9** | Sélection hotbar |
| **Z / X** | Lancer un sort équipé |
| **R** | Changer de sort |
| **F** | Compétence raciale |
| **E** | Ouvrir / fermer l'inventaire |
| **F2** | Sauvegarder |
| **F3** | Infos debug |
| **F5** | Respawn forcé |
| **Échap** | Pause / menus |

## Stack technique

- **Langage** : Rust (edition 2024)
- **Moteur** : [Bevy 0.15](https://bevyengine.org/) (ECS)
- **Génération** : `noise` (Perlin / Simplex)
- **Aléatoire** : `rand`
- **Rendu** : greedy meshing + LOD super-chunks (4×4 blocs fusionnés à distance)
- **Tâches async** : `AsyncComputeTaskPool` pour la génération de chunks

## Performance

L'Alpha tourne à **25–30 FPS** sur un PC modeste. Des optimisations (LOD plus agressif, culling, instanciation GPU) sont prévues pour la v0.2.

## État du projet

Alpha jouable : 5 races + 12 sorts + monde infini + combat + crafting + sauvegarde fonctionnels. Les fondations, l'interaction avec le monde et le système de magie/combat sont en place. Polish final (audio, tutorial) reporté à la v0.2.

## Auteur

**Djibzi** — conception, code, direction artistique.

## Licence

Projet personnel. Tous droits réservés pour le moment — une licence sera ajoutée à la sortie Beta.
