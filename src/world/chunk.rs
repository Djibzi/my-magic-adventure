//! Chunks du monde — génération asynchrone, stockage et mesh face-culled.
//!
//! Un chunk = cube 16×64×16 de blocs, généré procéduralement via
//! `world::generation::generate_chunk` puis maillé en face-culling (on n'émet
//! de triangles que pour les faces exposées à l'Air). La génération tourne
//! sur `AsyncComputeTaskPool` pour ne pas freezer le thread principal ; les
//! meshes terminés sont uploadés vers le GPU par petits paquets chaque frame
//! via `poll_chunk_tasks`.
//!
//! `BlockEdits` stocke les modifications posées par le joueur (casser/poser)
//! indépendamment de la génération : au moment où un chunk (re)spawne, on
//! applique ses edits enregistrés par-dessus le monde procédural.

use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use std::collections::{HashMap, HashSet};

use crate::world::generation::generate_chunk;
use crate::world::texture_atlas::{
    TILE_GRASS_TOP, TILE_GRASS_SIDE, TILE_DIRT, TILE_STONE, TILE_SAND, TILE_SNOW,
    TILE_WOOD_SIDE, TILE_WOOD_TOP, TILE_LEAVES, TILE_PLANKS, TILE_ICE,
    side_uvs, top_uvs,
};
use crate::player::Player;
use crate::GameState;
use crate::RenderDistanceConfig;

pub const CHUNK_SIZE:   usize = 16;
pub const CHUNK_HEIGHT: usize = 64;

/// Handle vers la texture PNG de l'atlas des blocs (chargée au démarrage).
#[derive(Resource)]
pub struct BlockAtlas {
    pub handle: Handle<Image>,
}

/// Matériau partagé par tous les chunks (le PNG de l'atlas + paramètres PBR),
/// créé une seule fois pour éviter de dupliquer des ressources GPU.
#[derive(Resource)]
pub struct ChunkMaterial {
    pub handle: Handle<StandardMaterial>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Block {
    Air,
    Grass,
    Dirt,
    Stone,
    Sand,
    Snow,
    Wood,
    Leaves,
    Planks,
    Ice,
}

impl Block {
    /// Un bloc est "solide" si la lumière/le raycast le bloquent — en pratique
    /// tout sauf Air.
    pub fn is_solid(self) -> bool {
        !matches!(self, Block::Air)
    }

    /// Indice de tuile pour la face du dessus (atlas horizontal).
    pub fn tile_top(self) -> usize {
        match self {
            Block::Grass  => TILE_GRASS_TOP,
            Block::Dirt   => TILE_DIRT,
            Block::Stone  => TILE_STONE,
            Block::Sand   => TILE_SAND,
            Block::Snow   => TILE_SNOW,
            Block::Wood   => TILE_WOOD_TOP,
            Block::Leaves => TILE_LEAVES,
            Block::Planks => TILE_PLANKS,
            Block::Ice    => TILE_ICE,
            Block::Air    => 0,
        }
    }

    /// Indice de tuile pour les 4 faces latérales.
    pub fn tile_side(self) -> usize {
        match self {
            Block::Grass  => TILE_GRASS_SIDE,
            Block::Dirt   => TILE_DIRT,
            Block::Stone  => TILE_STONE,
            Block::Sand   => TILE_SAND,
            Block::Snow   => TILE_SNOW,
            Block::Wood   => TILE_WOOD_SIDE,
            Block::Leaves => TILE_LEAVES,
            Block::Planks => TILE_PLANKS,
            Block::Ice    => TILE_ICE,
            Block::Air    => 0,
        }
    }

