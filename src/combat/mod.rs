//! Mobs et animaux.
//!
//! Trois catégories d'entités vivantes, toutes animées et soumises à la
//! gravité voxel du monde :
//!
//! - **Mobs hostiles** : Slime (petit cube vert qui erre), Wolf (charge le
//!   joueur), Humanoid (corps articulé identique au joueur, une par race).
//! - **Animaux passifs** : Cow, Chicken, Pig — errent paisiblement, peuvent
//!   être frappés pour drop de la viande côté `world::drops`.
//!
//! Chaque entité a son propre composant (`Mob` ou `Animal`), et sa gravité
//! est résolue contre le terrain réel (pas un plancher fixe) via
//! `is_block_solid`. Les humanoids ont une origine aux pieds (half_height=0)
//! tandis que slime/wolf ont une origine au centre — d'où la distinction via
//! `half_height` dans les checks de collision.

use bevy::prelude::*;
use rand::Rng;
use std::f32::consts::TAU;

use bevy::input::mouse::MouseButton;

use crate::GameState;
use crate::magic::spells::{Projectile, ProjectileKind};
use crate::particles::SpawnParticleBurst;
use crate::player::{Player, PlayerCamera, PlayerStats};
use crate::player::animation::PlayerAnimState;
use crate::world::chunk::{Chunk, ChunkManager, CHUNK_HEIGHT, CHUNK_SIZE};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MobKind {
    Slime,
    Wolf,
    Humanoid(HumanoidRace),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HumanoidRace {
    Sylvaris,
    Ignaar,
    Aethyn,
    Vorkai,
    Crysthari,
}

impl HumanoidRace {
    fn random() -> Self {
        let mut rng = rand::thread_rng();
        match rng.gen_range(0..5u32) {
            0 => Self::Sylvaris,
            1 => Self::Ignaar,
            2 => Self::Aethyn,
            3 => Self::Vorkai,
            _ => Self::Crysthari,
        }
    }

    fn body_color(self) -> Color {
        match self {
            Self::Sylvaris  => Color::srgb(0.15, 0.45, 0.15),
            Self::Ignaar    => Color::srgb(0.55, 0.18, 0.06),
            Self::Aethyn    => Color::srgb(0.18, 0.32, 0.60),
            Self::Vorkai    => Color::srgb(0.32, 0.08, 0.48),
            Self::Crysthari => Color::srgb(0.45, 0.45, 0.70),
        }
    }

    fn skin_color(self) -> Color {
        match self {
            Self::Sylvaris  => Color::srgb(0.72, 0.85, 0.55),
            Self::Ignaar    => Color::srgb(0.80, 0.38, 0.18),
            Self::Aethyn    => Color::srgb(0.78, 0.82, 0.95),
            Self::Vorkai    => Color::srgb(0.22, 0.18, 0.28),
            Self::Crysthari => Color::srgb(0.88, 0.88, 0.98),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AnimalKind {
    Cow,
    Chicken,
    Pig,
}

#[derive(Component)]
pub struct Mob {
    pub kind:      MobKind,
    pub hp:        f32,
    pub speed:     f32,
    pub damage:    f32,
    pub wander:    Vec3,
    pub wander_t:  f32,
    pub hit_cd:    f32,
    pub velocity_y: f32,
    pub on_ground:  bool,
    /// Phase accumulée pour l'animation de marche (sinusoïde).
    pub walk_phase: f32,
    /// Position X/Z du frame précédent, pour estimer la vitesse réelle
    /// (indépendamment du `speed` théorique — utile quand le mob est bloqué).
    pub prev_x: f32,
    pub prev_z: f32,
    /// Distance de l'origine de l'entité au bas du corps. Slime/Wolf ont une
    /// origine centrée (half_height > 0) ; les humanoids ont l'origine aux
    /// pieds donc half_height = 0.
    pub half_height: f32,
}

#[derive(Component)]
pub struct Animal {
    pub kind:      AnimalKind,
    pub hp:        f32,
    pub wander:    Vec3,
    pub wander_t:  f32,
    pub velocity_y: f32,
    pub on_ground:  bool,
    pub walk_phase: f32,
    pub prev_x: f32,
    pub prev_z: f32,
    /// Demi-hauteur du corps (origine → haut du corps), sert aux checks de
    /// collision sur le dessus.
    pub half_height: f32,
    /// Distance de l'origine au bas des pieds — sert au snap au sol. Le
    /// corps et les pattes ne sont pas centrés à la même hauteur, donc on
    /// garde les deux offsets.
    pub foot_offset: f32,
}

/// Marqueur sur chaque cuboïde du corps d'un humanoid.
#[derive(Component)]
struct MobBodyPart;

/// Identifie quelle partie du corps d'un humanoid pour animer le bon membre.
#[derive(Component, Clone, Copy)]
enum MobPartTag {
    Head,
    Torso,
    ArmLeft,
    ArmRight,
    LegLeft,
    LegRight,
}

/// Patte d'animal animée ; `phase_offset` décale la sinusoïde pour que les
/// pattes diagonales bougent en phase (0/3 ensemble, 1/2 ensemble).
#[derive(Component)]
struct AnimalLeg {
    phase_offset: f32,
}

/// Cuboïde du corps d'un animal (rebond vertical léger à la marche).
#[derive(Component)]
struct AnimalBody;

/// Cuboïde de la tête d'un animal. On conserve la Y de spawn pour pouvoir
/// ajouter un bounce par-dessus sans dériver.
#[derive(Component)]
struct AnimalHead {
    base_y: f32,
}

/// Portée vers l'avant de la tête selon l'espèce — sert au check de collision
/// du museau contre un mur (sinon la tête traverse visiblement les blocs).
fn head_reach(kind: AnimalKind) -> f32 {
    match kind {
        AnimalKind::Cow     => 1.05,
        AnimalKind::Chicken => 0.40,
        AnimalKind::Pig     => 0.80,
    }
}

fn head_y_center(kind: AnimalKind) -> f32 {
    match kind {
        AnimalKind::Cow     => 0.15,
        AnimalKind::Chicken => 0.25,
        AnimalKind::Pig     => 0.0,
    }
}

#[derive(Resource)]
pub struct MobSpawnTimer {
    pub mob_timer:    f32,
    pub animal_timer: f32,
}

impl Default for MobSpawnTimer {
    fn default() -> Self { Self { mob_timer: 4.0, animal_timer: 6.0 } }
}

#[derive(Resource, Default)]
pub struct MeleeState {
    pub cooldown: f32,
}

pub struct MobPlugin;

impl Plugin for MobPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MobSpawnTimer>()
           .init_resource::<MeleeState>()
           .add_systems(
               Update,
               (
                   spawn_mobs,
                   spawn_animals,
                   mob_gravity,
                   animal_gravity,
                   mob_ai,
                   animal_ai,
                   mob_walk_animation,
                   mob_slime_animation,
                   mob_wolf_animation,
                   animal_walk_animation,
                   player_melee_attack,
                   mob_contact_damage,
                   mob_hit_by_projectile,
                   despawn_dead_mobs,
                   despawn_far_entities,
               )
               .chain()
               .run_if(in_state(GameState::InGame)),
           );
    }
}

/// Cherche la hauteur du sol à (x, z) en scannant la colonne du chunk du
/// haut vers le bas. Renvoie `None` si le chunk n'est pas chargé — le caller
/// doit gérer ce cas (skip le spawn, retry plus tard).
fn surface_y_at(x: f32, z: f32, chunk_manager: &ChunkManager, chunk_q: &Query<&Chunk>) -> Option<f32> {
    let bx = x.floor() as i32;
    let bz = z.floor() as i32;
    let cx = bx.div_euclid(CHUNK_SIZE as i32);
    let cz = bz.div_euclid(CHUNK_SIZE as i32);
    let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
    let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;
    let chunk_ent = chunk_manager.loaded.get(&(cx, cz))?;
    let chunk = chunk_q.get(*chunk_ent).ok()?;
    for y in (0..CHUNK_HEIGHT).rev() {
        if chunk.blocks[lx][y][lz].is_solid() {
            return Some(y as f32 + 1.0);
        }
    }
    Some(1.0)
}

fn is_block_solid(x: f32, y: f32, z: f32, chunk_manager: &ChunkManager, chunk_q: &Query<&Chunk>) -> bool {
    let bx = x.floor() as i32;
    let by = y.floor() as i32;
    let bz = z.floor() as i32;
    if by < 0 { return true; }
    if by >= CHUNK_HEIGHT as i32 { return false; }
    let cx = bx.div_euclid(CHUNK_SIZE as i32);
    let cz = bz.div_euclid(CHUNK_SIZE as i32);
    let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
    let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;
    if let Some(&entity) = chunk_manager.loaded.get(&(cx, cz)) {
        if let Ok(chunk) = chunk_q.get(entity) {
            return chunk.blocks[lx][by as usize][lz].is_solid();
        }
    }
    false
}

fn darken(c: Color, factor: f32) -> Color {
    let l = c.to_linear();
    Color::linear_rgb(l.red * factor, l.green * factor, l.blue * factor)
}

/// Spawn aléatoire de mobs autour du joueur : un toutes les 8s tant qu'il y
/// en a moins de 12 dans le monde. Le type est tiré au sort (30% slime,
/// 25% wolf, 45% humanoid d'une race aléatoire).
fn spawn_mobs(
    mut commands:  Commands,
    mut timer:     ResMut<MobSpawnTimer>,
    time:          Res<Time>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    player_q:      Query<&Transform, With<Player>>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
    mob_q:         Query<&Mob>,
) {
    timer.mob_timer -= time.delta_secs();
    if timer.mob_timer > 0.0 { return; }
    timer.mob_timer = 8.0;

    if mob_q.iter().count() >= 12 { return; }

    let Ok(ptf) = player_q.get_single() else { return };

    let mut rng = rand::thread_rng();
    let angle = rng.gen_range(0.0..TAU);
    let dist  = rng.gen_range(12.0..22.0);
    let x = ptf.translation.x + angle.cos() * dist;
    let z = ptf.translation.z + angle.sin() * dist;

    let Some(sy) = surface_y_at(x, z, &chunk_manager, &chunk_q) else { return };

    let roll = rng.gen_range(0.0..1.0f32);
    let kind = if roll < 0.30 {
        MobKind::Slime
    } else if roll < 0.55 {
        MobKind::Wolf
    } else {
        MobKind::Humanoid(HumanoidRace::random())
    };

    match kind {
        MobKind::Slime => {
            let color = Color::srgb(0.30, 0.85, 0.40);
            let scale = Vec3::splat(0.85);
            let half_h = scale.y * 0.5;
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::from_size(Vec3::ONE))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: color,
                    perceptual_roughness: 0.85,
                    ..default()
                })),
                Transform::from_xyz(x, sy + half_h, z).with_scale(scale),
                Mob {
                    kind, hp: 20.0, speed: 1.2, damage: 4.0,
                    wander: Vec3::ZERO, wander_t: 0.0, hit_cd: 0.0,
                    velocity_y: 0.0, on_ground: true,
                    walk_phase: 0.0, prev_x: x, prev_z: z,
                    half_height: half_h,
                },
            ));
        }
        MobKind::Wolf => {
            let color = Color::srgb(0.55, 0.45, 0.35);
            let scale = Vec3::new(0.7, 0.7, 1.1);
            let half_h = scale.y * 0.5;
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::from_size(Vec3::ONE))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: color,
                    perceptual_roughness: 0.85,
                    ..default()
                })),
                Transform::from_xyz(x, sy + half_h, z).with_scale(scale),
                Mob {
                    kind, hp: 35.0, speed: 3.5, damage: 9.0,
                    wander: Vec3::ZERO, wander_t: 0.0, hit_cd: 0.0,
                    velocity_y: 0.0, on_ground: true,
                    walk_phase: 0.0, prev_x: x, prev_z: z,
                    half_height: half_h,
                },
            ));
        }
        MobKind::Humanoid(hrace) => {
            spawn_humanoid_mob(&mut commands, &mut meshes, &mut materials, x, sy, z, hrace);
        }
    }
}

