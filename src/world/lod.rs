//! LOD à la Distant-Horizons — meshes simplifiés pour les chunks lointains.
//!
//! Les chunks sont regroupés en "super-chunks" de SUPER_SIZE × SUPER_SIZE
//! (actuellement 4×4, soit un footprint de 64×64 blocs par super-chunk). Un
//! seul mesh/entité couvre tout un super-chunk et ne contient que la
//! silhouette du terrain (face du haut + faces latérales aux cassures de
//! hauteur). Les arbres ne sont dessinés que pour les super-chunks proches
//! du joueur.
//!
//! L'intérêt principal : diviser par ~16× le nombre de draw-calls sur
//! l'anneau LOD, qui est le coût GPU dominant à l'horizon.
//!
//! Le `needed` HashSet est mis en cache et ne se reconstruit que quand le
//! joueur traverse une frontière de super-chunk ou que le render distance
//! change — la plupart des frames font juste un early-out.

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use noise::NoiseFn;
use std::collections::{HashMap, HashSet};

use crate::player::Player;
use crate::GameState;
use crate::RenderDistanceConfig;

use super::chunk::{
    Block, ChunkMaterial, ChunkMeshArrays, arrays_to_mesh,
    CHUNK_SIZE, CHUNK_HEIGHT,
};
use super::generation::{terrain_height, biome_at, biome_surface, shared_perlin, Biome};
use super::texture_atlas::{top_uvs, side_uvs};

/// Côté d'un super-chunk, en chunks normaux.
const SUPER_SIZE: i32 = 4;
/// Côté d'un super-chunk, en blocs (= unités monde).
const SUPER_EXTENT: usize = SUPER_SIZE as usize * CHUNK_SIZE;

/// Extension du LOD au-delà du rayon de rendu détaillé. Chaque unité vaut
/// SUPER_SIZE chunks = SUPER_EXTENT blocs.
const LOD_EXTRA_SUPERS: i32 = 3;

/// Distance max (en chunks) à laquelle un super-chunk inclut encore les
/// arbres. Au-delà, l'horizon reste terrain-silhouette uniquement.
const LOD_TREE_DIST_CHUNKS: i32 = 8;

#[derive(Component)]
pub struct LodChunk;

struct LodGenResult {
    mesh: ChunkMeshArrays,
}

/// Registre des super-chunks chargés + cache du ring `needed`. Les sentinelles
/// `i32::MAX` / `i32::MIN` garantissent que la première frame après `clear()`
/// reconstruit systématiquement le cache.
#[derive(Resource)]
pub struct LodManager {
    loaded:  HashMap<(i32, i32), Entity>,
    pending: HashMap<(i32, i32), Task<LodGenResult>>,
    /// Anneau de coordonnées super-chunks nécessaires — ne se reconstruit que
    /// quand le joueur change de super-chunk ou que `rd` change.
    needed: HashSet<(i32, i32)>,
    last_player_super: (i32, i32),
    last_rd: i32,
}

impl Default for LodManager {
    fn default() -> Self {
        Self {
            loaded: HashMap::new(),
            pending: HashMap::new(),
            needed: HashSet::new(),
            last_player_super: (i32::MAX, i32::MAX),
            last_rd: i32::MIN,
        }
    }
}

impl LodManager {
    /// Reset complet entre deux parties.
    pub fn clear(&mut self) {
        self.loaded.clear();
        self.pending.clear();
        self.needed.clear();
        self.last_player_super = (i32::MAX, i32::MAX);
        self.last_rd = i32::MIN;
    }
}

pub struct LodPlugin;

impl Plugin for LodPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LodManager>()
           .add_systems(
               PreUpdate,
               (update_lod, poll_lod_tasks).chain()
                   .run_if(in_state(GameState::InGame)),
           );
    }
}

/// Données de génération d'un super-chunk : une heightmap plate (row-major)
/// + la liste des colonnes où planter un arbre simplifié.
pub struct SuperLodData {
    /// Row-major : `[x * SUPER_EXTENT + z] → (height, surface_block)`
    heightmap: Vec<(usize, Block)>,
    /// Arbres sous la forme `(local_x, local_z, surface_height)`,
    /// coordonnées locales dans 0..SUPER_EXTENT.
    trees: Vec<(usize, usize, usize)>,
}

