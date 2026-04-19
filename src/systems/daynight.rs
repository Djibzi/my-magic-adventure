//! Cycle jour/nuit + soleil/lune/nuages.
//!
//! Un compteur `DayTime::time` cycle de 0 à 1 sur 600 secondes (10 minutes
//! réelles par jour en jeu). 0 = midi, 0.5 = minuit. À partir de ce scalaire
//! on dérive :
//!   - la couleur du ciel (noir étoilé → bleu nuit → bleu ciel),
//!   - la direction et l'intensité de la `DirectionalLight` (le soleil
//!     "physique" qui projette les ombres),
//!   - la couleur ambiante,
//!   - la position visuelle du soleil et de la lune (carrés plats en
//!     billboard, centrés sur la caméra).
//!
//! Les nuages sont un jeu de petits cuboïdes semi-transparents qui dérivent
//! lentement sur X pour simuler du vent.

use bevy::prelude::*;
use bevy::pbr::{NotShadowCaster, CascadeShadowConfigBuilder};
use crate::GameState;

#[derive(Component)] pub struct Sun;
#[derive(Component)] struct SunVisual;
#[derive(Component)] struct MoonVisual;
#[derive(Component)] struct Cloud;

/// Horloge du cycle jour/nuit, normalisée sur [0, 1). Persistée entre
/// pauses / changements d'état pour ne pas "sauter" à la reprise.
#[derive(Resource)]
pub struct DayTime {
    /// 0.0 = midi, 0.5 = minuit. Cycle complet en 600 s.
    pub time: f32,
}
impl Default for DayTime {
    fn default() -> Self { Self { time: 0.0 } }
}

pub struct DayNightPlugin;

impl Plugin for DayNightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DayTime>()
           .insert_resource(AmbientLight { color: Color::WHITE, brightness: 600. })
           .add_systems(OnEnter(GameState::InGame), (spawn_sun, spawn_sky_objects))
           .add_systems(
               Update,
               (update_daynight, update_sky_positions, drift_clouds)
                   // Le ciel continue de tourner même en pause / menus pour
                   // éviter un "gel" visuel gênant quand on revient au jeu.
                   .run_if(
                       in_state(GameState::InGame)
                       .or(in_state(GameState::Paused))
                       .or(in_state(GameState::Inventory))
                       .or(in_state(GameState::Options))
                   ),
           );
    }
}

/// Spawn de la `DirectionalLight` qui fait le soleil physique. Deux cascades
/// d'ombres suffisent pour le rayon de rendu actuel ; ajouter plus coûte vite
/// cher en GPU.
fn spawn_sun(mut commands: Commands, q: Query<Entity, With<Sun>>) {
    if !q.is_empty() { return; }
    commands.spawn((
        DirectionalLight {
            illuminance:     15_000.,
            shadows_enabled: true,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: 2,
            minimum_distance: 0.1,
            maximum_distance: 90.0,
            first_cascade_far_bound: 22.0,
            overlap_proportion: 0.2,
        }.build(),
        Transform::from_rotation(Quat::from_euler(
            EulerRot::XYZ,
            -std::f32::consts::FRAC_PI_2,
            0.4,
            0.,
        )),
        Sun,
    ));
}

/// Spawn des objets "visuels" du ciel : un carré jaune pour le soleil, un
/// carré blanc-bleuté pour la lune, et des paquets de nuages plats. Tout est
/// `unlit` pour rester lumineux quelle que soit l'heure — sinon le soleil
/// serait noir la nuit.
fn spawn_sky_objects(
    mut commands: Commands,
    mut meshes:   ResMut<Assets<Mesh>>,
    mut mats:     ResMut<Assets<StandardMaterial>>,
    existing:     Query<Entity, With<SunVisual>>,
) {
    if !existing.is_empty() { return; }

    // Soleil : carré jaune plat.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(24., 24., 0.5))),
        MeshMaterial3d(mats.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.95, 0.20),
            unlit:      true,
            cull_mode:  None,
            ..default()
        })),
        Transform::from_xyz(0., 400., 0.),
        SunVisual,
        NotShadowCaster,
    ));

    // Lune : un peu plus petite, blanc-bleuté.
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(18., 18., 0.5))),
        MeshMaterial3d(mats.add(StandardMaterial {
            base_color: Color::srgb(0.84, 0.88, 0.97),
            unlit:      true,
            cull_mode:  None,
            ..default()
        })),
        Transform::from_xyz(0., -400., 0.),
        MoonVisual,
        NotShadowCaster,
    ));

    // Nuages : paquets de 3 cuboïdes plats légèrement décalés pour un effet
    // "moutonneux", matériau blanc translucide commun à tous les paquets.
    let cloud_mat = mats.add(StandardMaterial {
        base_color:  Color::srgba(0.96, 0.97, 1.0, 0.88),
        alpha_mode:  AlphaMode::Blend,
        unlit:       true,
        double_sided: true,
        cull_mode:   None,
        ..default()
    });

    // (offset_x, offset_z, hauteur) de chaque paquet de nuages.
    let clusters: &[(f32, f32, f32)] = &[
        (  0.,   0., 50.),  ( 45.,  28., 52.), (-55.,  18., 49.),
        ( 85., -22., 51.),  (-35., -65., 53.), ( 65.,  72., 50.),
        (-80.,  48., 52.),  ( 22., -82., 51.), (-62., -42., 49.),
        (105.,  12., 53.),  (-15., 105., 50.), ( 52.,-108., 52.),
    ];

    for &(ox, oz, oy) in clusters {
        // Un paquet = bloc principal + 2 bosses latérales un peu décalées.
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(20., 4., 12.))),
            MeshMaterial3d(cloud_mat.clone()),
            Transform::from_xyz(ox, oy, oz),
            Cloud, NotShadowCaster,
        ));
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(14., 4., 9.))),
            MeshMaterial3d(cloud_mat.clone()),
            Transform::from_xyz(ox + 9., oy + 2., oz + 2.),
            Cloud, NotShadowCaster,
        ));
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(12., 4., 8.))),
            MeshMaterial3d(cloud_mat.clone()),
            Transform::from_xyz(ox - 7., oy + 2., oz - 3.),
            Cloud, NotShadowCaster,
        ));
    }
}

