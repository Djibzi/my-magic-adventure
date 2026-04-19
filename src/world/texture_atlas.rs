//! Indices et helpers UV pour l'atlas de textures des blocs.
//!
//! L'atlas est une bande horizontale de 11 tuiles 16×16 dans `assets/blocks.png`,
//! chargée par `world::init_block_atlas`. Les constantes ci-dessous donnent la
//! position de chaque tuile (0 à gauche). Les deux fonctions `*_uvs` renvoient
//! les UV des 4 coins d'une face selon son orientation (latérale ou top/bottom).

pub const NUM_TILES: usize = 11;

pub const TILE_GRASS_TOP:  usize = 0;
pub const TILE_GRASS_SIDE: usize = 1;
pub const TILE_DIRT:       usize = 2;
pub const TILE_STONE:      usize = 3;
pub const TILE_SAND:       usize = 4;
pub const TILE_SNOW:       usize = 5;
pub const TILE_WOOD_SIDE:  usize = 6;
pub const TILE_WOOD_TOP:   usize = 7;
pub const TILE_LEAVES:     usize = 8;
pub const TILE_PLANKS:     usize = 9;
pub const TILE_ICE:        usize = 10;

/// UV pour une face latérale : V croît du haut (0) vers le bas (1) pour
/// garder la texture dans le bon sens quel que soit le côté du cube.
pub fn side_uvs(tile: usize) -> [[f32; 2]; 4] {
    let u0 = tile as f32 / NUM_TILES as f32;
    let u1 = (tile + 1) as f32 / NUM_TILES as f32;
    [
        [u0, 1.0],
        [u1, 1.0],
        [u1, 0.0],
        [u0, 0.0],
    ]
}

/// UV pour une face top ou bottom — la texture remplit tout le carré dans
/// l'orientation naturelle de l'atlas.
pub fn top_uvs(tile: usize) -> [[f32; 2]; 4] {
    let u0 = tile as f32 / NUM_TILES as f32;
    let u1 = (tile + 1) as f32 / NUM_TILES as f32;
    [
        [u0, 0.0],
        [u1, 0.0],
        [u1, 1.0],
        [u0, 1.0],
    ]
}