/// Génère la heightmap + liste d'arbres d'un super-chunk.
///
/// `detail_zone = Some((px_chunk, pz_chunk, rd))` : les colonnes dont la coord
/// chunk est dans ±rd autour de (px, pz) sont sautées pour les arbres, sinon
/// on afficherait les arbres LOD par-dessus les vrais arbres des chunks
/// détaillés (double rendu visible).
fn generate_super_lod_data(
    sx: i32,
    sz: i32,
    with_trees: bool,
    detail_zone: Option<(i32, i32, i32)>,
) -> SuperLodData {
    let noise = shared_perlin();
    let mut hm = vec![(0usize, Block::Air); SUPER_EXTENT * SUPER_EXTENT];
    let mut biome_map = vec![Biome::Plains; SUPER_EXTENT * SUPER_EXTENT];

    let base_wx = sx * SUPER_SIZE * CHUNK_SIZE as i32;
    let base_wz = sz * SUPER_SIZE * CHUNK_SIZE as i32;

    for x in 0..SUPER_EXTENT {
        for z in 0..SUPER_EXTENT {
            let wx = (base_wx + x as i32) as f64;
            let wz = (base_wz + z as i32) as f64;
            let h = terrain_height(noise, wx, wz);
            let biome = biome_at(noise, wx, wz);
            hm[x * SUPER_EXTENT + z] = (h, biome_surface(biome));
            biome_map[x * SUPER_EXTENT + z] = biome;
        }
    }

    let mut trees = Vec::new();
    if with_trees {
        for x in 0..SUPER_EXTENT {
            for z in 0..SUPER_EXTENT {
                let biome = biome_map[x * SUPER_EXTENT + z];
                if !matches!(biome, Biome::Forest | Biome::Swamp) { continue; }
                let h = hm[x * SUPER_EXTENT + z].0;
                if h + 7 >= CHUNK_HEIGHT { continue; }
                let wx_i = base_wx + x as i32;
                let wz_i = base_wz + z as i32;
                // Exclut les colonnes déjà couvertes par les chunks détaillés :
                // sinon les arbres LOD se superposeraient aux vrais arbres.
                if let Some((pcx, pcz, rd)) = detail_zone {
                    let cx = wx_i.div_euclid(CHUNK_SIZE as i32);
                    let cz = wz_i.div_euclid(CHUNK_SIZE as i32);
                    if (cx - pcx).abs() <= rd && (cz - pcz).abs() <= rd {
                        continue;
                    }
                }
                let wx = wx_i as f64;
                let wz = wz_i as f64;
                let n = noise.get([wx * 1.37 + 7.0, wz * 1.37 + 7.0]);
                let threshold = if biome == Biome::Forest { 0.78 } else { 0.86 };
                if n > threshold {
                    trees.push((x, z, h));
                }
            }
        }
    }

    SuperLodData { heightmap: hm, trees }
}

#[inline]
fn hm_get(hm: &[(usize, Block)], x: usize, z: usize) -> (usize, Block) {
    hm[x * SUPER_EXTENT + z]
}

// Mêmes luminosités que `chunk::build_mesh_arrays` pour que les transitions
// entre chunks détaillés et LOD ne "flashent" pas à l'horizon.
const LUM_TOP: [f32; 4] = [1.00, 1.00, 1.00, 1.0];
const LUM_EW:  [f32; 4] = [0.80, 0.80, 0.80, 1.0];
const LUM_NS:  [f32; 4] = [0.88, 0.88, 0.88, 1.0];

