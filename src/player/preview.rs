//! Rendu d'un mini-personnage 3D pour l'UI (sélection de race + inventaire).
//!
//! On spawn tout un micro-monde caché très loin dans la carte (`PX/PY/PZ`)
//! avec sa propre caméra qui rend vers une `Image` partagée — cette image
//! est ensuite affichée en `ImageNode` dans l'UI. Les entités du preview
//! sont isolées sur leur propre `RenderLayer` pour que la caméra principale
//! ne les voie pas.

use bevy::prelude::*;
use bevy::render::camera::{ClearColorConfig, RenderTarget};
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy::render::view::RenderLayers;

use crate::race::Race;

/// Render layer réservé au preview. La caméra principale n'en voit rien,
/// seule la caméra preview le rend.
pub const PREVIEW_LAYER: usize = 2;

/// Position cachée loin de la zone jouable — évite que le preview apparaisse
/// par hasard si le joueur explore vraiment loin.
const PX: f32 = 4000.0;
const PY: f32 = 4000.0;
const PZ: f32 = 4000.0;

/// Handle de l'image cible, à afficher dans l'UI. `dead_code` parce que
/// l'UI récupère l'image par `rt.clone()` directement, pas via la ressource.
#[derive(Resource)]
#[allow(dead_code)]
pub struct PreviewTarget(pub Handle<Image>);

/// Marque toutes les parties du corps du preview (pour les despawn en bloc).
#[derive(Component)]
pub struct PreviewCharacter;

/// Marque la caméra preview (pour la despawn quand on quitte l'écran).
#[derive(Component)]
pub struct PreviewCam;

/// Crée la scène RTT de A à Z : image cible, personnage, caméra dédiée.
/// Appelée depuis `spawn_inventory_ui` et `spawn_race_select` — la `Handle`
/// renvoyée est ce qu'on plugge dans l'`ImageNode` de l'UI.
pub fn create_preview_scene(
    commands:  &mut Commands,
    images:    &mut Assets<Image>,
    meshes:    &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    race:      &Race,
    width:     u32,
    height:    u32,
) -> Handle<Image> {
    // Image cible : même format que le swapchain par défaut de Bevy. Les flags
    // combinent "utilisable comme texture en shader", "destination de copie"
    // et "render attachment" pour que la caméra puisse y dessiner.
    let size = Extent3d { width, height, depth_or_array_layers: 1 };
    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label:             None,
            size,
            dimension:         TextureDimension::D2,
            format:            TextureFormat::Bgra8UnormSrgb,
            mip_level_count:   1,
            sample_count:      1,
            usage:             TextureUsages::TEXTURE_BINDING
                             | TextureUsages::COPY_DST
                             | TextureUsages::RENDER_ATTACHMENT,
            view_formats:      &[],
        },
        ..default()
    };
    image.resize(size);
    let rt = images.add(image);
    commands.insert_resource(PreviewTarget(rt.clone()));

    spawn_preview_char(commands, meshes, materials, race);

    // Caméra : placée en face du personnage, un peu au-dessus (regard à
    // hauteur de torse). `order: 1` la fait rendre après la caméra principale
    // et son `clear_color` custom lui donne son fond bleu nuit.
    commands.spawn((
        Camera3d::default(),
        Camera {
            target: RenderTarget::Image(rt.clone()),
            clear_color:   ClearColorConfig::Custom(Color::srgb(0.10, 0.10, 0.16)),
            order:         1,
            ..default()
        },
        Transform::from_xyz(PX, PY + 1.1, PZ + 2.5)
            .looking_at(Vec3::new(PX, PY + 0.9, PZ), Vec3::Y),
        RenderLayers::layer(PREVIEW_LAYER),
        PreviewCam,
    ));

    rt
}

/// Supprime tout le matériel preview quand on quitte l'écran concerné.
pub fn sys_destroy_preview(
    mut commands: Commands,
    chars:        Query<Entity, With<PreviewCharacter>>,
    cams:         Query<Entity, With<PreviewCam>>,
) {
    for e in &chars { commands.entity(e).despawn_recursive(); }
    for e in &cams  { commands.entity(e).despawn_recursive(); }
    commands.remove_resource::<PreviewTarget>();
}

