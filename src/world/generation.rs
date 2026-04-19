//! Génération procédurale des chunks — hauteur, biomes, arbres.
//!
//! Tout part d'un `Perlin` partagé (seed fixe) pour que deux runs du jeu
//! génèrent exactement le même monde. La hauteur combine 3 octaves, les
//! biomes se choisissent sur 2 couches (température/humidité) et les arbres
//! viennent en dernier par seuillage d'un bruit haute fréquence.

use noise::{NoiseFn, Perlin};
use std::sync::OnceLock;
use super::chunk::{Block, ChunkBlocks, CHUNK_SIZE, CHUNK_HEIGHT};

pub const SEED: u32 = 42_195;

/// Instance Perlin partagée : recréer un Perlin à chaque appel coûte un
/// shuffle de 256 entrées, ce qui grimpe vite quand beaucoup de tâches de
/// génération (chunks + LOD) tournent en parallèle.
pub fn shared_perlin() -> &'static Perlin {
    static PERLIN: OnceLock<Perlin> = OnceLock::new();
    PERLIN.get_or_init(|| Perlin::new(SEED))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Biome {
    Plains,
    Forest,
    Desert,
    Tundra,
    Swamp,
}

/// Hauteur de terrain en blocs. 3 octaves pour un relief varié, clampé pour
/// laisser au moins 2 blocs d'air au-dessus (évite que les arbres sortent
/// du chunk).
pub fn terrain_height(noise: &Perlin, wx: f64, wz: f64) -> usize {
    let v = noise.get([wx / 128.0, wz / 128.0]) * 0.50
          + noise.get([wx /  64.0, wz /  64.0]) * 0.30
          + noise.get([wx /  32.0, wz /  32.0]) * 0.20;
    let normalized = (v + 1.0) * 0.5;
    ((normalized * 40.0) as usize + 6).clamp(3, CHUNK_HEIGHT - 2)
}

/// Biome d'une colonne — on tire deux bruits décalés (température / humidité)
/// et on range dans une des 5 cases suivant des seuils simples.
pub fn biome_at(noise: &Perlin, wx: f64, wz: f64) -> Biome {
    let temp  = noise.get([wx / 220.0 + 500.0, wz / 220.0 + 500.0]);
    let humid = noise.get([wx / 180.0 + 912.0, wz / 180.0 + 912.0]);
    if temp > 0.35 {
        if humid < -0.15 { Biome::Desert } else { Biome::Plains }
    } else if temp < -0.35 {
        Biome::Tundra
    } else if humid > 0.30 {
        Biome::Swamp
    } else if humid > 0.0 {
        Biome::Forest
    } else {
        Biome::Plains
    }
}

/// Bloc posé sur le dessus d'une colonne (la surface que le joueur voit).
pub fn biome_surface(biome: Biome) -> Block {
    match biome {
        Biome::Plains | Biome::Forest => Block::Grass,
        Biome::Desert => Block::Sand,
        Biome::Tundra => Block::Snow,
        Biome::Swamp  => Block::Dirt,
    }
}

/// Bloc juste sous la surface (les quelques couches avant la pierre). Sable
/// dans le désert pour éviter la bande de terre bizarre en bord de dune.
fn subsoil(biome: Biome) -> Block {
    match biome {
        Biome::Desert => Block::Sand,
        _             => Block::Dirt,
    }
}