/// Construit le mesh surface-only d'un super-chunk : une face top par colonne
/// + des faces latérales là où le voisin est plus bas (cascade de falaise).
/// Les arbres sont ajoutés à la fin comme cuboïdes simplifiés.
fn build_super_lod_mesh(data: &SuperLodData) -> ChunkMeshArrays {
    let hm = &data.heightmap;
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals:   Vec<[f32; 3]> = Vec::new();
    let mut colors:    Vec<[f32; 4]> = Vec::new();
    let mut uvs:       Vec<[f32; 2]> = Vec::new();
    let mut indices:   Vec<u32>      = Vec::new();

    for x in 0..SUPER_EXTENT {
        for z in 0..SUPER_EXTENT {
            let (h, block) = hm_get(hm, x, z);
            if !block.is_solid() { continue; }

            let fx = x as f32;
            let fy = h as f32;
            let fz = z as f32;

            // Face du haut : visible de toute l'atmosphère, toujours émise.
            add_face(
                &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                [fx, fy + 1., fz], [fx + 1., fy + 1., fz],
                [fx + 1., fy + 1., fz + 1.], [fx, fy + 1., fz + 1.],
                [0., 1., 0.], LUM_TOP, top_uvs(block.tile_top()),
            );

            // Faces latérales : une par direction où le voisin est plus bas.
            let side_neighbors: [(i32, i32, [f32; 3], [f32; 4]); 4] = [
                ( 1,  0, [1., 0., 0.], LUM_EW),
                (-1,  0, [-1., 0., 0.], LUM_EW),
                ( 0,  1, [0., 0., 1.], LUM_NS),
                ( 0, -1, [0., 0., -1.], LUM_NS),
            ];

            for (dx, dz, normal, lum) in side_neighbors {
                let nx = x as i32 + dx;
                let nz = z as i32 + dz;
                let nh = if nx >= 0 && nx < SUPER_EXTENT as i32
                         && nz >= 0 && nz < SUPER_EXTENT as i32
                {
                    hm_get(hm, nx as usize, nz as usize).0
                } else {
                    0
                };
                if nh >= h { continue; }

                let low  = nh as f32 + 1.0;
                let high = fy + 1.0;
                let mid  = fy;

                // Choix des sommets selon la face qu'on dessine. Renvoie les
                // 4 coins du rectangle vertical correspondant.
                let verts = |y_lo: f32, y_hi: f32| -> ([f32;3],[f32;3],[f32;3],[f32;3]) {
                    match (dx, dz) {
                        ( 1, 0) => ([fx+1.,y_lo,fz],[fx+1.,y_lo,fz+1.],[fx+1.,y_hi,fz+1.],[fx+1.,y_hi,fz]),
                        (-1, 0) => ([fx,y_lo,fz+1.],[fx,y_lo,fz],[fx,y_hi,fz],[fx,y_hi,fz+1.]),
                        ( 0, 1) => ([fx+1.,y_lo,fz+1.],[fx,y_lo,fz+1.],[fx,y_hi,fz+1.],[fx+1.,y_hi,fz+1.]),
                        _       => ([fx,y_lo,fz],[fx+1.,y_lo,fz],[fx+1.,y_hi,fz],[fx,y_hi,fz]),
                    }
                };

                // Haut de la falaise (surface du bloc courant) — texture surface.
                if mid < high {
                    let (v0,v1,v2,v3) = verts(mid.max(low), high);
                    add_face(
                        &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                        v0, v1, v2, v3, normal, lum, side_uvs(block.tile_side()),
                    );
                }
                // Bas de la falaise — texture dirt pour que la paroi ait l'air
                // terreuse, pas recouverte d'herbe.
                if low < mid {
                    let (v0,v1,v2,v3) = verts(low, mid);
                    add_face(
                        &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
                        v0, v1, v2, v3, normal, lum, side_uvs(Block::Dirt.tile_side()),
                    );
                }
            }
        }
    }

    // Arbres : silhouette simplifiée (tronc + cube de canopée). Pas de
    // détail de feuilles — c'est suffisant à distance.
    for &(lx, lz, h) in &data.trees {
        let fx = lx as f32;
        let fz = lz as f32;
        let fh = h as f32;

        add_cuboid(
            &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
            [fx, fh + 1.0, fz], [fx + 1.0, fh + 4.0, fz + 1.0],
            Block::Wood.tile_top(), Block::Wood.tile_side(),
        );

        // La canopée peut dépasser les bords du super-chunk : on ne clamp
        // pas, sinon les feuilles seraient coupées net tous les 64 blocs.
        let cx0 = lx as f32 - 1.0;
        let cx1 = lx as f32 + 2.0;
        let cz0 = lz as f32 - 1.0;
        let cz1 = lz as f32 + 2.0;
        add_cuboid(
            &mut positions, &mut normals, &mut colors, &mut uvs, &mut indices,
            [cx0, fh + 4.0, cz0], [cx1, fh + 7.0, cz1],
            Block::Leaves.tile_top(), Block::Leaves.tile_side(),
        );
    }

    ChunkMeshArrays { positions, normals, colors, uvs, indices }
}

