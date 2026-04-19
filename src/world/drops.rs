//! Mini-blocs qui tombent au sol quand un bloc est cassé.
//!
//! On spawn un petit cube texturé à la position du bloc, il retombe par
//! gravité, flotte avec un léger bob, tourne sur lui-même, et se ramasse
//! dès que le joueur s'approche à moins de `PICKUP_RADIUS`. Après
//! `DROP_LIFETIME` secondes sans ramassage il disparaît pour ne pas polluer
//! la carte.

use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};
use bevy::render::render_asset::RenderAssetUsages;

use crate::player::Player;
use crate::crafting::BlockCollected;
use crate::world::chunk::{Block, Chunk, ChunkManager, ChunkMaterial, CHUNK_HEIGHT, CHUNK_SIZE};
use crate::world::texture_atlas::{top_uvs, side_uvs};
use crate::GameState;

const PICKUP_RADIUS: f32 = 1.6;
const DROP_SCALE:    f32 = 0.30;
const DROP_GRAVITY:  f32 = 16.0;
const DROP_LIFETIME: f32 = 120.0;
const BOB_AMPLITUDE: f32 = 0.08;
const BOB_SPEED:     f32 = 2.5;
const SPIN_SPEED:    f32 = 1.8;

#[derive(Component)]
pub struct DroppedItem {
    pub block:     Block,
    pub velocity:  Vec3,
    pub age:       f32,
    pub on_ground: bool,
    pub base_y:    f32,
}

#[derive(Event)]
pub struct SpawnDroppedItem {
    pub block:    Block,
    pub position: Vec3,
}

pub struct DropsPlugin;

impl Plugin for DropsPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SpawnDroppedItem>()
           .add_systems(Update, (
               spawn_drops,
               drop_physics,
               drop_animate,
               pickup_drops,
               despawn_old_drops,
           ).chain().run_if(in_state(GameState::InGame)));
    }
}

/// Construit un mini cube texturé avec les 6 faces UV-mappées sur l'atlas
/// (mêmes luminosités directionnelles que `chunk::build_mesh` pour que le
/// drop ait l'air cohérent avec le bloc d'origine).
fn build_drop_mesh(block: Block) -> Mesh {
    let s = 0.5; // Demi-taille ; la Transform applique ensuite DROP_SCALE.

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(24);
    let mut normals:   Vec<[f32; 3]> = Vec::with_capacity(24);
    let mut colors:    Vec<[f32; 4]> = Vec::with_capacity(24);
    let mut uvs:       Vec<[f32; 2]> = Vec::with_capacity(24);
    let mut indices:   Vec<u32>      = Vec::with_capacity(36);

    // Mêmes teintes que les chunks pour que la petite cube ait l'air d'un
    // vrai morceau arraché du terrain.
    let lum_top:    [f32; 4] = [1.00, 1.00, 1.00, 1.0];
    let lum_bottom: [f32; 4] = [0.65, 0.65, 0.65, 1.0];
    let lum_ew:     [f32; 4] = [0.80, 0.80, 0.80, 1.0];
    let lum_ns:     [f32; 4] = [0.88, 0.88, 0.88, 1.0];

    let top_uv    = top_uvs(block.tile_top());
    let bottom_uv = top_uvs(block.tile_bottom());
    let side_uv   = side_uvs(block.tile_side());

    // Petit helper pour éviter de répéter 6 fois les mêmes extends.
    let mut add = |v0: [f32;3], v1: [f32;3], v2: [f32;3], v3: [f32;3],
                   n: [f32;3], c: [f32;4], fuv: [[f32;2];4]| {
        let base = positions.len() as u32;
        positions.extend_from_slice(&[v0, v1, v2, v3]);
        normals.extend_from_slice(&[n; 4]);
        colors.extend_from_slice(&[c; 4]);
        uvs.extend_from_slice(&fuv);
        indices.extend_from_slice(&[base, base+2, base+1, base, base+3, base+2]);
    };

    add([-s, s, -s], [s, s, -s], [s, s, s], [-s, s, s],
        [0.,1.,0.], lum_top, top_uv);
    add([-s, -s, s], [s, -s, s], [s, -s, -s], [-s, -s, -s],
        [0.,-1.,0.], lum_bottom, bottom_uv);
    add([-s, -s, s], [-s, s, s], [s, s, s], [s, -s, s],
        [0.,0.,1.], lum_ns, side_uv);
    add([s, -s, -s], [s, s, -s], [-s, s, -s], [-s, -s, -s],
        [0.,0.,-1.], lum_ns, side_uv);
    add([s, -s, s], [s, s, s], [s, s, -s], [s, -s, -s],
        [1.,0.,0.], lum_ew, side_uv);
    add([-s, -s, -s], [-s, s, -s], [-s, s, s], [-s, -s, s],
        [-1.,0.,0.], lum_ew, side_uv);

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL,   normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR,    colors);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0,     uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Matérialise chaque `SpawnDroppedItem` : on crée un mini cube qui part en
/// l'air avec une petite vitesse horizontale. L'angle de dispersion vient
/// de la position pour que plusieurs drops au même endroit ne partent pas
/// tous dans la même direction.
fn spawn_drops(
    mut commands:  Commands,
    mut events:    EventReader<SpawnDroppedItem>,
    mut meshes:    ResMut<Assets<Mesh>>,
    chunk_mat:     Option<Res<ChunkMaterial>>,
) {
    let Some(mat) = chunk_mat else { return };
    let mat_handle = mat.handle.clone();

    for ev in events.read() {
        let drop_mesh = build_drop_mesh(ev.block);

        let angle = ev.position.x * 3.7 + ev.position.z * 2.3;
        let vx = angle.cos() * 2.0;
        let vz = angle.sin() * 2.0;

        commands.spawn((
            Mesh3d(meshes.add(drop_mesh)),
            MeshMaterial3d(mat_handle.clone()),
            Transform::from_translation(ev.position)
                .with_scale(Vec3::splat(DROP_SCALE)),
            DroppedItem {
                block:     ev.block,
                velocity:  Vec3::new(vx, 4.0, vz),
                age:       0.0,
                on_ground: false,
                base_y:    ev.position.y,
            },
        ));
    }
}