/// Fait avancer l'heure et propage sa valeur partout où elle influence le
/// rendu : ciel, lumière ambiante, direction et teinte du soleil physique.
fn update_daynight(
    time:         Res<Time>,
    mut day_time: ResMut<DayTime>,
    mut sun_q:    Query<(&mut DirectionalLight, &mut Transform), With<Sun>>,
    mut ambient:  ResMut<AmbientLight>,
    mut clear:    ResMut<ClearColor>,
) {
    const PERIOD: f32 = 600.0;
    day_time.time = (day_time.time + time.delta_secs() / PERIOD).fract();

    let t   = day_time.time;
    let cos = (t * std::f32::consts::TAU).cos(); // 1 = midi, -1 = minuit
    let day = ((cos + 1.0) * 0.5).clamp(0., 1.);

    // Couleur du ciel : noir étoilé → bleu nuit → bleu ciel.
    let sky_r = lerp(0.01, 0.52, day);
    let sky_g = lerp(0.01, 0.73, day);
    let sky_b = lerp(0.10, 0.95, day);
    clear.0 = Color::srgb(sky_r, sky_g, sky_b);

    // Lumière ambiante colorée : bleu nuit → blanc chaud en journée.
    ambient.color      = Color::srgb(lerp(0.10, 1.00, day), lerp(0.13, 0.97, day), lerp(0.32, 0.90, day));
    ambient.brightness = lerp(55., 620., day);

    if let Ok((mut light, mut transform)) = sun_q.get_single_mut() {
        light.illuminance = (day * 15_000.).max(0.);

        // Teinte du soleil : orange aux horizons (lever/coucher), blanc chaud
        // en plein jour. `horizon` = 1 quand le soleil touche l'horizon.
        let horizon    = 1.0 - cos.abs();
        light.color    = Color::srgb(1.0, (0.97 - horizon * 0.30).max(0.), (0.90 - horizon * 0.55).max(0.));

        let angle = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        transform.rotation = Quat::from_euler(EulerRot::XYZ, angle, 0.4, 0.);
    }
}

/// Replace le soleil et la lune en face/derrière la caméra chaque frame —
/// le soleil dans la direction "d'où vient la lumière", la lune à l'opposé.
fn update_sky_positions(
    day_time:     Res<DayTime>,
    cam_q:        Query<&GlobalTransform, With<Camera3d>>,
    mut sun_vis:  Query<&mut Transform, (With<SunVisual>,  Without<MoonVisual>)>,
    mut moon_vis: Query<&mut Transform, (With<MoonVisual>, Without<SunVisual>)>,
) {
    let Ok(cam_gt) = cam_q.get_single() else { return };
    let cam_pos = cam_gt.translation();

    let t   = day_time.time;
    let angle = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
    let rot = Quat::from_euler(EulerRot::XYZ, angle, 0.4, 0.);
    let sun_dir = rot * Vec3::NEG_Z;

    const DIST: f32 = 460.;

    if let Ok(mut st) = sun_vis.get_single_mut() {
        st.translation = cam_pos - sun_dir * DIST;
        // Garde-fou contre un up colinéaire à la direction de visée.
        let up = if sun_dir.dot(Vec3::Y).abs() > 0.98 { Vec3::Z } else { Vec3::Y };
        st.look_at(cam_pos, up);
    }
    if let Ok(mut mt) = moon_vis.get_single_mut() {
        mt.translation = cam_pos + sun_dir * DIST;
        let up = if (-sun_dir).dot(Vec3::Y).abs() > 0.98 { Vec3::Z } else { Vec3::Y };
        mt.look_at(cam_pos, up);
    }
}

/// Dérive lente des nuages sur X pour simuler du vent. Wrap dans une bande
/// de ±220 pour ne jamais les perdre de vue.
fn drift_clouds(time: Res<Time>, mut q: Query<&mut Transform, With<Cloud>>) {
    let drift = time.delta_secs() * 1.8;
    for mut t in &mut q {
        t.translation.x += drift;
        if t.translation.x > 220. { t.translation.x -= 440.; }
    }
}

#[inline] fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