/// Spawn un humanoid avec les mêmes proportions que le joueur, mais animé
/// par `mob_walk_animation` plutôt que par `player::animation`. Ses stats
/// varient selon la race (Ignaar = tank, Aethyn = rapide mais fragile…).
fn spawn_humanoid_mob(
    commands:  &mut Commands,
    meshes:    &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    x: f32, sy: f32, z: f32,
    hrace: HumanoidRace,
) {
    let body_color  = hrace.body_color();
    let skin_color  = hrace.skin_color();
    let pants_color = darken(body_color, 0.55);

    let skin_mat  = materials.add(StandardMaterial { base_color: skin_color,  perceptual_roughness: 0.9, ..default() });
    let body_mat  = materials.add(StandardMaterial { base_color: body_color,  perceptual_roughness: 0.9, ..default() });
    let pants_mat = materials.add(StandardMaterial { base_color: pants_color, perceptual_roughness: 0.9, ..default() });

    let head_mesh = meshes.add(Cuboid::new(0.50, 0.50, 0.50));
    let body_mesh = meshes.add(Cuboid::new(0.38, 0.75, 0.25));
    let arm_mesh  = meshes.add(Cuboid::new(0.20, 0.65, 0.20));
    let leg_mesh  = meshes.add(Cuboid::new(0.18, 0.65, 0.22));

    let mut rng = rand::thread_rng();
    let (hp, speed, damage) = match hrace {
        HumanoidRace::Ignaar    => (50.0, 2.8, 12.0),
        HumanoidRace::Vorkai    => (40.0, 3.5, 10.0),
        HumanoidRace::Aethyn    => (35.0, 4.0,  8.0),
        HumanoidRace::Sylvaris  => (45.0, 2.5, 10.0),
        HumanoidRace::Crysthari => (38.0, 3.0,  9.0),
    };

    let parent = commands.spawn((
        Transform::from_xyz(x, sy, z),
        Visibility::default(),
        Mob {
            kind: MobKind::Humanoid(hrace),
            hp, speed, damage,
            wander: Vec3::ZERO,
            wander_t: rng.gen_range(0.0..3.0),
            hit_cd: 0.0,
            velocity_y: 0.0,
            on_ground: true,
            walk_phase: 0.0, prev_x: x, prev_z: z,
            half_height: 0.0,
        },
    )).id();

    let parts: &[(Handle<Mesh>, Handle<StandardMaterial>, Vec3, MobPartTag)] = &[
        (head_mesh,          skin_mat.clone(),  Vec3::new( 0.00, 1.55,  0.00), MobPartTag::Head),
        (body_mesh,          body_mat.clone(),  Vec3::new( 0.00, 0.975, 0.00), MobPartTag::Torso),
        (arm_mesh.clone(),   skin_mat.clone(),  Vec3::new(-0.30, 0.975, 0.00), MobPartTag::ArmLeft),
        (arm_mesh,           skin_mat,          Vec3::new( 0.30, 0.975, 0.00), MobPartTag::ArmRight),
        (leg_mesh.clone(),   pants_mat.clone(), Vec3::new(-0.10, 0.325, 0.00), MobPartTag::LegLeft),
        (leg_mesh,           pants_mat,         Vec3::new( 0.10, 0.325, 0.00), MobPartTag::LegRight),
    ];

    for (mesh, mat, pos, tag) in parts {
        let part = commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(*pos),
            MobBodyPart,
            *tag,
        )).id();
        commands.entity(parent).add_child(part);
    }
}