fn add_cuboid(
    positions: &mut Vec<[f32; 3]>,
    normals:   &mut Vec<[f32; 3]>,
    colors:    &mut Vec<[f32; 4]>,
    uvs:       &mut Vec<[f32; 2]>,
    indices:   &mut Vec<u32>,
    min: [f32; 3],
    max: [f32; 3],
    top_tile: usize,
    side_tile: usize,
) {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let top_uv  = top_uvs(top_tile);
    let side_uv = side_uvs(side_tile);

    add_face(positions, normals, colors, uvs, indices,
        [x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1],
        [0., 1., 0.], LUM_TOP, top_uv);
    add_face(positions, normals, colors, uvs, indices,
        [x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0],
        [0., -1., 0.], LUM_TOP, top_uv);
    add_face(positions, normals, colors, uvs, indices,
        [x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0],
        [1., 0., 0.], LUM_EW, side_uv);
    add_face(positions, normals, colors, uvs, indices,
        [x0, y0, z1], [x0, y0, z0], [x0, y1, z0], [x0, y1, z1],
        [-1., 0., 0.], LUM_EW, side_uv);
    add_face(positions, normals, colors, uvs, indices,
        [x1, y0, z1], [x0, y0, z1], [x0, y1, z1], [x1, y1, z1],
        [0., 0., 1.], LUM_NS, side_uv);
    add_face(positions, normals, colors, uvs, indices,
        [x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0],
        [0., 0., -1.], LUM_NS, side_uv);
}

fn add_face(
    positions: &mut Vec<[f32; 3]>,
    normals:   &mut Vec<[f32; 3]>,
    colors:    &mut Vec<[f32; 4]>,
    uvs:       &mut Vec<[f32; 2]>,
    indices:   &mut Vec<u32>,
    v0: [f32; 3], v1: [f32; 3], v2: [f32; 3], v3: [f32; 3],
    normal: [f32; 3],
    color:  [f32; 4],
    face_uvs: [[f32; 2]; 4],
) {
    let base = positions.len() as u32;
    positions.extend_from_slice(&[v0, v1, v2, v3]);
    normals.extend_from_slice(&[normal; 4]);
    colors.extend_from_slice(&[color; 4]);
    uvs.extend_from_slice(&face_uvs);
    indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
}

