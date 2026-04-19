//! Particules Alpha — petits cubes unlit avec gravité, fade et durée de vie.
//!
//! Les autres modules envoient un `SpawnParticleBurst { position, color, ... }`
//! et `apply_particle_spawns` instancie les entités. `update_particles` gère
//! ensuite mouvement, gravité et fade-out jusqu'à expiration.

use bevy::prelude::*;
use crate::GameState;

/// Demande d'éclatement de particules — envoyée par d'autres systèmes
/// (cassage de bloc, impact de sort, etc.).
#[derive(Event, Clone)]
pub struct SpawnParticleBurst {
    pub position: Vec3,
    pub color:    Color,
    pub count:    u32,
    pub speed:    f32,
    pub lifetime: f32,
}

/// Une particule vivante — porte son propre matériau pour que le fade alpha
/// soit indépendant de ses voisines.
#[derive(Component)]
pub struct Particle {
    pub velocity: Vec3,
    pub age:      f32,
    pub lifetime: f32,
    pub gravity:  f32,
    pub material: Handle<StandardMaterial>,
    pub base_color: Color,
}

pub struct ParticlesPlugin;

impl Plugin for ParticlesPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SpawnParticleBurst>()
           .add_systems(Update, (
               apply_particle_spawns,
               update_particles,
           ).run_if(in_state(GameState::InGame)));
    }
}

/// Matérialise chaque `SpawnParticleBurst` reçu en `count` particules qui
/// partent en gerbe dans toutes les directions. Les directions viennent d'une
/// spirale déterministe basée sur la position de l'éclatement — c'est moins
/// aléatoire qu'un vrai random mais suffit visuellement et évite d'ajouter
/// un RNG ici.
fn apply_particle_spawns(
    mut commands:  Commands,
    mut events:    EventReader<SpawnParticleBurst>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh_handle = meshes.add(Cuboid::from_size(Vec3::splat(0.10)));

    for ev in events.read() {
        for i in 0..ev.count {
            let t = i as f32 / ev.count.max(1) as f32;
            let yaw = t * std::f32::consts::TAU + ev.position.x * 0.7;
            let pitch = (t * 4.7 + ev.position.z * 0.3).sin() * 0.6 + 0.5;
            let dir = Vec3::new(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos());
            let velocity = dir * ev.speed;

            let mat = materials.add(StandardMaterial {
                base_color: ev.color,
                emissive:   linear_from_color(ev.color, 0.7),
                unlit:      true,
                alpha_mode: AlphaMode::Blend,
                ..default()
            });

            commands.spawn((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(ev.position),
                Particle {
                    velocity,
                    age:        0.0,
                    lifetime:   ev.lifetime,
                    gravity:    9.8,
                    material:   mat,
                    base_color: ev.color,
                },
            ));
        }
    }
}

/// Intègre gravité + vitesse, fade l'alpha vers 0 au fil de la vie, puis
/// despawn à l'expiration.
fn update_particles(
    mut commands:  Commands,
    time:          Res<Time>,
    mut q:         Query<(Entity, &mut Particle, &mut Transform)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let dt = time.delta_secs();
    for (entity, mut p, mut tf) in &mut q {
        p.age += dt;
        if p.age >= p.lifetime {
            commands.entity(entity).despawn();
            continue;
        }
        p.velocity.y -= p.gravity * dt;
        tf.translation += p.velocity * dt;

        let life_ratio = 1.0 - (p.age / p.lifetime).clamp(0.0, 1.0);
        if let Some(mat) = materials.get_mut(&p.material) {
            mat.base_color = with_alpha(p.base_color, life_ratio);
        }
    }
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    let lin = color.to_linear();
    Color::srgba(lin.red, lin.green, lin.blue, alpha)
}

fn linear_from_color(color: Color, intensity: f32) -> LinearRgba {
    let lin = color.to_linear();
    LinearRgba::new(lin.red * intensity, lin.green * intensity, lin.blue * intensity, 1.0)
}