    /// Indice de tuile pour la face du dessous — Grass montre de la terre en
    /// dessous, Snow aussi (on ne voit pas la neige par en-dessous).
    pub fn tile_bottom(self) -> usize {
        match self {
            Block::Grass | Block::Dirt => TILE_DIRT,
            Block::Stone  => TILE_STONE,
            Block::Sand   => TILE_SAND,
            Block::Snow   => TILE_DIRT,
            Block::Wood   => TILE_WOOD_TOP,
            Block::Leaves => TILE_LEAVES,
            Block::Planks => TILE_PLANKS,
            Block::Ice    => TILE_ICE,
            Block::Air    => 0,
        }
    }
}

pub type ChunkBlocks = [[[Block; CHUNK_SIZE]; CHUNK_HEIGHT]; CHUNK_SIZE];

/// Un chunk chargé en mémoire — sa `Transform` est posée aux coordonnées du
/// coin, et son mesh est construit à partir de `blocks`.
#[derive(Component)]
pub struct Chunk {
    pub blocks: Box<ChunkBlocks>,
}

/// Buffers de mesh "bruts" renvoyés par une tâche de génération. On ne
/// construit pas le `Mesh` directement dans la tâche parce que `Mesh` n'est
/// pas `Send` (il contient des handles GPU) — on assemble sur le thread
/// principal dans `poll_chunk_tasks`.
pub struct ChunkMeshArrays {
    pub positions: Vec<[f32; 3]>,
    pub normals:   Vec<[f32; 3]>,
    pub colors:    Vec<[f32; 4]>,
    pub uvs:       Vec<[f32; 2]>,
    pub indices:   Vec<u32>,
}

pub struct ChunkGenResult {
    pub blocks: Box<ChunkBlocks>,
    pub mesh:   ChunkMeshArrays,
}

/// Registre des chunks chargés et des tâches de génération en vol. Aussi
/// utilisé comme cache de "frame dernière évaluée" pour un early-out quand
/// le joueur ne traverse aucune frontière de chunk.
#[derive(Resource)]
pub struct ChunkManager {
    pub loaded:       HashMap<(i32, i32), Entity>,
    pub pending:      HashMap<(i32, i32), Task<ChunkGenResult>>,
    last_player_chunk: (i32, i32),
}

impl Default for ChunkManager {
    fn default() -> Self {
        Self {
            loaded: HashMap::default(),
            pending: HashMap::default(),
            // Sentinelle loin du spawn pour forcer la réévaluation à la
            // première frame de Loading.
            last_player_chunk: (i32::MIN, i32::MIN),
        }
    }
}

impl ChunkManager {
    /// Reset complet entre deux parties : vide les maps ET force
    /// `update_chunks` à réévaluer dès la prochaine frame (sentinelle
    /// loin du spawn habituel pour que l'early-out ne se déclenche pas
    /// par accident si le joueur respawn au même endroit).
    pub fn full_clear(&mut self) {
        self.loaded.clear();
        self.pending.clear();
        self.last_player_chunk = (i32::MAX, i32::MAX);
    }
}

pub type ChunkEdits = HashMap<(u8, u8, u8), Block>;

/// Modifications de blocs persistées (deltas vs génération procédurale).
/// Indexé par chunk pour qu'on puisse appliquer rapidement les edits d'un
/// chunk pendant sa génération asynchrone, sans cloner toute la map globale.
#[derive(Resource, Default)]
pub struct BlockEdits {
    pub map: HashMap<(i32, i32), ChunkEdits>,
}

impl BlockEdits {
    /// Enregistre une modification de bloc en coordonnées monde. Ignoré si
    /// la coord Y sort du chunk (ça ne devrait pas arriver mais on blinde).
    pub fn record(&mut self, world_pos: IVec3, block: Block) {
        if world_pos.y < 0 || world_pos.y >= CHUNK_HEIGHT as i32 { return; }
        let cx = world_pos.x.div_euclid(CHUNK_SIZE as i32);
        let cz = world_pos.z.div_euclid(CHUNK_SIZE as i32);
        let lx = world_pos.x.rem_euclid(CHUNK_SIZE as i32) as u8;
        let lz = world_pos.z.rem_euclid(CHUNK_SIZE as i32) as u8;
        let ly = world_pos.y as u8;
        self.map.entry((cx, cz)).or_default().insert((lx, ly, lz), block);
    }

    pub fn for_chunk(&self, cx: i32, cz: i32) -> Option<ChunkEdits> {
        self.map.get(&(cx, cz)).cloned()
    }

