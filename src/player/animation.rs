//! Animation du corps 3D du joueur + du bras FP (vue première personne).
//!
//! Deux "rigs" coexistent :
//!   - Le **corps 3D** visible en TP (tête/torse/bras/jambes) oscille quand
//!     le joueur marche, frappe ou casse un bloc.
//!   - Le **bras FP** (un seul mesh d'avant-bras) vit sur un render layer
//!     dédié (`ARM_LAYER`) avec sa propre caméra. Il est caché en TP, visible
//!     en FP, et fait les mêmes swings que le bras TP.
//!
//! Les phases (`walk_phase`, `break_phase`, `place_phase`, `attack_phase`)
//! sont stockées dans une ressource partagée pour que les deux rigs restent
//! synchronisés — utile quand le joueur bascule FP ↔ TP en plein geste.

use bevy::prelude::*;
use bevy::render::view::RenderLayers;

use crate::GameState;
use crate::player::{CameraMode, Player, PlayerBodyPart, PlayerCamera};
use crate::world::breaking::BreakingState;

/// Render layer dédié au bras FP (vu uniquement par `ArmCam`).
pub const ARM_LAYER: usize = 3;

/// Main dominante : latéralise le bras FP à gauche ou à droite.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub enum HandSide { #[default] Right, Left }

/// Tag sémantique sur chaque partie du corps — permet d'animer les bras et
/// jambes sans toucher la tête et le torse.
#[derive(Component, Clone, Copy)]
pub enum BodyPartTag {
    Head,
    Torso,
    ArmLeft,
    ArmRight,
    LegLeft,
    LegRight,
}

#[derive(Component)]
pub struct ArmViewMesh;

#[derive(Component)]
pub struct ArmCam;

/// Phases partagées entre le rig TP et le bras FP. Toutes sont remises à
/// zéro quand l'action termine — sauf `walk_phase` qui fait un retour
/// progressif à zéro pour éviter un snap visuel quand le joueur s'arrête.
#[derive(Resource, Default)]
pub struct PlayerAnimState {
    pub walk_phase:   f32,
    pub break_phase:  f32,
    pub place_phase:  f32,
    pub attack_phase: f32,
}

pub struct PlayerAnimPlugin;

impl Plugin for PlayerAnimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerAnimState>()
           .add_systems(OnEnter(GameState::InGame), spawn_arm_view);
        // Les systèmes Update sont ajoutés par PlayerPlugin dans sa propre
        // chain pour éviter les conflits de query avec les systèmes de
        // mouvement (ils touchent aux mêmes Transforms).
    }
}

/// Spawn du bras FP et de sa caméra dédiée, attachés comme enfants de la
/// `PlayerCamera` pour hériter de son yaw/pitch.
fn spawn_arm_view(
    mut commands:  Commands,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cam_q:         Query<Entity, With<PlayerCamera>>,
    player_config: Res<crate::PlayerConfig>,
    existing:      Query<Entity, With<ArmViewMesh>>,
) {
    if !existing.is_empty() { return; }
    let Ok(cam_entity) = cam_q.get_single() else { return; };

    let skin = super::race_skin_color(&player_config.race);

    // Mesh du bras : petit cuboïde placé en bas à droite de l'écran, orienté
    // avec un léger angle pour ressembler à un avant-bras tendu.
    let arm_ent = commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(0.20, 0.65, 0.20))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: skin,
            unlit:      true,
            ..default()
        })),
        Transform {
            translation: Vec3::new(0.36, -0.42, -0.50),
            rotation:    Quat::from_rotation_z(-0.28) * Quat::from_rotation_x(-0.10),
            ..default()
        },
        RenderLayers::layer(ARM_LAYER),
        ArmViewMesh,
    )).id();

    // Caméra dédiée : ne clear pas la couleur (on garde le monde derrière),
    // son order=2 la fait rendre après la principale, et son depth buffer
    // frais garantit que le bras passe toujours devant le monde.
    let arm_cam_ent = commands.spawn((
        Camera3d::default(),
        Camera {
            order:       2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Transform::default(),
        RenderLayers::layer(ARM_LAYER),
        ArmCam,
    )).id();

    // Parenting : les deux sont enfants de PlayerCamera, donc suivent
    // automatiquement les mouvements de tête du joueur.
    commands.entity(cam_entity).add_child(arm_ent);
    commands.entity(cam_entity).add_child(arm_cam_ent);
}