/// Spawn d'animaux passifs. Cadence plus lente que les mobs (10s) et cap à
/// 10 entités simultanées.
fn spawn_animals(
    mut commands:  Commands,
    mut timer:     ResMut<MobSpawnTimer>,
    time:          Res<Time>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    player_q:      Query<&Transform, With<Player>>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
    animal_q:      Query<&Animal>,
) {
    timer.animal_timer -= time.delta_secs();
    if timer.animal_timer > 0.0 { return; }
    timer.animal_timer = 10.0;

    if animal_q.iter().count() >= 10 { return; }

    let Ok(ptf) = player_q.get_single() else { return };

    let mut rng = rand::thread_rng();
    let angle = rng.gen_range(0.0..TAU);
    let dist  = rng.gen_range(10.0..20.0);
    let x = ptf.translation.x + angle.cos() * dist;
    let z = ptf.translation.z + angle.sin() * dist;

    let Some(sy) = surface_y_at(x, z, &chunk_manager, &chunk_q) else { return };

    let kind: AnimalKind = match rng.gen_range(0..3u32) {
        0 => AnimalKind::Cow,
        1 => AnimalKind::Chicken,
        _ => AnimalKind::Pig,
    };

    let (color, scale, hp) = match kind {
        AnimalKind::Cow     => (Color::srgb(0.90, 0.88, 0.85), Vec3::new(0.9, 0.8, 1.3), 15.0),
        AnimalKind::Chicken => (Color::srgb(0.95, 0.92, 0.80), Vec3::new(0.4, 0.5, 0.4), 6.0),
        AnimalKind::Pig     => (Color::srgb(0.95, 0.70, 0.65), Vec3::new(0.7, 0.6, 1.0), 12.0),
    };

    let half_h = scale.y * 0.5;
    // Longueur de patte par espèce — doit coller à ce qu'on spawn plus bas,
    // sinon les pieds s'enfoncent dans le sol ou flottent au-dessus.
    let leg_len = match kind {
        AnimalKind::Cow     => 0.5,
        AnimalKind::Pig     => 0.35,
        AnimalKind::Chicken => 0.25,
    };
    // Le -0.05 vient du fait que les pattes chevauchent légèrement le corps.
    let foot_off = half_h + leg_len - 0.05;
    let parent = commands.spawn((
        Transform::from_xyz(x, sy + foot_off, z),
        Visibility::default(),
        Animal {
            kind, hp,
            wander: Vec3::ZERO,
            wander_t: 0.0,
            velocity_y: 0.0,
            on_ground: true,
            walk_phase: 0.0, prev_x: x, prev_z: z,
            half_height: half_h,
            foot_offset: foot_off,
        },
    )).id();

    let body_part = commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::ONE))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.85,
            ..default()
        })),
        Transform::from_scale(scale),
        AnimalBody,
    )).id();
    commands.entity(parent).add_child(body_part);

    let head_color = match kind {
        AnimalKind::Cow     => Color::srgb(0.50, 0.35, 0.20),
        AnimalKind::Chicken => Color::srgb(0.95, 0.30, 0.15), // crête rouge
        AnimalKind::Pig     => Color::srgb(0.90, 0.60, 0.55),
    };
    let head_scale = match kind {
        AnimalKind::Cow     => Vec3::new(0.5, 0.5, 0.4),
        AnimalKind::Chicken => Vec3::new(0.25, 0.3, 0.2),
        AnimalKind::Pig     => Vec3::new(0.4, 0.35, 0.3),
    };
    let head_offset = match kind {
        AnimalKind::Cow     => Vec3::new(0.0, 0.15, 0.85),
        AnimalKind::Chicken => Vec3::new(0.0, 0.25, 0.30),
        AnimalKind::Pig     => Vec3::new(0.0, 0.0, 0.65),
    };
    let head_part = commands.spawn((
        Mesh3d(meshes.add(Cuboid::from_size(Vec3::ONE))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: head_color,
            perceptual_roughness: 0.85,
            ..default()
        })),
        Transform::from_translation(head_offset).with_scale(head_scale),
        AnimalHead { base_y: head_offset.y },
    )).id();
    commands.entity(parent).add_child(head_part);

    let leg_color = match kind {
        AnimalKind::Cow     => Color::srgb(0.50, 0.35, 0.20),
        AnimalKind::Chicken => Color::srgb(0.90, 0.75, 0.20),
        AnimalKind::Pig     => Color::srgb(0.85, 0.55, 0.50),
    };
    let leg_mat = materials.add(StandardMaterial {
        base_color: leg_color,
        perceptual_roughness: 0.85,
        ..default()
    });

    match kind {
        AnimalKind::Cow | AnimalKind::Pig => {
            let lw = if kind == AnimalKind::Cow { 0.15 } else { 0.12 };
            let lh = if kind == AnimalKind::Cow { 0.5  } else { 0.35 };
            let sx = if kind == AnimalKind::Cow { 0.25 } else { 0.20 };
            let sz = if kind == AnimalKind::Cow { 0.40 } else { 0.30 };
            let ly = -(scale.y * 0.5) - lh * 0.5 + 0.05;
            // Phases diagonales : pattes 0&3 en phase, 1&2 en phase opposée
            // — donne la démarche naturelle à 4 pattes.
            let offsets = [0.0, 0.5, 0.5, 0.0];
            for (i, &(dx, dz)) in [(-sx, -sz), (sx, -sz), (-sx, sz), (sx, sz)].iter().enumerate() {
                let leg = commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(lw, lh, lw))),
                    MeshMaterial3d(leg_mat.clone()),
                    Transform::from_xyz(dx, ly, dz),
                    AnimalLeg { phase_offset: offsets[i] },
                )).id();
                commands.entity(parent).add_child(leg);
            }
        }
        AnimalKind::Chicken => {
            let lh = 0.25;
            let ly = -(scale.y * 0.5) - lh * 0.5 + 0.05;
            for (i, dx) in [-0.08f32, 0.08].iter().enumerate() {
                let leg = commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(0.06, lh, 0.06))),
                    MeshMaterial3d(leg_mat.clone()),
                    Transform::from_xyz(*dx, ly, 0.0),
                    AnimalLeg { phase_offset: if i == 0 { 0.0 } else { 0.5 } },
                )).id();
                commands.entity(parent).add_child(leg);
            }
        }
    }
}