/// Redessine le personnage quand la race sélectionnée change (sur l'écran de
/// choix de race). On despawn les anciennes parties et on en spawn de
/// nouvelles avec les bonnes couleurs — plus simple que de patcher les
/// matériaux existants.
pub fn sys_update_preview(
    mut commands:  Commands,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    chars:         Query<Entity, With<PreviewCharacter>>,
    selected:      Option<Res<crate::SelectedRacePreview>>,
) {
    let Some(sel) = selected else { return };
    if !sel.is_changed() { return; }
    let Some(race) = &sel.race else { return };

    for e in &chars { commands.entity(e).despawn_recursive(); }
    spawn_preview_char(&mut commands, &mut meshes, &mut materials, race);
}

/// Spawn des 6 cubes qui forment le mini-personnage (tête, torse, 2 bras,
/// 2 jambes). Toutes les parties sont `unlit` parce qu'on éclaire le preview
/// uniquement via la couleur ambiante de la caméra dédiée — ça garantit un
/// rendu constant quelle que soit l'heure du jour in-game.
fn spawn_preview_char(
    commands:  &mut Commands,
    meshes:    &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    race:      &Race,
) {
    let body  = body_color(race);
    let skin  = skin_color(race);
    let pants = darken(body, 0.55);

    let skin_m  = materials.add(StandardMaterial { base_color: skin,  unlit: true, ..default() });
    let body_m  = materials.add(StandardMaterial { base_color: body,  unlit: true, ..default() });
    let pants_m = materials.add(StandardMaterial { base_color: pants, unlit: true, ..default() });

    let head = meshes.add(Cuboid::new(0.50, 0.50, 0.50));
    let torso = meshes.add(Cuboid::new(0.38, 0.75, 0.25));
    let arm  = meshes.add(Cuboid::new(0.20, 0.65, 0.20));
    let leg  = meshes.add(Cuboid::new(0.18, 0.65, 0.22));

    let parts: &[(Handle<Mesh>, Handle<StandardMaterial>, Vec3)] = &[
        (head,        skin_m.clone(),  Vec3::new(PX,        PY + 1.55,  PZ)),
        (torso,       body_m,          Vec3::new(PX,        PY + 0.975, PZ)),
        (arm.clone(), skin_m.clone(),  Vec3::new(PX - 0.30, PY + 0.975, PZ)),
        (arm,         skin_m,          Vec3::new(PX + 0.30, PY + 0.975, PZ)),
        (leg.clone(), pants_m.clone(), Vec3::new(PX - 0.10, PY + 0.325, PZ)),
        (leg,         pants_m,         Vec3::new(PX + 0.10, PY + 0.325, PZ)),
    ];

    for (mesh, mat, pos) in parts {
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(*pos),
            RenderLayers::layer(PREVIEW_LAYER),
            PreviewCharacter,
        ));
    }
}

fn body_color(race: &Race) -> Color {
    match race {
        Race::Sylvaris  => Color::srgb(0.15, 0.45, 0.15),
        Race::Ignaar    => Color::srgb(0.55, 0.18, 0.06),
        Race::Aethyn    => Color::srgb(0.18, 0.32, 0.60),
        Race::Vorkai    => Color::srgb(0.32, 0.08, 0.48),
        Race::Crysthari => Color::srgb(0.45, 0.45, 0.70),
    }
}

fn skin_color(race: &Race) -> Color {
    match race {
        Race::Sylvaris  => Color::srgb(0.72, 0.85, 0.55),
        Race::Ignaar    => Color::srgb(0.80, 0.38, 0.18),
        Race::Aethyn    => Color::srgb(0.78, 0.82, 0.95),
        Race::Vorkai    => Color::srgb(0.22, 0.18, 0.28),
        Race::Crysthari => Color::srgb(0.88, 0.88, 0.98),
    }
}

fn darken(c: Color, factor: f32) -> Color {
    let l = c.to_linear();
    Color::linear_rgb(l.red * factor, l.green * factor, l.blue * factor)
}
