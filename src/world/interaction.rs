//! Interaction avec les blocs du monde — poser, sélectionner, surligner.
//!
//! Trois systèmes se coordonnent :
//!   - `hotbar_select` : touches 1-9 et molette pour changer de slot.
//!   - `block_interact` : clic droit pour poser le bloc tenu (le clic gauche
//!     casse et passe par `world::breaking` qui gère la progression).
//!   - `update_block_outline` : déplace le cadre noir sur le bloc visé.
//!
//! Le raycast est un pas fixe (0.04) sur 6 blocs — assez fin pour ne rien
//! louper et assez court pour rester gratuit côté CPU.

use bevy::input::mouse::{MouseButton, MouseWheel};
use bevy::prelude::*;

use crate::world::chunk::{build_mesh, Block, BlockEdits, Chunk, ChunkManager, CHUNK_HEIGHT, CHUNK_SIZE};
use crate::player::PlayerCamera;
use crate::GameState;

/// Barre d'accès rapide : 9 slots, chacun pouvant contenir une pile de blocs.
#[derive(Resource)]
pub struct Hotbar {
    pub slots:    [Option<(Block, u32)>; 9],
    pub selected: usize,
}

impl Default for Hotbar {
    fn default() -> Self {
        Self {
            slots:    [None; 9],
            selected: 0,
        }
    }
}

/// Marque l'entité qui dessine le cadre noir autour du bloc visé.
#[derive(Component)]
pub struct BlockOutline;

pub struct BlockInteractionPlugin;

impl Plugin for BlockInteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Hotbar>()
           .add_systems(OnEnter(GameState::InGame), spawn_block_outline)
           .add_systems(
               Update,
               (hotbar_select, block_interact, update_block_outline)
                   .chain()
                   .run_if(in_state(GameState::InGame)),
           );
    }
}

/// Spawn du cadre noir (cube à peine plus grand que 1, semi-transparent).
/// Il vit tout le temps mais reste caché sous la carte (y = -1000) quand le
/// joueur ne vise rien.
fn spawn_block_outline(
    mut commands:  Commands,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    existing:      Query<Entity, With<BlockOutline>>,
) {
    if !existing.is_empty() { return; }
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::splat(1.02)))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.0, 0.0, 0.0, 0.40),
            alpha_mode: AlphaMode::Blend,
            unlit:      true,
            ..default()
        })),
        Transform::from_xyz(0.0, -1000.0, 0.0),
        BlockOutline,
    ));
}

/// Cherche le premier bloc solide sur la ligne de visée et replace le cadre
/// dessus. Si rien n'est visé, on planque le cadre sous la carte.
fn update_block_outline(
    camera_q:      Query<&GlobalTransform, With<PlayerCamera>>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
    mut outline_q: Query<&mut Transform, With<BlockOutline>>,
) {
    let Ok(cam) = camera_q.get_single() else { return };
    let Ok(mut outline_tf) = outline_q.get_single_mut() else { return };

    let origin    = cam.translation();
    let direction = cam.forward().as_vec3();

    const STEP:  f32 = 0.04;
    const RANGE: f32 = 6.0;
    let steps = (RANGE / STEP) as usize;

    let mut hit: Option<IVec3> = None;
    'ray: for i in 0..steps {
        let p  = origin + direction * (i as f32 * STEP);
        let bx = p.x.floor() as i32;
        let by = p.y.floor() as i32;
        let bz = p.z.floor() as i32;

        if by < 0 || by >= CHUNK_HEIGHT as i32 { continue; }

        let cx = bx.div_euclid(CHUNK_SIZE as i32);
        let cz = bz.div_euclid(CHUNK_SIZE as i32);
        let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;

        if let Some(&entity) = chunk_manager.loaded.get(&(cx, cz)) {
            if let Ok(chunk) = chunk_q.get(entity) {
                if chunk.blocks[lx][by as usize][lz].is_solid() {
                    hit = Some(IVec3::new(bx, by, bz));
                    break 'ray;
                }
            }
        }
    }

    match hit {
        Some(hp) => {
            outline_tf.translation = Vec3::new(
                hp.x as f32 + 0.5,
                hp.y as f32 + 0.5,
                hp.z as f32 + 0.5,
            );
        }
        None => {
            outline_tf.translation.y = -1000.0;
        }
    }
}