const GRAVITY: f32 = 22.0;

fn mob_gravity(
    time:          Res<Time>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
    mut mob_q:     Query<(&mut Transform, &mut Mob)>,
) {
    let dt = time.delta_secs();
    for (mut tf, mut mob) in mob_q.iter_mut() {
        if !mob.on_ground {
            mob.velocity_y -= GRAVITY * dt;
        }
        tf.translation.y += mob.velocity_y * dt;

        // Bas du corps selon l'ancrage : slime/wolf centrés (origin - half),
        // humanoid ancré aux pieds (origin directement).
        let bottom_y = tf.translation.y - mob.half_height;
        let check_y  = bottom_y - 0.05;
        if mob.velocity_y <= 0.0 && is_block_solid(tf.translation.x, check_y, tf.translation.z, &chunk_manager, &chunk_q) {
            tf.translation.y = check_y.floor() + 1.0 + mob.half_height + 0.01;
            mob.velocity_y = 0.0;
            mob.on_ground = true;
        } else if mob.velocity_y <= 0.0 {
            mob.on_ground = false;
        }

        if tf.translation.y < 0.0 {
            tf.translation.y = mob.half_height + 0.5;
            mob.velocity_y = 0.0;
            mob.on_ground = true;
        }
    }
}

fn animal_gravity(
    time:          Res<Time>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
    mut animal_q:  Query<(&mut Transform, &mut Animal)>,
) {
    let dt = time.delta_secs();
    for (mut tf, mut animal) in animal_q.iter_mut() {
        if !animal.on_ground {
            animal.velocity_y -= GRAVITY * dt;
        }
        tf.translation.y += animal.velocity_y * dt;

        let bottom_y = tf.translation.y - animal.foot_offset;
        let check_y = bottom_y - 0.05;
        if animal.velocity_y <= 0.0 && is_block_solid(tf.translation.x, check_y, tf.translation.z, &chunk_manager, &chunk_q) {
            tf.translation.y = check_y.floor() + 1.0 + animal.foot_offset + 0.01;
            animal.velocity_y = 0.0;
            animal.on_ground = true;
        } else if animal.velocity_y <= 0.0 {
            animal.on_ground = false;
        }

        if tf.translation.y < 0.0 {
            tf.translation.y = animal.foot_offset + 0.5;
            animal.velocity_y = 0.0;
            animal.on_ground = true;
        }
    }
}