    pub fn clear(&mut self) { self.map.clear(); }
}

/// Conversion Block → id u8 stable (pour la persistance en fichier texte).
/// Les valeurs ne doivent jamais changer : elles sont écrites dans les saves.
pub fn block_to_id(b: Block) -> u8 {
    match b {
        Block::Air    => 0,
        Block::Grass  => 1,
        Block::Dirt   => 2,
        Block::Stone  => 3,
        Block::Sand   => 4,
        Block::Snow   => 5,
        Block::Wood   => 6,
        Block::Leaves => 7,
        Block::Planks => 8,
        Block::Ice    => 9,
    }
}
pub fn id_to_block(id: u8) -> Block {
    match id {
        1 => Block::Grass,
        2 => Block::Dirt,
        3 => Block::Stone,
        4 => Block::Sand,
        5 => Block::Snow,
        6 => Block::Wood,
        7 => Block::Leaves,
        8 => Block::Planks,
        9 => Block::Ice,
        _ => Block::Air,
    }
}

pub struct ChunkPlugin;

impl Plugin for ChunkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChunkManager>()
           .init_resource::<BlockEdits>()
           .add_systems(
               PreUpdate,
               (update_chunks, poll_chunk_tasks).chain().run_if(
                   in_state(GameState::InGame).or(in_state(GameState::Loading))
               ),
           );
    }
}

/// Décide quels chunks charger/décharger en fonction de la position du joueur.
///
/// On calcule l'ensemble `needed` (anneau carré de rayon `rd` autour du
/// joueur), on early-out si le joueur n'a pas traversé de frontière et que
/// tout est déjà couvert, puis on spawn des tâches de génération pour les
/// nouveaux chunks — triés par distance pour que le centre de vue soit prêt
/// en premier.
pub fn update_chunks(
    mut commands:      Commands,
    mut chunk_manager: ResMut<ChunkManager>,
    mut materials:     ResMut<Assets<StandardMaterial>>,
    atlas:             Option<Res<BlockAtlas>>,
    chunk_mat:         Option<Res<ChunkMaterial>>,
    player_query:      Query<&Transform, With<Player>>,
    render_dist:       Option<Res<RenderDistanceConfig>>,
    state:             Res<State<GameState>>,
    edits:             Res<BlockEdits>,
) {
    // L'atlas doit être chargé avant tout rendu de chunk.
    let Some(atlas) = atlas else {
        info!("update_chunks: atlas not ready yet");
        return;
    };

    // Création paresseuse du matériau partagé à la première frame où
    // l'atlas est dispo. Après ça il vit dans World pour toutes les parties.
    if chunk_mat.is_none() {
        let h = materials.add(StandardMaterial {
            base_color_texture:   Some(atlas.handle.clone()),
            perceptual_roughness: 1.0,
            metallic:             0.0,
            reflectance:          0.04,
            ..default()
        });
        commands.insert_resource(ChunkMaterial { handle: h });
    }

    let player_pos = match player_query.get_single() {
        Ok(t) => t.translation,
        Err(_) => return,
    };

    let in_loading = *state.get() == GameState::Loading;
    let rd = render_dist.map(|r| r.distance).unwrap_or(4);

    let px = (player_pos.x / CHUNK_SIZE as f32).floor() as i32;
    let pz = (player_pos.z / CHUNK_SIZE as f32).floor() as i32;

    let mut needed: HashSet<(i32, i32)> = HashSet::new();
    for dx in -rd..=rd {
        for dz in -rd..=rd {
            needed.insert((px + dx, pz + dz));
        }
    }

    // Early-out : joueur au même chunk qu'avant, et on a déjà tous les
    // chunks de `needed` en chargé ou pending. Économise ~300 HashMap
    // lookups la plupart des frames.
    if (px, pz) == chunk_manager.last_player_chunk
        && needed.iter().all(|k| {
            chunk_manager.loaded.contains_key(k) || chunk_manager.pending.contains_key(k)
        })
    {
        return;
    }
    chunk_manager.last_player_chunk = (px, pz);

    // Décharge tout ce qui est sorti de l'anneau.
    let to_remove: Vec<(i32, i32)> = chunk_manager.loaded.keys()
        .filter(|k| !needed.contains(k))
        .copied()
        .collect();
    for key in to_remove {
        if let Some(entity) = chunk_manager.loaded.remove(&key) {
            commands.entity(entity).despawn_recursive();
        }
    }
    // Annule aussi les tâches de génération devenues inutiles.
    chunk_manager.pending.retain(|k, _| needed.contains(k));

    // Nouveaux chunks à charger, triés par distance au joueur pour que
    // `handle_loading` voie vite les ~9 chunks centraux prêts.
    let mut to_load: Vec<(i32, i32)> = needed.iter().copied()
        .filter(|k| !chunk_manager.loaded.contains_key(k) && !chunk_manager.pending.contains_key(k))
        .collect();
    to_load.sort_by_key(|(cx, cz)| {
        let dx = cx - px;
        let dz = cz - pz;
        dx * dx + dz * dz
    });

    // Limite de tâches lancées par frame : plus large pendant le chargement
    // initial (l'utilisateur attend déjà), plus serré en jeu pour rester fluide.
    let budget: usize = if in_loading { 12 } else { 6 };
    let pool = AsyncComputeTaskPool::get();

    for coord in to_load.into_iter().take(budget) {
        let (cx, cz) = coord;
        // Snapshot des edits du chunk ; déplacé dans la closure pour rester `Send`.
        let chunk_edits = edits.for_chunk(cx, cz);
        let task = pool.spawn(async move {
            let mut blocks = generate_chunk(cx, cz);
            if let Some(edits) = chunk_edits {
                for ((lx, ly, lz), block) in &edits {
                    blocks[*lx as usize][*ly as usize][*lz as usize] = *block;
                }
            }
            let mesh = build_mesh_arrays(&blocks);
            ChunkGenResult { blocks: Box::new(blocks), mesh }
        });
        chunk_manager.pending.insert(coord, task);
    }
}

