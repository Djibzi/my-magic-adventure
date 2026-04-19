//! Cassage progressif des blocs (clic gauche maintenu).
//!
//! Quand le joueur vise un bloc et tient le clic gauche, on accumule un
//! `progress` (0→1) à la vitesse déterminée par `break_time`. À 25/55/80 %
//! on affiche des fissures par étages, et à 100 % le bloc devient Air, drop
//! une mini-copie ramassable et crache une gerbe de poussière.
//!
//! Un `BreakingState` en ressource partage l'état entre frames — sinon le
//! moindre relâchement de bouton repartirait de zéro.

use bevy::prelude::*;

use crate::world::chunk::{build_mesh, Block, BlockEdits, Chunk, ChunkManager, CHUNK_HEIGHT, CHUNK_SIZE};
use crate::player::PlayerCamera;
use crate::world::drops::SpawnDroppedItem;
use crate::particles::SpawnParticleBurst;
use crate::GameState;

/// Durée de cassage en secondes selon le type de bloc. Les valeurs ne sont
/// pas vraiment calibrées, juste équilibrées à la main pour que la neige
/// parte vite et la pierre demande du temps.
fn break_time(block: Block) -> f32 {
    match block {
        Block::Snow   => 0.35,
        Block::Leaves => 0.30,
        Block::Ice    => 0.50,
        Block::Sand   => 0.55,
        Block::Grass  => 0.80,
        Block::Dirt   => 1.10,
        Block::Planks => 1.20,
        Block::Wood   => 1.90,
        Block::Stone  => 2.80,
        Block::Air    => f32::INFINITY,
    }
}

/// État courant du cassage. Tant que `target` est `Some`, on est en train
/// de casser ce bloc-là.
#[derive(Resource, Default)]
pub struct BreakingState {
    pub target:    Option<IVec3>,
    pub progress:  f32,
    block:         Option<Block>,
    overlay:       Option<Entity>,
    overlay_mat:   Option<Handle<StandardMaterial>>,
    cracks:        Vec<Entity>,
    last_stage:    usize,
}

#[derive(Component)] pub struct BreakingOverlay;
#[derive(Component)] pub struct BreakingCrack;

pub struct BlockBreakingPlugin;

impl Plugin for BlockBreakingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BreakingState>()
           .add_systems(
               Update,
               sys_block_breaking.run_if(in_state(GameState::InGame)),
           );
    }
}

/// Fissures placées sur les faces du bloc selon le stage d'avancement.
/// Chaque tuple est (offset depuis le centre du bloc, taille du petit cuboid
/// sombre). L'offset de 0.506 place la fissure juste devant la face pour
/// éviter le Z-fighting, et l'épaisseur de 0.012 la rend fine sans disparaître.
///
/// On ajoute des fissures sur de nouvelles faces à chaque stage pour que
/// le bloc ait l'air de plus en plus endommagé, pas juste "la même chose
/// en plus gros".
fn cracks_for_stage(stage: usize) -> Vec<(Vec3, Vec3)> {
    match stage {
        // Stage 1 (≥25 %) : premières fissures, face avant + dessus.
        1 => vec![
            (Vec3::new( 0.00,  0.06,  0.506), Vec3::new(0.60, 0.030, 0.012)),
            (Vec3::new(-0.17,  0.21,  0.506), Vec3::new(0.030, 0.40, 0.012)),
            (Vec3::new( 0.10,  0.506,  0.02), Vec3::new(0.52, 0.012, 0.030)),
        ],
        // Stage 2 (≥55 %) : face gauche + fissure basse sur l'avant.
        2 => vec![
            (Vec3::new(-0.506,  0.10,  0.04), Vec3::new(0.012, 0.030, 0.58)),
            (Vec3::new(-0.506, -0.14, -0.10), Vec3::new(0.012, 0.46, 0.030)),
            (Vec3::new( 0.16, -0.12,  0.506), Vec3::new(0.34, 0.030, 0.012)),
        ],
        // Stage 3 (≥80 %) : face arrière + droite + dessus croisé.
        3 => vec![
            (Vec3::new(-0.06,  0.02, -0.506), Vec3::new(0.65, 0.030, 0.012)),
            (Vec3::new( 0.12, -0.18, -0.506), Vec3::new(0.030, 0.44, 0.012)),
            (Vec3::new(-0.14,  0.506,  0.08), Vec3::new(0.030, 0.012, 0.50)),
            (Vec3::new( 0.506,  0.05, -0.04), Vec3::new(0.012, 0.030, 0.62)),
        ],
        _ => vec![],
    }
}