/// Décide quels super-chunks charger/décharger. Fast-path si le joueur n'a
/// pas traversé de frontière super et que rien n'est en cours — retourne
/// après deux comparaisons et un `.is_empty()`.
fn update_lod(
    mut commands:    Commands,
    mut lod_mgr:     ResMut<LodManager>,
    player_query:    Query<&Transform, With<Player>>,
    render_dist:     Option<Res<RenderDistanceConfig>>,
    chunk_mat:       Option<Res<ChunkMaterial>>,
) {
    let Some(_mat) = chunk_mat else { return };
    let player_pos = match player_query.get_single() {
        Ok(t) => t.translation,
        Err(_) => return,
    };

    let rd = render_dist.map(|r| r.distance).unwrap_or(4);

    let px = (player_pos.x / CHUNK_SIZE as f32).floor() as i32;
    let pz = (player_pos.z / CHUNK_SIZE as f32).floor() as i32;

    let psx = px.div_euclid(SUPER_SIZE);
    let psz = pz.div_euclid(SUPER_SIZE);

    let super_changed = (psx, psz) != lod_mgr.last_player_super || rd != lod_mgr.last_rd;

    // Fast-path : joueur stable, rien à générer → on sort immédiatement.
    // Économise ~289 insertions HashSet et ~300 lookups HashMap par frame.
    if !super_changed && lod_mgr.pending.is_empty() {
        return;
    }

    // Ne reconstruit le ring `needed` que si vraiment les entrées ont changé.
    if super_changed {
        let super_radius = rd / SUPER_SIZE + 1 + LOD_EXTRA_SUPERS;
        lod_mgr.needed.clear();
        for dsx in -super_radius..=super_radius {
            for dsz in -super_radius..=super_radius {
                let sx = psx + dsx;
                let sz = psz + dsz;
                let cx_lo = sx * SUPER_SIZE;
                let cx_hi = cx_lo + SUPER_SIZE - 1;
                let cz_lo = sz * SUPER_SIZE;
                let cz_hi = cz_lo + SUPER_SIZE - 1;
                // Super-chunks entièrement inclus dans la zone détaillée :
                // inutiles côté LOD, les chunks normaux s'en occupent.
                let fully_in_detail = cx_lo >= px - rd && cx_hi <= px + rd
                                   && cz_lo >= pz - rd && cz_hi <= pz + rd;
                if fully_in_detail { continue; }
                lod_mgr.needed.insert((sx, sz));
            }
        }
        lod_mgr.last_player_super = (psx, psz);
        lod_mgr.last_rd = rd;

        // Décharge ce qui est sorti du ring.
        let to_remove: Vec<(i32, i32)> = lod_mgr.loaded.keys()
            .filter(|k| !lod_mgr.needed.contains(k))
            .copied()
            .collect();
        for key in to_remove {
            if let Some(entity) = lod_mgr.loaded.remove(&key) {
                commands.entity(entity).despawn_recursive();
            }
        }
        // Split-borrow pour que la closure `retain` puisse lire `needed`
        // pendant que `pending` est emprunté en mutable.
        let mgr = &mut *lod_mgr;
        let needed = &mgr.needed;
        mgr.pending.retain(|k, _| needed.contains(k));
    }

    // Après le nettoyage, loaded/pending n'ont que des clés dans `needed`,
    // donc la comparaison de longueur suffit comme test de couverture.
    if lod_mgr.loaded.len() + lod_mgr.pending.len() >= lod_mgr.needed.len() {
        return;
    }

    // Nouveaux super-chunks à charger, les plus proches d'abord.
    let mut to_load: Vec<(i32, i32)> = lod_mgr.needed.iter()
        .filter(|k| !lod_mgr.loaded.contains_key(k) && !lod_mgr.pending.contains_key(k))
        .copied()
        .collect();
    to_load.sort_by_key(|(sx, sz)| {
        let dx = sx - psx;
        let dz = sz - psz;
        dx * dx + dz * dz
    });

    // Chaque super = ~16× le travail d'un chunk LOD normal, donc budget tight.
    let budget = 2;
    let pool = AsyncComputeTaskPool::get();

    for (sx, sz) in to_load.into_iter().take(budget) {
        // Arbres : inclus seulement si le super-chunk est proche du joueur.
        // Au-delà de LOD_TREE_DIST_CHUNKS, on garde juste la silhouette du
        // terrain pour économiser des triangles à l'horizon.
        let cx_lo = sx * SUPER_SIZE;
        let cx_hi = cx_lo + SUPER_SIZE - 1;
        let cz_lo = sz * SUPER_SIZE;
        let cz_hi = cz_lo + SUPER_SIZE - 1;
        let closest_cx = px.clamp(cx_lo, cx_hi);
        let closest_cz = pz.clamp(cz_lo, cz_hi);
        let chunk_dist = (closest_cx - px).abs().max((closest_cz - pz).abs());
        let with_trees = chunk_dist <= LOD_TREE_DIST_CHUNKS;

        let detail_zone = Some((px, pz, rd));
        let task = pool.spawn(async move {
            let data = generate_super_lod_data(sx, sz, with_trees, detail_zone);
            let mesh = build_super_lod_mesh(&data);
            LodGenResult { mesh }
        });
        lod_mgr.pending.insert((sx, sz), task);
    }
}

/// Termine les tâches prêtes, upload leur mesh et spawn l'entité LOD. Plus
/// petit budget que les chunks normaux (2/frame) parce qu'un mesh LOD est
/// beaucoup plus lourd en triangles qu'un chunk normal.
fn poll_lod_tasks(
    mut commands:  Commands,
    mut lod_mgr:   ResMut<LodManager>,
    mut meshes:    ResMut<Assets<Mesh>>,
    chunk_mat:     Option<Res<ChunkMaterial>>,
) {
    let Some(mat) = chunk_mat else { return };
    let mat_handle = mat.handle.clone();

    let max_per_frame: usize = 2;

    let keys: Vec<(i32, i32)> = lod_mgr.pending.keys().copied().collect();
    let mut uploaded = 0usize;
    for coord in keys {
        if uploaded >= max_per_frame { break; }
        let ready = lod_mgr.pending.get_mut(&coord)
            .and_then(|task| block_on(poll_once(task)));

        let Some(result) = ready else { continue };
        lod_mgr.pending.remove(&coord);
        uploaded += 1;

        let mesh = arrays_to_mesh(result.mesh);
        let (sx, sz) = coord;
        let entity = commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mat_handle.clone()),
            Transform::from_xyz(
                (sx * SUPER_SIZE * CHUNK_SIZE as i32) as f32,
                0.0,
                (sz * SUPER_SIZE * CHUNK_SIZE as i32) as f32,
            ),
            LodChunk,
        )).id();

        lod_mgr.loaded.insert(coord, entity);
    }
}