/// Termine les tâches prêtes, upload leur mesh vers le GPU et spawn l'entité
/// Chunk correspondante. On limite les uploads par frame pour éviter des
/// freezes au chargement massif (pipeline compilation GPU coûteuse).
pub fn poll_chunk_tasks(
    mut commands:      Commands,
    mut chunk_manager: ResMut<ChunkManager>,
    mut meshes:        ResMut<Assets<Mesh>>,
    chunk_mat:         Option<Res<ChunkMaterial>>,
    state:             Res<State<GameState>>,
) {
    // Sans matériau partagé on ne peut pas spawn ; `update_chunks` le crée
    // paresseusement, il sera dispo dès la frame suivante au pire.
    let Some(mat) = chunk_mat else { return };
    let mat_handle = mat.handle.clone();

    // Plus permissif pendant le chargement initial — le joueur attend déjà
    // et un freeze momentané au début est moins gênant qu'une progression
    // fuyante.
    let max_uploads_per_frame: usize = if *state.get() == GameState::Loading { 8 } else { 3 };

    let keys: Vec<(i32, i32)> = chunk_manager.pending.keys().copied().collect();
    let mut uploaded = 0usize;
    for coord in keys {
        if uploaded >= max_uploads_per_frame { break; }
        let ready = chunk_manager.pending.get_mut(&coord)
            .and_then(|task| block_on(poll_once(task)));

        let Some(result) = ready else { continue };
        chunk_manager.pending.remove(&coord);
        uploaded += 1;

        let mesh = arrays_to_mesh(result.mesh);
        let (cx, cz) = coord;
        let entity = commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mat_handle.clone()),
            Transform::from_xyz(
                (cx * CHUNK_SIZE as i32) as f32,
                0.0,
                (cz * CHUNK_SIZE as i32) as f32,
            ),
            Chunk { blocks: result.blocks },
        )).id();

        chunk_manager.loaded.insert(coord, entity);
    }
}

/// Luminosités par direction de face — simule un faux ambient-occlusion à la
/// Minecraft (le top est plein blanc, les côtés légèrement sombres, le bas
/// encore plus). Appliqué comme couleur vertex, pas de vrai AO calculé.
const LUM_TOP:    [f32; 4] = [1.00, 1.00, 1.00, 1.0];
const LUM_EW:     [f32; 4] = [0.80, 0.80, 0.80, 1.0];
const LUM_NS:     [f32; 4] = [0.88, 0.88, 0.88, 1.0];
const LUM_BOTTOM: [f32; 4] = [0.55, 0.55, 0.55, 1.0];

/// Point d'entrée pratique : construit tout le mesh d'un chunk en une passe.
pub fn build_mesh(blocks: &ChunkBlocks) -> Mesh {
    arrays_to_mesh(build_mesh_arrays(blocks))
}