/// Boucle principale : raycast pour trouver le bloc visé, si on tient le clic
/// on avance la progression, on ajoute des fissures, et à 100 % on casse.
pub fn sys_block_breaking(
    mouse:         Res<ButtonInput<MouseButton>>,
    camera_q:      Query<&GlobalTransform, With<PlayerCamera>>,
    time:          Res<Time>,
    mut state:     ResMut<BreakingState>,
    mut commands:  Commands,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    chunk_manager: Res<ChunkManager>,
    mut chunk_q:   Query<(&mut Chunk, &Mesh3d)>,
    mut edits:     ResMut<BlockEdits>,
    mut drop_evts: EventWriter<SpawnDroppedItem>,
    mut bursts:    EventWriter<SpawnParticleBurst>,
) {
    let held = mouse.pressed(MouseButton::Left);

    // Raycast : on cherche le premier bloc solide dans la ligne de visée.
    let target: Option<(IVec3, Block)> = if held {
        let Ok(cam) = camera_q.get_single() else { return };
        let origin    = cam.translation();
        let direction = cam.forward().as_vec3();
        const STEP: f32 = 0.04;
        const RANGE: f32 = 6.0;
        let mut found = None;
        'ray: for i in 0..(RANGE / STEP) as usize {
            let p  = origin + direction * (i as f32 * STEP);
            let bx = p.x.floor() as i32;
            let by = p.y.floor() as i32;
            let bz = p.z.floor() as i32;
            if by < 0 || by >= CHUNK_HEIGHT as i32 { continue; }
            let cx = bx.div_euclid(CHUNK_SIZE as i32);
            let cz = bz.div_euclid(CHUNK_SIZE as i32);
            let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
            let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;
            if let Some(&ent) = chunk_manager.loaded.get(&(cx, cz)) {
                if let Ok((chunk, _)) = chunk_q.get(ent) {
                    let b = chunk.blocks[lx][by as usize][lz];
                    if b.is_solid() {
                        found = Some((IVec3::new(bx, by, bz), b));
                        break 'ray;
                    }
                }
            }
        }
        found
    } else {
        None
    };

    // Bouton relâché ou plus rien en vue : on annule tout proprement.
    let Some((pos, block)) = target else {
        reset(&mut state, &mut commands);
        return;
    };

    // Changement de cible : on repart à zéro (pas de carry-over de progress).
    if state.target != Some(pos) {
        reset(&mut state, &mut commands);
        state.target = Some(pos);
        state.block  = Some(block);

        // Overlay : un cube à peine plus gros que le bloc, opacité initiale
        // à 0, qui assombrit progressivement la face du bloc.
        let center = block_center(pos);
        let mat_handle = materials.add(StandardMaterial {
            base_color: Color::srgba(0.0, 0.0, 0.0, 0.0),
            alpha_mode: AlphaMode::Blend,
            unlit:      true,
            cull_mode:  None,
            ..default()
        });
        state.overlay_mat = Some(mat_handle.clone());
        let ent = commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(1.018, 1.018, 1.018))),
            MeshMaterial3d(mat_handle),
            Transform::from_translation(center),
            BreakingOverlay,
        )).id();
        state.overlay = Some(ent);
    }

    // On avance la jauge. `progress` dépend uniquement du delta-time et de
    // la dureté du bloc, pas du framerate.
    let dt = time.delta_secs();
    state.progress = (state.progress + dt / break_time(block)).min(1.0);

    // L'overlay assombrit progressivement le bloc (alpha 0 → 0.55).
    if let Some(ref handle) = state.overlay_mat.clone() {
        if let Some(mat) = materials.get_mut(handle) {
            mat.base_color = Color::srgba(0.0, 0.0, 0.0, state.progress * 0.55);
        }
    }

    // Seuils fixes : on ne régresse pas de stage (si le joueur relâche puis
    // re-presse sans changer de bloc on repartirait avec les fissures déjà
    // visibles, ce qui fait un peu bizarre mais reste cohérent côté code).
    let stage = if      state.progress >= 0.80 { 3 }
                else if state.progress >= 0.55 { 2 }
                else if state.progress >= 0.25 { 1 }
                else                           { 0 };

    if stage > state.last_stage {
        let center = block_center(pos);
        let crack_mat = materials.add(StandardMaterial {
            base_color: Color::srgba(0.04, 0.02, 0.02, 0.92),
            alpha_mode: AlphaMode::Blend,
            unlit:      true,
            ..default()
        });
        for new_s in (state.last_stage + 1)..=stage {
            for (offset, size) in cracks_for_stage(new_s) {
                let e = commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
                    MeshMaterial3d(crack_mat.clone()),
                    Transform::from_translation(center + offset),
                    BreakingCrack,
                )).id();
                state.cracks.push(e);
            }
        }
        state.last_stage = stage;
    }

    // Bloc cassé : on le remplace par Air, enregistre l'édit, spawn le drop
    // et l'éclatement de poussière, puis reset de tout l'état.
    if state.progress >= 1.0 {
        let bx = pos.x;
        let by = pos.y as usize;
        let bz = pos.z;
        let cx = bx.div_euclid(CHUNK_SIZE as i32);
        let cz = bz.div_euclid(CHUNK_SIZE as i32);
        let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;

        if let Some(&ent) = chunk_manager.loaded.get(&(cx, cz)) {
            if let Ok((mut chunk, mesh3d)) = chunk_q.get_mut(ent) {
                let broken = chunk.blocks[lx][by][lz];
                chunk.blocks[lx][by][lz] = Block::Air;
                let new_mesh = build_mesh(&chunk.blocks);
                if let Some(m) = meshes.get_mut(&mesh3d.0) {
                    *m = new_mesh;
                }
                edits.record(pos, Block::Air);
                if broken != Block::Air {
                    drop_evts.send(SpawnDroppedItem {
                        block:    broken,
                        position: block_center(pos),
                    });
                    bursts.send(SpawnParticleBurst {
                        position: block_center(pos),
                        color:    block_dust_color(broken),
                        count:    14,
                        speed:    3.0,
                        lifetime: 0.55,
                    });
                }
            }
        }
        reset(&mut state, &mut commands);
    }
}