/// Anime les bras et jambes du corps 3D (visible en TP). La priorité des
/// animations du bras droit est : attaque > cassage > pose > marche, parce
/// qu'on préfère toujours voir la dernière action déclenchée.
pub(super) fn sys_player_animation(
    player_q:    Query<&Player>,
    break_state: Res<BreakingState>,
    mouse:       Res<ButtonInput<MouseButton>>,
    mut parts_q: Query<(&BodyPartTag, &mut Transform), With<PlayerBodyPart>>,
    time:        Res<Time>,
    mut anim:    ResMut<PlayerAnimState>,
) {
    let Ok(player) = player_q.get_single() else { return };

    let dt         = time.delta_secs();
    let speed_xz   = Vec2::new(player.velocity.x, player.velocity.z).length();
    let is_moving  = speed_xz > 0.5;
    let is_breaking = break_state.target.is_some();

    // Phase de marche : incrémentée à vitesse fixe tant qu'on bouge, sinon
    // décroissance exponentielle pour ramener les membres au repos en
    // douceur (snap à zéro en-dessous d'un petit epsilon).
    if is_moving {
        anim.walk_phase += dt * 9.0;
    } else {
        anim.walk_phase *= 1.0 - dt * 10.0;
        if anim.walk_phase.abs() < 0.02 { anim.walk_phase = 0.0; }
    }

    if is_breaking {
        anim.break_phase += dt * 11.0;
    } else {
        anim.break_phase = 0.0;
    }

    // Pose : déclenchée au front montant du clic droit, puis retombe en ~0.2s.
    if mouse.just_pressed(MouseButton::Right) {
        anim.place_phase = 1.0;
    }
    if anim.place_phase > 0.0 {
        anim.place_phase = (anim.place_phase - dt * 5.0).max(0.0);
    }

    // Attaque : la phase est mise à 1.0 par le système de combat puis retombe.
    if anim.attack_phase > 0.0 {
        anim.attack_phase = (anim.attack_phase - dt * 6.0).max(0.0);
    }

    let walk = anim.walk_phase.sin() * 0.55; // amplitude max ~31°

    for (tag, mut tf) in parts_q.iter_mut() {
        match tag {
            BodyPartTag::ArmLeft => {
                let θ = walk;
                let (ty, tz) = pivot_offset(1.3, 0.325, θ);
                tf.translation = Vec3::new(-0.30, ty, tz);
                tf.rotation    = Quat::from_rotation_x(θ);
            }
            BodyPartTag::ArmRight => {
                let θ = if anim.attack_phase > 0.0 {
                    let atk_swing = (anim.attack_phase * std::f32::consts::PI).sin();
                    -(atk_swing * 1.3) - 0.10
                } else if is_breaking {
                    let swing = (1.0 - (anim.break_phase * 2.0).cos()) * 0.5;
                    -(swing * 1.05) - 0.08
                } else if anim.place_phase > 0.0 {
                    // Arc avant puis retour — demi-sinus synchronisé sur
                    // place_phase (1 → 0).
                    let place_swing = (anim.place_phase * std::f32::consts::PI).sin();
                    -(place_swing * 0.90) - 0.08
                } else {
                    -walk
                };
                let (ty, tz) = pivot_offset(1.3, 0.325, θ);
                tf.translation = Vec3::new(0.30, ty, tz);
                tf.rotation    = Quat::from_rotation_x(θ);
            }
            BodyPartTag::LegLeft => {
                let θ = -walk; // opposition avec le bras gauche
                let (ty, tz) = pivot_offset(0.65, 0.325, θ);
                tf.translation = Vec3::new(-0.10, ty, tz);
                tf.rotation    = Quat::from_rotation_x(θ);
            }
            BodyPartTag::LegRight => {
                let θ = walk; // opposition avec le bras droit
                let (ty, tz) = pivot_offset(0.65, 0.325, θ);
                tf.translation = Vec3::new(0.10, ty, tz);
                tf.rotation    = Quat::from_rotation_x(θ);
            }
            _ => {} // Head et Torso restent fixes.
        }
    }
}

