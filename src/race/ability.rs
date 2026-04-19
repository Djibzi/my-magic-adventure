//! Compétences raciales actives — touche F.
//!
//! Chaque race a une compétence unique avec son propre cooldown global. Les
//! effets réutilisent les ressources de buffs existantes (`CloakState`,
//! `WaterShieldState`, `FlashState`) et les helpers de modification de bloc
//! pour rester compacts.

use bevy::prelude::*;

use crate::GameState;
use crate::PlayerConfig;
use crate::magic::spells::{
    CloakState, FlashState, ProjectileKind, WaterShieldState,
};
use crate::player::{Player, PlayerCamera, PlayerStats};
use crate::race::Race;
use crate::world::chunk::{build_mesh, Block, BlockEdits, Chunk, ChunkManager, CHUNK_HEIGHT, CHUNK_SIZE};

/// Cooldown global de la compétence raciale (en secondes). 0 = prêt.
#[derive(Resource, Default)]
pub struct RaceAbilityCooldown(pub f32);

pub struct RaceAbilityPlugin;

impl Plugin for RaceAbilityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RaceAbilityCooldown>()
           .add_systems(
               Update,
               (tick_race_ab_cd, race_ability_cast)
                   .chain()
                   .run_if(in_state(GameState::InGame)),
           );
    }
}

fn tick_race_ab_cd(time: Res<Time>, mut cd: ResMut<RaceAbilityCooldown>) {
    if cd.0 > 0.0 { cd.0 = (cd.0 - time.delta_secs()).max(0.0); }
}

/// Gère le cast de la compétence raciale à l'appui de F. Le cooldown est
/// armé avant le match pour que chaque branche n'ait qu'à faire son effet.
fn race_ability_cast(
    keys:           Res<ButtonInput<KeyCode>>,
    cfg:            Res<PlayerConfig>,
    mut cd:         ResMut<RaceAbilityCooldown>,
    mut player_q:   Query<(&mut Player, &mut PlayerStats, &Transform)>,
    cam_q:          Query<&GlobalTransform, With<PlayerCamera>>,
    mut commands:   Commands,
    mut meshes:     ResMut<Assets<Mesh>>,
    mut materials:  ResMut<Assets<StandardMaterial>>,
    chunk_manager:  Res<ChunkManager>,
    mut chunk_q:    Query<(&mut Chunk, &Mesh3d)>,
    mut cloak:      ResMut<CloakState>,
    mut shield:     ResMut<WaterShieldState>,
    mut flash:      ResMut<FlashState>,
    mut edits:      ResMut<BlockEdits>,
) {
    if !keys.just_pressed(KeyCode::KeyF) { return; }
    if cd.0 > 0.0 { return; }

    let Ok((mut player, mut stats, ptf)) = player_q.get_single_mut() else { return };
    let Ok(cam_gt)                       = cam_q.get_single()        else { return };

    let origin    = cam_gt.translation();
    let direction = cam_gt.forward().as_vec3();
    // Direction horizontale normalisée — sert aux capacités qui doivent
    // partir devant le joueur indépendamment du regard vertical.
    let mut horiz = Vec3::new(direction.x, 0.0, direction.z);
    if horiz.length_squared() < 0.001 { horiz = Vec3::Z; }
    horiz = horiz.normalize();

    cd.0 = cfg.race.ability_cooldown();

    match cfg.race {
        Race::Sylvaris => {
            // Enracinement : gros soin immédiat + bouclier d'eau prolongé,
            // cercle d'herbe décoratif sous les pieds.
            stats.current_hp = (stats.current_hp + 35.0).min(stats.max_hp);
            shield.remaining = 8.0;
            let center = ptf.translation;
            for du in -1..=1i32 {
                for dv in -1..=1i32 {
                    let p = center + Vec3::new(du as f32, -1.0, dv as f32);
                    let bp = IVec3::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                    if set_block_at(bp, Block::Grass, &chunk_manager, &mut chunk_q, &mut meshes) {
                        edits.record(bp, Block::Grass);
                    }
                }
            }
        }
        Race::Ignaar => {
            // Éruption : sphère de rayon ~3 centrée sur le joueur, tout en
            // Air. Petit rebond vertical pour s'extraire du cratère.
            let center = ptf.translation;
            let cx0 = center.x.floor() as i32;
            let cy0 = center.y.floor() as i32;
            let cz0 = center.z.floor() as i32;
            for dx in -3..=3i32 {
                for dy in -1..=2i32 {
                    for dz in -3..=3i32 {
                        if dx*dx + dy*dy + dz*dz > 11 { continue; }
                        let bp = IVec3::new(cx0 + dx, cy0 + dy, cz0 + dz);
                        if set_block_at(bp, Block::Air, &chunk_manager, &mut chunk_q, &mut meshes) {
                            edits.record(bp, Block::Air);
                        }
                    }
                }
            }
            player.velocity.y = (player.velocity.y + 9.0).max(9.0);
        }
        Race::Aethyn => {
            // Bourrasque : dash aérien très puissant dans la direction du regard.
            player.velocity.x += direction.x * 30.0;
            player.velocity.z += direction.z * 30.0;
            player.velocity.y  = (player.velocity.y + direction.y * 12.0 + 8.0).max(8.0);
        }
        Race::Vorkai => {
            // Voile d'ombre prolongé + petit regain de mana.
            cloak.remaining = 12.0;
            stats.current_mana = (stats.current_mana + 20.0).min(stats.max_mana);
        }
        Race::Crysthari => {
            // Éclat prismatique : 3 projectiles lumineux en éventail (±0.25 rad).
            flash.remaining = 0.4;
            flash.duration  = 0.4;
            let right = Vec3::new(-horiz.z, 0.0, horiz.x);
            for offset in [-0.25_f32, 0.0, 0.25] {
                let dir = (direction + right * offset).normalize();
                spawn_proj_light(&mut commands, &mut meshes, &mut materials,
                    origin + dir * 1.2, dir * 30.0);
            }
        }
    }
}

fn spawn_proj_light(
    commands:  &mut Commands,
    meshes:    &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    pos:       Vec3,
    velocity:  Vec3,
) {
    use crate::magic::spells::Projectile;
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.20))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 1.0, 0.85),
            emissive:   LinearRgba::new(5.0, 5.0, 3.0, 1.0),
            ..default()
        })),
        Transform::from_translation(pos),
        Projectile { velocity, lifetime: 3.0, kind: ProjectileKind::Light },
    ));
}

/// Écrit un bloc dans le chunk correspondant et reconstruit son mesh.
/// Renvoie `false` si la coord Y est hors du chunk ou si le chunk n'est pas
/// chargé (rare mais possible en bord de carte).
fn set_block_at(
    world_pos:     IVec3,
    block:         Block,
    chunk_manager: &ChunkManager,
    chunk_q:       &mut Query<(&mut Chunk, &Mesh3d)>,
    meshes:        &mut Assets<Mesh>,
) -> bool {
    if world_pos.y < 0 || world_pos.y >= CHUNK_HEIGHT as i32 { return false; }
    let cx = world_pos.x.div_euclid(CHUNK_SIZE as i32);
    let cz = world_pos.z.div_euclid(CHUNK_SIZE as i32);
    let lx = world_pos.x.rem_euclid(CHUNK_SIZE as i32) as usize;
    let lz = world_pos.z.rem_euclid(CHUNK_SIZE as i32) as usize;
    let by = world_pos.y as usize;
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