/// IA des mobs : Slime erre, Wolf et Humanoid chassent le joueur dans un
/// rayon de 25m puis retombent en errance s'il est trop loin. Step-up
/// automatique sur les blocs d'un cube de haut pour ne pas rester bloqué.
fn mob_ai(
    mut mob_q:     Query<(&mut Transform, &mut Mob)>,
    player_q:      Query<&Transform, (With<Player>, Without<Mob>)>,
    time:          Res<Time>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
) {
    let Ok(ptf) = player_q.get_single() else { return };
    let dt = time.delta_secs();
    let mut rng = rand::thread_rng();

    for (mut tf, mut mob) in mob_q.iter_mut() {
        mob.hit_cd = (mob.hit_cd - dt).max(0.0);

        let to_player = ptf.translation - tf.translation;
        let dist = to_player.length();

        let dir = match mob.kind {
            MobKind::Slime => {
                mob.wander_t -= dt;
                if mob.wander_t <= 0.0 {
                    let a = rng.gen_range(0.0..TAU);
                    mob.wander = Vec3::new(a.cos(), 0.0, a.sin());
                    mob.wander_t = rng.gen_range(2.0..4.0);
                }
                mob.wander
            }
            MobKind::Wolf | MobKind::Humanoid(_) => {
                if dist < 25.0 && dist > 1.0 {
                    let flat = Vec3::new(to_player.x, 0.0, to_player.z);
                    let chase_dir = flat.normalize_or_zero();
                    // Oriente le mob vers le joueur pendant la chasse pour
                    // que son animation de marche parte dans le bon sens.
                    if chase_dir.length_squared() > 0.01 {
                        let angle = chase_dir.x.atan2(chase_dir.z);
                        tf.rotation = Quat::from_rotation_y(angle);
                    }
                    chase_dir
                } else {
                    mob.wander_t -= dt;
                    if mob.wander_t <= 0.0 {
                        let a = rng.gen_range(0.0..TAU);
                        mob.wander = Vec3::new(a.cos(), 0.0, a.sin());
                        mob.wander_t = rng.gen_range(2.0..4.0);
                    }
                    mob.wander
                }
            }
        };

        // Collision axe par axe avec step-up automatique. Pour les humanoids
        // (half_height=0), les pieds sont à l'origine et la tête à +1.0 ;
        // pour slime/wolf (origine centrée), pieds = origin - half, tête = origin + half.
        let new_x = tf.translation.x + dir.x * mob.speed * dt;
        let new_z = tf.translation.z + dir.z * mob.speed * dt;
        let feet_y = tf.translation.y - mob.half_height;
        let head_y = if mob.half_height > 0.0 {
            tf.translation.y + mob.half_height
        } else {
            tf.translation.y + 1.0
        };

        let blocked_x_feet = is_block_solid(new_x, feet_y, tf.translation.z, &chunk_manager, &chunk_q);
        let blocked_x_head = is_block_solid(new_x, head_y, tf.translation.z, &chunk_manager, &chunk_q);

        if !blocked_x_feet && !blocked_x_head {
            tf.translation.x = new_x;
        } else if mob.on_ground && blocked_x_feet && !blocked_x_head
            && !is_block_solid(new_x, feet_y + 1.0, tf.translation.z, &chunk_manager, &chunk_q)
        {
            // Step-up : bloc bas + espace libre au-dessus → saut auto.
            tf.translation.x = new_x;
            mob.velocity_y = 8.0;
            mob.on_ground = false;
        } else if !mob.on_ground && mob.velocity_y > 0.0 && !blocked_x_head {
            // En montée d'un step-up : on continue à avancer tant que la
            // tête est libre, sinon on pourrait retomber avant le bord.
            tf.translation.x = new_x;
        } else {
            // Totalement bloqué → demi-tour sur cet axe pour l'errance.
            mob.wander.x = -mob.wander.x;
        }

        let blocked_z_feet = is_block_solid(tf.translation.x, feet_y, new_z, &chunk_manager, &chunk_q);
        let blocked_z_head = is_block_solid(tf.translation.x, head_y, new_z, &chunk_manager, &chunk_q);

        if !blocked_z_feet && !blocked_z_head {
            tf.translation.z = new_z;
        } else if mob.on_ground && blocked_z_feet && !blocked_z_head
            && !is_block_solid(tf.translation.x, feet_y + 1.0, new_z, &chunk_manager, &chunk_q)
        {
            tf.translation.z = new_z;
            mob.velocity_y = 8.0;
            mob.on_ground = false;
        } else if !mob.on_ground && mob.velocity_y > 0.0 && !blocked_z_head {
            tf.translation.z = new_z;
        } else {
            mob.wander.z = -mob.wander.z;
        }

        // Les slimes sautent aussi aléatoirement pour le style "bondissant".
        if mob.kind == MobKind::Slime && mob.on_ground && rng.gen_ratio(1, 90) {
            mob.velocity_y = 5.5;
            mob.on_ground = false;
        }
    }
}

/// IA des animaux : simple errance à vitesse constante, demi-tour si bloqué,
/// step-up d'un bloc. Pas de chasse : les animaux ignorent le joueur.
fn animal_ai(
    mut animal_q:  Query<(&mut Transform, &mut Animal)>,
    time:          Res<Time>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
) {
    let dt = time.delta_secs();
    let mut rng = rand::thread_rng();

    for (mut tf, mut animal) in animal_q.iter_mut() {
        animal.wander_t -= dt;
        if animal.wander_t <= 0.0 {
            let a = rng.gen_range(0.0..TAU);
            animal.wander = Vec3::new(a.cos(), 0.0, a.sin());
            animal.wander_t = rng.gen_range(3.0..7.0);
        }
        let speed = match animal.kind {
            AnimalKind::Cow     => 0.8,
            AnimalKind::Chicken => 1.5,
            AnimalKind::Pig     => 0.9,
        };

        // Collision avec step-up : trois tests par axe (pieds, haut du
        // corps, museau avancé) pour éviter que la tête traverse un mur.
        let new_x = tf.translation.x + animal.wander.x * speed * dt;
        let new_z = tf.translation.z + animal.wander.z * speed * dt;
        let bottom_y = tf.translation.y - animal.foot_offset;
        let top_y    = tf.translation.y + animal.half_height;
        let head_y   = tf.translation.y + head_y_center(animal.kind);
        let reach    = head_reach(animal.kind);
        let fwd_x = animal.wander.x;
        let fwd_z = animal.wander.z;

        let blocked_x_bot  = is_block_solid(new_x, bottom_y, tf.translation.z, &chunk_manager, &chunk_q);
        let blocked_x_top  = is_block_solid(new_x, top_y, tf.translation.z, &chunk_manager, &chunk_q);
        let blocked_x_head = is_block_solid(new_x + fwd_x * reach, head_y, tf.translation.z + fwd_z * reach, &chunk_manager, &chunk_q);

        if !blocked_x_bot && !blocked_x_top && !blocked_x_head {
            tf.translation.x = new_x;
        } else if animal.on_ground && blocked_x_bot && !blocked_x_top && !blocked_x_head
            && !is_block_solid(new_x, bottom_y + 1.0, tf.translation.z, &chunk_manager, &chunk_q)
        {
            tf.translation.x = new_x;
            animal.velocity_y = 7.0;
            animal.on_ground = false;
        } else if !animal.on_ground && animal.velocity_y > 0.0 && !blocked_x_top && !blocked_x_head {
            tf.translation.x = new_x;
        } else {
            animal.wander.x = -animal.wander.x;
        }

        let blocked_z_bot  = is_block_solid(tf.translation.x, bottom_y, new_z, &chunk_manager, &chunk_q);
        let blocked_z_top  = is_block_solid(tf.translation.x, top_y, new_z, &chunk_manager, &chunk_q);
        let blocked_z_head = is_block_solid(tf.translation.x + fwd_x * reach, head_y, new_z + fwd_z * reach, &chunk_manager, &chunk_q);

        if !blocked_z_bot && !blocked_z_top && !blocked_z_head {
            tf.translation.z = new_z;
        } else if animal.on_ground && blocked_z_bot && !blocked_z_top && !blocked_z_head
            && !is_block_solid(tf.translation.x, bottom_y + 1.0, new_z, &chunk_manager, &chunk_q)
        {
            tf.translation.z = new_z;
            animal.velocity_y = 7.0;
            animal.on_ground = false;
        } else if !animal.on_ground && animal.velocity_y > 0.0 && !blocked_z_top && !blocked_z_head {
            tf.translation.z = new_z;
        } else {
            animal.wander.z = -animal.wander.z;
        }

        if animal.wander.length_squared() > 0.01 {
            let angle = animal.wander.x.atan2(animal.wander.z);
            tf.rotation = Quat::from_rotation_y(angle);
        }
    }
}