/// Calcule (translation_y, translation_z) pour pivoter un membre autour
/// d'un point haut (épaule ou hanche) plutôt qu'autour du centre du mesh —
/// sinon les bras "coulissent" au lieu de se balancer.
///
/// `pivot_y` : hauteur du pivot dans l'espace local joueur
/// `half_h`  : demi-hauteur du mesh
/// `θ`       : angle de rotation (rad)
#[inline]
fn pivot_offset(pivot_y: f32, half_h: f32, θ: f32) -> (f32, f32) {
    (pivot_y - half_h * θ.cos(), -half_h * θ.sin())
}

/// Anime le bras FP : bob vertical léger pendant la marche, swing prioritaire
/// sur attaque > cassage > pose > repos. Les offsets correspondent à ceux du
/// rig TP, ce qui garantit qu'un geste déclenché en FP reste cohérent si le
/// joueur bascule en TP.
pub(super) fn sys_arm_view_anim(
    break_state: Res<BreakingState>,
    anim:        Res<PlayerAnimState>,
    hand:        Res<HandSide>,
    mut arm_q:   Query<&mut Transform, With<ArmViewMesh>>,
) {
    let Ok(mut tf) = arm_q.get_single_mut() else { return };

    let is_breaking = break_state.target.is_some();

    let bob_y = if anim.walk_phase.abs() > 0.02 {
        (anim.walk_phase * 2.0).sin() * 0.018
    } else {
        0.0
    };

    let rot_x = if anim.attack_phase > 0.0 {
        let atk_swing = (anim.attack_phase * std::f32::consts::PI).sin();
        -0.10 - atk_swing * 1.0
    } else if is_breaking {
        let swing = (1.0 - (anim.break_phase * 2.0).cos()) * 0.5;
        -0.10 - swing * 0.85
    } else if anim.place_phase > 0.0 {
        let place_swing = (anim.place_phase * std::f32::consts::PI).sin();
        -0.10 - place_swing * 0.55
    } else {
        -0.10
    };

    // Position latérale selon la main dominante.
    let (arm_x, rot_z) = match *hand {
        HandSide::Right => ( 0.36, -0.28),
        HandSide::Left  => (-0.36,  0.28),
    };

    tf.translation = Vec3::new(arm_x, -0.42 + bob_y, -0.50);
    tf.rotation    = Quat::from_rotation_z(rot_z) * Quat::from_rotation_x(rot_x);
}

/// Cache le bras FP et sa caméra en vue 3ème personne — sinon on verrait un
/// bras flotter devant le corps visible, ce qui serait franchement bizarre.
pub(super) fn sys_arm_view_visibility(
    mode:       Res<CameraMode>,
    mut arm_q:  Query<&mut Visibility, With<ArmViewMesh>>,
    mut cam_q:  Query<&mut Camera, With<ArmCam>>,
) {
    let is_fp = *mode == CameraMode::FirstPerson;
    if let Ok(mut vis) = arm_q.get_single_mut() {
        *vis = if is_fp { Visibility::Visible } else { Visibility::Hidden };
    }
    // On désactive aussi la caméra pour éviter tout rendu résiduel ARM_LAYER.
    if let Ok(mut cam) = cam_q.get_single_mut() {
        cam.is_active = is_fp;
    }
}

/// Cache le corps 3D en vue 1ère personne (sinon on verrait sa tête/ses bras
/// en bas de l'écran, plutôt moche).
pub(super) fn sys_body_visibility(
    mode:        Res<CameraMode>,
    mut parts_q: Query<&mut Visibility, With<PlayerBodyPart>>,
) {
    let vis = match *mode {
        CameraMode::FirstPerson => Visibility::Hidden,
        CameraMode::ThirdPerson => Visibility::Visible,
    };
    for mut v in parts_q.iter_mut() {
        *v = vis;
    }
}