/// Plante un petit arbre (tronc 4 blocs + canopée 3×3×2 + pointe) au sommet
/// `top_y` de la colonne locale (lx, lz). Tout ce qui dépasserait le chunk
/// est simplement coupé — les chunks voisins font leurs propres arbres, donc
/// on accepte une légère désynchro aux frontières pour l'alpha.
fn place_tree(blocks: &mut ChunkBlocks, lx: usize, lz: usize, top_y: usize, kind: Block) {
    let trunk_h = 4usize;
    for dy in 1..=trunk_h {
        let y = top_y + dy;
        if y >= CHUNK_HEIGHT { return; }
        blocks[lx][y][lz] = Block::Wood;
        let _ = kind;
    }
    // Canopée : deux étages 3×3 de feuilles autour du haut du tronc.
    let base_y = top_y + trunk_h;
    for dy in 0..=1i32 {
        for dx in -1..=1i32 {
            for dz in -1..=1i32 {
                let x = lx as i32 + dx;
                let z = lz as i32 + dz;
                let y = base_y as i32 + dy;
                if x < 0 || x >= CHUNK_SIZE as i32 { continue; }
                if z < 0 || z >= CHUNK_SIZE as i32 { continue; }
                if y < 0 || y >= CHUNK_HEIGHT as i32 { continue; }
                // On n'écrase pas le tronc.
                if blocks[x as usize][y as usize][z as usize] == Block::Wood { continue; }
                blocks[x as usize][y as usize][z as usize] = Block::Leaves;
            }
        }
    }
    // Petite pointe au sommet pour que l'arbre ne soit pas plat.
    let top = base_y + 2;
    if top < CHUNK_HEIGHT {
        blocks[lx][top][lz] = Block::Leaves;
    }
}

/// Génère tous les blocs d'un chunk. Fait en trois passes : relief + biome,
/// couche de glace en toundra, puis arbres en forêt/marécage. Les arbres
/// viennent en dernier pour écrire par-dessus les blocs d'air des colonnes.
pub fn generate_chunk(chunk_x: i32, chunk_z: i32) -> ChunkBlocks {
    let noise = shared_perlin();
    let mut blocks = [[[Block::Air; CHUNK_SIZE]; CHUNK_HEIGHT]; CHUNK_SIZE];

    // Hauteur + biome par colonne, et remplissage stone/dirt/surface.
    let mut heights = [[0usize; CHUNK_SIZE]; CHUNK_SIZE];
    let mut col_biome = [[Biome::Plains; CHUNK_SIZE]; CHUNK_SIZE];

    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let wx = (chunk_x * CHUNK_SIZE as i32 + x as i32) as f64;
            let wz = (chunk_z * CHUNK_SIZE as i32 + z as i32) as f64;

            let height = terrain_height(noise, wx, wz);
            let biome  = biome_at(noise, wx, wz);
            heights[x][z] = height;
            col_biome[x][z] = biome;

            let surface = biome_surface(biome);
            let soil    = subsoil(biome);

            for y in 0..CHUNK_HEIGHT {
                blocks[x][y][z] = if y == 0 {
                    Block::Stone
                } else if y < height.saturating_sub(3) {
                    Block::Stone
                } else if y < height {
                    soil
                } else if y == height {
                    surface
                } else {
                    Block::Air
                };
            }

            // En toundra, on sème un peu de glace par touches — un bruit
            // haute fréquence donne un aspect tacheté plutôt qu'uniforme.
            if biome == Biome::Tundra && height + 1 < CHUNK_HEIGHT {
                let ice = noise.get([wx / 14.0 + 100.0, wz / 14.0 + 100.0]);
                if ice > 0.55 {
                    blocks[x][height][z] = Block::Ice;
                }
            }
        }
    }

    // Arbres : seulement en forêt et marécage, avec un seuil plus bas en
    // forêt pour qu'elles soient plus denses.
    for x in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            let biome = col_biome[x][z];
            if !matches!(biome, Biome::Forest | Biome::Swamp) { continue; }
            let h = heights[x][z];
            if h + 6 >= CHUNK_HEIGHT { continue; }

            let wx = (chunk_x * CHUNK_SIZE as i32 + x as i32) as f64;
            let wz = (chunk_z * CHUNK_SIZE as i32 + z as i32) as f64;

            let n = noise.get([wx * 1.37 + 7.0, wz * 1.37 + 7.0]);
            let threshold = if biome == Biome::Forest { 0.78 } else { 0.86 };
            if n > threshold {
                place_tree(&mut blocks, x, z, h, Block::Wood);
            }
        }
    }

    blocks
}