/// Attaque mêlée du joueur au clic gauche : cherche la cible la plus proche
/// dans un cône de ~60° devant la caméra à moins de 3.5m. L'animation de
/// swing est déclenchée même si aucune cible n'est touchée.
fn player_melee_attack(
    mouse:        Res<ButtonInput<MouseButton>>,
    time:         Res<Time>,
    mut state:    ResMut<MeleeState>,
    player_q:     Query<(&Transform, &PlayerStats), With<Player>>,
    camera_q:     Query<&GlobalTransform, With<PlayerCamera>>,
    mut mob_q:    Query<(&Transform, &mut Mob), Without<Player>>,
    mut animal_q: Query<(Entity, &Transform, &mut Animal), (Without<Player>, Without<Mob>)>,
    mut bursts:   EventWriter<SpawnParticleBurst>,
    mut anim:     ResMut<PlayerAnimState>,
) {
    let dt = time.delta_secs();
    state.cooldown = (state.cooldown - dt).max(0.0);

    if !mouse.just_pressed(MouseButton::Left) { return; }
    if state.cooldown > 0.0 { return; }

    let Ok((ptf, stats)) = player_q.get_single() else { return };
    let Ok(cam) = camera_q.get_single() else { return };

    let origin = cam.translation();
    let forward = cam.forward().as_vec3();
    let dmg = stats.melee_dmg;
    let reach = 3.5;

    let mut best_dist = f32::MAX;
    let mut hit_pos = Vec3::ZERO;

    // Swing joué qu'il y ait cible ou non : le joueur sent le coup partir.
    anim.attack_phase = 1.0;
    state.cooldown = 0.35;

    let mut hit = false;

    for (mtf, mut mob) in mob_q.iter_mut() {
        let mob_center = mtf.translation + Vec3::Y * 0.8;
        let to_mob = mob_center - origin;
        let dist = to_mob.length();
        if dist > reach { continue; }

        // Cône frontal ~60° : dot > 0.5 filtre ce qui est hors champ.
        let dot = to_mob.normalize_or_zero().dot(forward);
        if dot < 0.5 { continue; }

        if dist < best_dist {
            best_dist = dist;
            mob.hp -= dmg;
            hit_pos = mtf.translation + Vec3::Y * 0.5;
            hit = true;
            break;
        }
    }

    if !hit {
        for (_, atf, mut animal) in animal_q.iter_mut() {
            let animal_center = atf.translation + Vec3::Y * 0.3;
            let to_animal = animal_center - origin;
            let dist = to_animal.length();
            if dist > reach { continue; }

            let dot = to_animal.normalize_or_zero().dot(forward);
            if dot < 0.5 { continue; }

            if dist < best_dist {
                animal.hp -= dmg;
                hit_pos = atf.translation;
                hit = true;
                break;
            }
        }
    }

    if hit {
        bursts.send(SpawnParticleBurst {
            position: hit_pos,
            color:    Color::srgb(1.0, 0.85, 0.6),
            count:    10,
            speed:    3.0,
            lifetime: 0.35,
        });
    }
}

/// Dégâts au contact mob → joueur. Applique un knockback à l'opposé du mob
/// pour éviter de rester coincé contre lui. Respecte les buffs (Cloak =
/// intangibilité, WaterShield = -50% de dégâts).
fn mob_contact_damage(
    mut mob_q:    Query<(&Transform, &mut Mob)>,
    mut player_q: Query<(&Transform, &mut Player, &mut PlayerStats)>,
    cloak:        Res<crate::magic::spells::CloakState>,
    shield:       Res<crate::magic::spells::WaterShieldState>,
    mut bursts:   EventWriter<SpawnParticleBurst>,
) {
    let Ok((ptf, mut player, mut stats)) = player_q.get_single_mut() else { return };
    if cloak.remaining > 0.0 { return; }

    for (tf, mut mob) in mob_q.iter_mut() {
        if mob.hit_cd > 0.0 { continue; }
        let diff = ptf.translation - tf.translation;
        let d = diff.length();
        if d < 1.8 {
            let mut dmg = mob.damage;
            if shield.remaining > 0.0 { dmg *= 0.5; }
            stats.current_hp = (stats.current_hp - dmg).max(0.0);
            mob.hit_cd = 1.0;

            // Knockback horizontal + petit boost vertical pour dégager le
            // joueur du mob (sinon dégât continu tant qu'on reste collé).
            let kb_dir = if d > 0.01 { Vec3::new(diff.x, 0.0, diff.z).normalize_or_zero() } else { Vec3::X };
            player.velocity.x += kb_dir.x * 6.0;
            player.velocity.z += kb_dir.z * 6.0;
            player.velocity.y = player.velocity.y.max(4.0);

            bursts.send(SpawnParticleBurst {
                position: ptf.translation + Vec3::Y * 0.5,
                color:    Color::srgb(0.9, 0.15, 0.1),
                count:    12,
                speed:    3.0,
                lifetime: 0.4,
            });
        }
    }
}