/// Couleur de poussière approximative de chaque bloc — utilisée uniquement
/// pour la gerbe de particules à la destruction.
fn block_dust_color(block: Block) -> Color {
    match block {
        Block::Grass  => Color::srgb(0.36, 0.62, 0.28),
        Block::Dirt   => Color::srgb(0.46, 0.32, 0.20),
        Block::Stone  => Color::srgb(0.55, 0.55, 0.58),
        Block::Sand   => Color::srgb(0.92, 0.84, 0.55),
        Block::Snow   => Color::srgb(0.95, 0.97, 1.00),
        Block::Wood   => Color::srgb(0.45, 0.30, 0.18),
        Block::Leaves => Color::srgb(0.28, 0.55, 0.22),
        Block::Planks => Color::srgb(0.78, 0.60, 0.36),
        Block::Ice    => Color::srgb(0.70, 0.88, 1.00),
        Block::Air    => Color::WHITE,
    }
}

fn block_center(pos: IVec3) -> Vec3 {
    Vec3::new(pos.x as f32 + 0.5, pos.y as f32 + 0.5, pos.z as f32 + 0.5)
}

/// Remet tout à zéro : despawn de l'overlay, des fissures, état vide.
fn reset(state: &mut BreakingState, commands: &mut Commands) {
    if let Some(e) = state.overlay.take() {
        commands.entity(e).despawn();
    }
    for e in state.cracks.drain(..) {
        commands.entity(e).despawn();
    }
    state.target      = None;
    state.block       = None;
    state.progress    = 0.0;
    state.overlay_mat = None;
    state.last_stage  = 0;
}