/// Change le slot actif de la hotbar. Les chiffres 1-9 font un accès direct,
/// la molette avance/recule d'un cran (sans wrap, intentionnellement).
fn hotbar_select(
    keys:        Res<ButtonInput<KeyCode>>,
    mut scroll:  EventReader<MouseWheel>,
    mut hotbar:  ResMut<Hotbar>,
) {
    let digit_keys = [
        KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3,
        KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6,
        KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9,
    ];
    for (i, key) in digit_keys.iter().enumerate() {
        if keys.just_pressed(*key) {
            hotbar.selected = i;
        }
    }

    for ev in scroll.read() {
        if ev.y > 0.0 {
            hotbar.selected = hotbar.selected.saturating_sub(1);
        } else if ev.y < 0.0 {
            hotbar.selected = (hotbar.selected + 1).min(8);
        }
    }
}

/// Pose un bloc avec le clic droit. On raycast comme pour l'outline, mais on
/// garde la *dernière* position vide traversée avant de toucher du solide :
/// c'est là qu'on veut placer le nouveau bloc (juste devant la surface visée).
fn block_interact(
    mouse:         Res<ButtonInput<MouseButton>>,
    camera_q:      Query<&GlobalTransform, With<PlayerCamera>>,
    mut hotbar:    ResMut<Hotbar>,
    chunk_manager: Res<ChunkManager>,
    mut chunk_q:   Query<(&mut Chunk, &Mesh3d)>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut edits:     ResMut<BlockEdits>,
) {
    let right = mouse.just_pressed(MouseButton::Right);
    if !right { return; }

    let cam_transform = match camera_q.get_single() { Ok(t) => t, Err(_) => return };

    let origin    = cam_transform.translation();
    let direction = cam_transform.forward().as_vec3();

    const STEP:  f32 = 0.04;
    const RANGE: f32 = 6.0;

    let steps = (RANGE / STEP) as usize;
    let mut prev_pos: Option<IVec3> = None;

    for i in 0..steps {
        let p   = origin + direction * (i as f32 * STEP);
        let bx  = p.x.floor() as i32;
        let by  = p.y.floor() as i32;
        let bz  = p.z.floor() as i32;

        if by < 0 || by >= CHUNK_HEIGHT as i32 { continue; }

        let cx = bx.div_euclid(CHUNK_SIZE as i32);
        let cz = bz.div_euclid(CHUNK_SIZE as i32);
        let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;

        if let Some(&entity) = chunk_manager.loaded.get(&(cx, cz)) {
            if let Ok((chunk, _)) = chunk_q.get(entity) {
                if chunk.blocks[lx][by as usize][lz].is_solid() {
                    break;
                }
            }
        }
        prev_pos = Some(IVec3::new(bx, by, bz));
    }

    let sel = hotbar.selected;
    let block_opt = hotbar.slots[sel].map(|(b, _)| b);
    if let (Some(pp), Some(block)) = (prev_pos, block_opt) {
        if set_block_and_rebuild(pp, block, &chunk_manager, &mut chunk_q, &mut meshes) {
            edits.record(pp, block);
            if let Some((_, ref mut count)) = hotbar.slots[sel] {
                *count -= 1;
                if *count == 0 { hotbar.slots[sel] = None; }
            }
        }
    }
}

/// Écrit un bloc dans le chunk cible et reconstruit son mesh. Renvoie `false`
/// si le chunk n'est pas chargé (rare mais possible aux frontières).
fn set_block_and_rebuild(
    world_pos:     IVec3,
    block:         Block,
    chunk_manager: &ChunkManager,
    chunk_q:       &mut Query<(&mut Chunk, &Mesh3d)>,
    meshes:        &mut Assets<Mesh>,
) -> bool {
    let bx = world_pos.x;
    let by = world_pos.y as usize;
    let bz = world_pos.z;

    let cx = bx.div_euclid(CHUNK_SIZE as i32);
    let cz = bz.div_euclid(CHUNK_SIZE as i32);
    let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
    let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;

    if let Some(&entity) = chunk_manager.loaded.get(&(cx, cz)) {
        if let Ok((mut chunk, mesh3d)) = chunk_q.get_mut(entity) {
            chunk.blocks[lx][by][lz] = block;
            let new_mesh = build_mesh(&chunk.blocks);
            if let Some(mesh) = meshes.get_mut(&mesh3d.0) {
                *mesh = new_mesh;
            }
            return true;
        }
    }
    false
}