/// Impact projectile → mob/animal. Rayon de hit élargi (2m) parce que les
/// projectiles sont rapides et que tester la case exacte du mob produit
/// trop de ratés à cause du pas d'intégration.
fn mob_hit_by_projectile(
    mut commands: Commands,
    proj_q:       Query<(Entity, &Transform, &Projectile)>,
    mut mob_q:    Query<(&Transform, &mut Mob), Without<Projectile>>,
    mut animal_q: Query<(Entity, &Transform, &mut Animal), (Without<Projectile>, Without<Mob>)>,
    mut bursts:   EventWriter<SpawnParticleBurst>,
) {
    for (proj_ent, ptf, proj) in &proj_q {
        let dmg = match proj.kind {
            ProjectileKind::Fire  => 22.0,
            ProjectileKind::Ice   => 18.0,
            ProjectileKind::Drain => 14.0,
            ProjectileKind::Light => 16.0,
            ProjectileKind::Wind  => 20.0,
        };
        let hit_radius = 2.0;
        let mut hit_pos = Vec3::ZERO;
        let mut hit = false;

        for (mtf, mut mob) in mob_q.iter_mut() {
            // Offset Y=0.8 pour viser le "torse" du mob, sinon le projectile
            // tiré à hauteur d'yeux passe au-dessus du slime et le rate.
            let mob_center = mtf.translation + Vec3::Y * 0.8;
            if (mob_center - ptf.translation).length() < hit_radius {
                mob.hp -= dmg;
                hit_pos = mtf.translation + Vec3::Y * 0.5;
                hit = true;
                break;
            }
        }
        if !hit {
            for (_, atf, mut animal) in animal_q.iter_mut() {
                let animal_center = atf.translation;
                if (animal_center - ptf.translation).length() < hit_radius {
                    animal.hp -= dmg;
                    hit_pos = atf.translation;
                    hit = true;
                    break;
                }
            }
        }
        if hit {
            let burst_color = match proj.kind {
                ProjectileKind::Fire  => Color::srgb(1.0, 0.5, 0.1),
                ProjectileKind::Ice   => Color::srgb(0.5, 0.85, 1.0),
                ProjectileKind::Drain => Color::srgb(0.55, 0.1, 0.75),
                ProjectileKind::Light => Color::srgb(1.0, 1.0, 0.85),
                ProjectileKind::Wind  => Color::srgb(0.7, 1.0, 0.85),
            };
            bursts.send(SpawnParticleBurst {
                position: hit_pos,
                color:    burst_color,
                count:    16,
                speed:    4.0,
                lifetime: 0.5,
            });
            commands.entity(proj_ent).despawn_recursive();
        }
    }
}

/// Anime les bras et jambes des humanoids. Le mouvement est piloté par la
/// vitesse réelle (delta position), pas par `mob.speed`, pour que l'anim
/// s'arrête quand le mob est bloqué contre un mur.
fn mob_walk_animation(
    time:       Res<Time>,
    mut mob_q:  Query<(&Transform, &mut Mob, &Children)>,
    mut part_q: Query<(&MobPartTag, &mut Transform), Without<Mob>>,
) {
    let dt = time.delta_secs();

    for (tf, mut mob, children) in mob_q.iter_mut() {
        let dx = tf.translation.x - mob.prev_x;
        let dz = tf.translation.z - mob.prev_z;
        mob.prev_x = tf.translation.x;
        mob.prev_z = tf.translation.z;
        let speed_xz = (dx * dx + dz * dz).sqrt() / dt.max(0.001);
        let is_moving = speed_xz > 0.3;

        if is_moving {
            mob.walk_phase += dt * 8.0;
        } else {
            // Retour doux à zéro plutôt que cut brusque, pour ne pas figer
            // les membres au milieu de leur cycle à l'arrêt.
            mob.walk_phase *= 1.0 - dt * 10.0;
            if mob.walk_phase.abs() < 0.02 { mob.walk_phase = 0.0; }
        }

        let walk = mob.walk_phase.sin() * 0.50;

        // Slime et Wolf ont leurs anims dédiées (squash & stretch / bob).
        if mob.kind == MobKind::Slime { continue; }
        if mob.kind == MobKind::Wolf  { continue; }

        for &child in children.iter() {
            let Ok((tag, mut ptf)) = part_q.get_mut(child) else { continue };
            match tag {
                MobPartTag::ArmLeft => {
                    let angle = walk;
                    ptf.translation = Vec3::new(-0.30, 0.975 - 0.325 * (1.0 - angle.cos()), -0.325 * angle.sin());
                    ptf.rotation = Quat::from_rotation_x(angle);
                }
                MobPartTag::ArmRight => {
                    let angle = -walk;
                    ptf.translation = Vec3::new(0.30, 0.975 - 0.325 * (1.0 - angle.cos()), -0.325 * angle.sin());
                    ptf.rotation = Quat::from_rotation_x(angle);
                }
                MobPartTag::LegLeft => {
                    let angle = -walk;
                    ptf.translation = Vec3::new(-0.10, 0.325 - 0.325 * (1.0 - angle.cos()), -0.325 * angle.sin());
                    ptf.rotation = Quat::from_rotation_x(angle);
                }
                MobPartTag::LegRight => {
                    let angle = walk;
                    ptf.translation = Vec3::new(0.10, 0.325 - 0.325 * (1.0 - angle.cos()), -0.325 * angle.sin());
                    ptf.rotation = Quat::from_rotation_x(angle);
                }
                _ => {}
            }
        }
    }
}

/// Squash & stretch sur le slime : il s'étire verticalement puis s'aplatit
/// en bougeant. Modulé par le rythme de déplacement.
fn mob_slime_animation(
    time:      Res<Time>,
    mut mob_q: Query<(&mut Transform, &mut Mob)>,
) {
    let dt = time.delta_secs();

    for (mut tf, mut mob) in mob_q.iter_mut() {
        if mob.kind != MobKind::Slime { continue; }

        let dx = tf.translation.x - mob.prev_x;
        let dz = tf.translation.z - mob.prev_z;
        mob.prev_x = tf.translation.x;
        mob.prev_z = tf.translation.z;
        let speed_xz = (dx * dx + dz * dz).sqrt() / dt.max(0.001);

        if speed_xz > 0.2 {
            mob.walk_phase += dt * 6.0;
        } else {
            mob.walk_phase *= 1.0 - dt * 8.0;
        }

        let bounce = mob.walk_phase.sin().abs();
        let sy = 0.85 + bounce * 0.20;
        let sx = 0.85 + (1.0 - bounce) * 0.08;
        tf.scale = Vec3::new(sx, sy, sx);
    }
}