/// Physique simple : gravité + détection de sol. Une fois posé, on fige la
/// vélocité et on enregistre `base_y` pour le bob d'animation.
fn drop_physics(
    time:          Res<Time>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
    mut items:     Query<(&mut DroppedItem, &mut Transform)>,
) {
    let dt = time.delta_secs();

    for (mut item, mut tf) in &mut items {
        if item.on_ground { continue; }

        item.velocity.y -= DROP_GRAVITY * dt;
        tf.translation += item.velocity * dt;

        let pos = tf.translation;
        let bx = pos.x.floor() as i32;
        let by = (pos.y - DROP_SCALE * 0.5).floor() as i32;
        let bz = pos.z.floor() as i32;

        if by < 0 {
            tf.translation.y = 0.5;
            item.velocity = Vec3::ZERO;
            item.on_ground = true;
            item.base_y = tf.translation.y;
            continue;
        }

        if by < CHUNK_HEIGHT as i32 {
            let cx = bx.div_euclid(CHUNK_SIZE as i32);
            let cz = bz.div_euclid(CHUNK_SIZE as i32);
            let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
            let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;

            let solid = if let Some(&entity) = chunk_manager.loaded.get(&(cx, cz)) {
                if let Ok(chunk) = chunk_q.get(entity) {
                    chunk.blocks[lx][by as usize][lz].is_solid()
                } else { false }
            } else { false };

            if solid {
                tf.translation.y = by as f32 + 1.0 + DROP_SCALE * 0.5;
                item.velocity = Vec3::ZERO;
                item.on_ground = true;
                item.base_y = tf.translation.y;
            }
        }
    }
}

/// Rotation continue autour de Y + bob vertical léger quand le drop touche
/// le sol. Le facteur `tf.translation.x * 0.3` décale les phases pour que
/// deux drops voisins ne pulsent pas à l'unisson.
fn drop_animate(
    time:      Res<Time>,
    mut items: Query<(&mut DroppedItem, &mut Transform)>,
) {
    let dt = time.delta_secs();
    let t  = time.elapsed_secs();

    for (mut item, mut tf) in &mut items {
        item.age += dt;

        tf.rotation = Quat::from_rotation_y(t * SPIN_SPEED + tf.translation.x * 0.5);

        if item.on_ground {
            let bob = (t * BOB_SPEED + tf.translation.x * 0.3).sin() * BOB_AMPLITUDE;
            tf.translation.y = item.base_y + bob;
        }
    }
}

/// Ramassage automatique dès que le joueur passe dans un rayon de
/// `PICKUP_RADIUS`. Envoie un `BlockCollected` et despawn l'entité.
fn pickup_drops(
    mut commands:  Commands,
    player_q:      Query<&Transform, With<Player>>,
    items:         Query<(Entity, &DroppedItem, &Transform), Without<Player>>,
    mut collected: EventWriter<BlockCollected>,
) {
    let Ok(player_tf) = player_q.get_single() else { return };
    let player_pos = player_tf.translation;

    for (entity, item, tf) in &items {
        let dist = player_pos.distance(tf.translation);
        if dist < PICKUP_RADIUS {
            collected.send(BlockCollected { block: item.block });
            commands.entity(entity).despawn();
        }
    }
}

/// Nettoyage : les drops qui traînent depuis trop longtemps disparaissent
/// pour éviter de laisser le monde plein de mini-cubes oubliés.
fn despawn_old_drops(
    mut commands: Commands,
    items:        Query<(Entity, &DroppedItem)>,
) {
    for (entity, item) in &items {
        if item.age >= DROP_LIFETIME {
            commands.entity(entity).despawn();
        }
    }
}