/// Variante thread-safe : renvoie les buffers bruts au lieu d'un `Mesh`.
/// Face-culled : on n'émet une face que si son voisin est Air ou hors du chunk.
/// Note : on ne regarde pas les chunks voisins, donc les faces exposées en
/// bord de chunk sont toujours émises — compromis assumé pour garder la
/// génération parallèle simple.
pub fn build_mesh_arrays(blocks: &ChunkBlocks) -> ChunkMeshArrays {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals:   Vec<[f32; 3]> = Vec::new();
    let mut colors:    Vec<[f32; 4]> = Vec::new();
    let mut uvs:       Vec<[f32; 2]> = Vec::new();
    let mut indices:   Vec<u32>      = Vec::new();

    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_HEIGHT {
            for z in 0..CHUNK_SIZE {
                let block = blocks[x][y][z];
                if !block.is_solid() { continue; }

                let fx = x as f32;
                let fy = y as f32;
                let fz = z as f32;

                // Top (+Y)
                if y == CHUNK_HEIGHT - 1 || !blocks[x][y + 1][z].is_solid() {
                    add_face(
                        &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                        [fx,fy+1.,fz], [fx+1.,fy+1.,fz], [fx+1.,fy+1.,fz+1.], [fx,fy+1.,fz+1.],
                        [0., 1., 0.], LUM_TOP, top_uvs(block.tile_top()),
                    );
                }
                // Bottom (-Y)
                if y == 0 || !blocks[x][y - 1][z].is_solid() {
                    add_face(
                        &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                        [fx,fy,fz+1.], [fx+1.,fy,fz+1.], [fx+1.,fy,fz], [fx,fy,fz],
                        [0.,-1., 0.], LUM_BOTTOM, top_uvs(block.tile_bottom()),
                    );
                }
                // +X
                if x == CHUNK_SIZE - 1 || !blocks[x + 1][y][z].is_solid() {
                    add_face(
                        &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                        [fx+1.,fy,fz], [fx+1.,fy,fz+1.], [fx+1.,fy+1.,fz+1.], [fx+1.,fy+1.,fz],
                        [1., 0., 0.], LUM_EW, side_uvs(block.tile_side()),
                    );
                }
                // -X
                if x == 0 || !blocks[x - 1][y][z].is_solid() {
                    add_face(
                        &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                        [fx,fy,fz+1.], [fx,fy,fz], [fx,fy+1.,fz], [fx,fy+1.,fz+1.],
                        [-1., 0., 0.], LUM_EW, side_uvs(block.tile_side()),
                    );
                }
                // +Z
                if z == CHUNK_SIZE - 1 || !blocks[x][y][z + 1].is_solid() {
                    add_face(
                        &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                        [fx+1.,fy,fz+1.], [fx,fy,fz+1.], [fx,fy+1.,fz+1.], [fx+1.,fy+1.,fz+1.],
                        [0., 0., 1.], LUM_NS, side_uvs(block.tile_side()),
                    );
                }
                // -Z
                if z == 0 || !blocks[x][y][z - 1].is_solid() {
                    add_face(
                        &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                        [fx,fy,fz], [fx+1.,fy,fz], [fx+1.,fy+1.,fz], [fx,fy+1.,fz],
                        [0., 0.,-1.], LUM_NS, side_uvs(block.tile_side()),
                    );
                }
            }
        }
    }

    ChunkMeshArrays { positions, normals, colors, uvs, indices }
}

pub fn arrays_to_mesh(arr: ChunkMeshArrays) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, arr.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL,   arr.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR,    arr.colors);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0,     arr.uvs);
    mesh.insert_indices(Indices::U32(arr.indices));
    mesh
}

/// Ajoute une face (2 triangles, 4 sommets) aux buffers. L'enroulement est
/// inversé pour matcher le back-face culling de Bevy : front-face = CCW vu
/// depuis l'extérieur du cube.
fn add_face(
    positions: &mut Vec<[f32; 3]>,
    normals:   &mut Vec<[f32; 3]>,
    colors:    &mut Vec<[f32; 4]>,
    uvs:       &mut Vec<[f32; 2]>,
    indices:   &mut Vec<u32>,
    v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], v3: [f32; 3],
    normal:   [f32; 3],
    color:    [f32; 4],
    face_uvs: [[f32; 2]; 4],
) {
    let base = positions.len() as u32;
    positions.extend_from_slice(&[v0, v1, v2, v3]);
    normals.extend_from_slice(&[normal; 4]);
    colors.extend_from_slice(&[color; 4]);
    uvs.extend_from_slice(&face_uvs);
    indices.extend_from_slice(&[base, base+2, base+1, base, base+3, base+2]);
}