/// Wolf : léger bob vertical et roulis sur le Z pour simuler la foulée.
/// On extrait le yaw actuel pour préserver le facing avant de réappliquer
/// une rotation composée (yaw + roll).
fn mob_wolf_animation(
    time:      Res<Time>,
    mut mob_q: Query<(&mut Transform, &mut Mob)>,
) {
    let dt = time.delta_secs();

    for (mut tf, mut mob) in mob_q.iter_mut() {
        if mob.kind != MobKind::Wolf { continue; }

        let dx = tf.translation.x - mob.prev_x;
        let dz = tf.translation.z - mob.prev_z;
        mob.prev_x = tf.translation.x;
        mob.prev_z = tf.translation.z;
        let speed_xz = (dx * dx + dz * dz).sqrt() / dt.max(0.001);

        if speed_xz > 0.3 {
            mob.walk_phase += dt * 10.0;
        } else {
            mob.walk_phase *= 1.0 - dt * 8.0;
        }

        let bob = mob.walk_phase.sin() * 0.06;
        let roll = (mob.walk_phase * 0.5).sin() * 0.10;
        let facing = tf.rotation;
        let (axis, angle) = facing.to_axis_angle();
        let yaw_only = if axis.y.abs() > 0.5 { angle * axis.y.signum() } else { 0.0 };
        tf.rotation = Quat::from_rotation_y(yaw_only) * Quat::from_rotation_z(roll);

        tf.scale = Vec3::new(0.7, 0.7 + bob, 1.1);
    }
}

/// Anim des animaux : rotation des pattes sur l'axe X (foulée), rebond
/// vertical du corps (2 rebonds par cycle = 1 par paire de pattes) et nod
/// de la tête en phase avec la démarche.
fn animal_walk_animation(
    time:         Res<Time>,
    mut animal_q: Query<(&Transform, &mut Animal, &Children)>,
    mut leg_q:    Query<(&AnimalLeg, &mut Transform), (Without<Animal>, Without<AnimalBody>, Without<AnimalHead>)>,
    mut body_q:   Query<&mut Transform, (With<AnimalBody>, Without<Animal>, Without<AnimalLeg>, Without<AnimalHead>)>,
    mut head_q:   Query<(&AnimalHead, &mut Transform), (Without<Animal>, Without<AnimalLeg>, Without<AnimalBody>)>,
) {
    let dt = time.delta_secs();

    for (tf, mut animal, children) in animal_q.iter_mut() {
        let dx = tf.translation.x - animal.prev_x;
        let dz = tf.translation.z - animal.prev_z;
        animal.prev_x = tf.translation.x;
        animal.prev_z = tf.translation.z;
        let speed_xz = (dx * dx + dz * dz).sqrt() / dt.max(0.001);
        let is_moving = speed_xz > 0.2;

        if is_moving {
            let anim_speed = match animal.kind {
                AnimalKind::Chicken => 12.0,
                _ => 7.0,
            };
            animal.walk_phase += dt * anim_speed;
        } else {
            animal.walk_phase *= 1.0 - dt * 10.0;
            if animal.walk_phase.abs() < 0.02 { animal.walk_phase = 0.0; }
        }

        let leg_amp = match animal.kind {
            AnimalKind::Cow     => 0.40,
            AnimalKind::Pig     => 0.35,
            AnimalKind::Chicken => 0.50,
        };
        let bounce_amp = match animal.kind {
            AnimalKind::Cow     => 0.04,
            AnimalKind::Pig     => 0.03,
            AnimalKind::Chicken => 0.07,
        };
        // `*2.0` + `.abs()` : 2 rebonds par cycle complet = 1 par paire.
        let bounce = (animal.walk_phase * 2.0).sin().abs() * bounce_amp;
        let nod_amp = match animal.kind {
            AnimalKind::Cow     => 0.12,
            AnimalKind::Pig     => 0.08,
            AnimalKind::Chicken => 0.20,
        };
        let nod = (animal.walk_phase * 2.0).sin() * nod_amp;

        for &child in children.iter() {
            if let Ok((leg, mut ltf)) = leg_q.get_mut(child) {
                let phase = animal.walk_phase + leg.phase_offset * std::f32::consts::PI;
                let swing = phase.sin() * leg_amp;
                ltf.rotation = Quat::from_rotation_x(swing);
            } else if let Ok(mut btf) = body_q.get_mut(child) {
                btf.translation.y = bounce;
            } else if let Ok((head, mut htf)) = head_q.get_mut(child) {
                htf.translation.y = head.base_y + bounce;
                htf.rotation = Quat::from_rotation_x(nod);
            }
        }
    }
}

fn despawn_dead_mobs(
    mut commands: Commands,
    mob_q:        Query<(Entity, &Mob)>,
    animal_q:     Query<(Entity, &Animal)>,
) {
    for (e, mob) in &mob_q {
        if mob.hp <= 0.0 {
            commands.entity(e).despawn_recursive();
        }
    }
    for (e, animal) in &animal_q {
        if animal.hp <= 0.0 {
            commands.entity(e).despawn_recursive();
        }
    }
}

/// Nettoie les mobs trop éloignés pour ne pas garder en mémoire ceux qui
/// ont dérivé hors du rayon de rendu. 80m = largement au-delà du vue, mais
/// pas assez pour que le respawn soit gênant si le joueur revient vite.
fn despawn_far_entities(
    mut commands: Commands,
    player_q:     Query<&Transform, With<Player>>,
    mob_q:        Query<(Entity, &Transform), With<Mob>>,
    animal_q:     Query<(Entity, &Transform), (With<Animal>, Without<Mob>)>,
) {
    let Ok(ptf) = player_q.get_single() else { return };
    let max_dist_sq = 80.0f32 * 80.0;
    for (e, tf) in &mob_q {
        if tf.translation.distance_squared(ptf.translation) > max_dist_sq {
            commands.entity(e).despawn_recursive();
        }
    }
    for (e, tf) in &animal_q {
        if tf.translation.distance_squared(ptf.translation) > max_dist_sq {
            commands.entity(e).despawn_recursive();
        }
    }
}
