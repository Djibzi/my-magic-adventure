//! Joueur : spawn, contrôles, caméra et collisions.
//!
//! Le joueur est une entité unique parent d'une caméra `PlayerCamera` et d'un
//! ensemble de parties de corps (`BodyPartTag`) qui forment la silhouette
//! visible en 3ème personne. Le rig d'animation des bras FP est géré à part
//! dans [`animation`].
//!
//! La physique est volontairement simple : AABB contre la voxellisation du
//! monde, gravité constante, et un "plancher de sécurité" à y=0 pour ne pas
//! tomber dans le vide quand un chunk n'est pas encore chargé. Le composant
//! marqueur [`NeedsGrounding`] suspend gravité et déplacement tant que le
//! chunk sous le joueur n'est pas disponible — sans ça on traverserait le sol
//! au respawn et on finirait sur le plancher de sécurité.

pub mod animation;
pub mod preview;

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow, WindowFocused};

use crate::world::chunk::{Chunk, ChunkManager, CHUNK_HEIGHT, CHUNK_SIZE};
use crate::GameState;

#[derive(Resource, Default, PartialEq, Eq, Clone, Copy)]
pub enum CameraMode { #[default] FirstPerson, ThirdPerson }

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(animation::PlayerAnimPlugin)
           .init_resource::<CameraMode>()
           .init_resource::<animation::HandSide>()
           .add_systems(OnEnter(GameState::Loading), spawn_player)
           // `ground_player` tourne aussi pendant le Loading : il a besoin
           // d'attendre le chargement du chunk sous le spawn pour plaquer le
           // joueur au sol avant d'activer la gravité.
           .add_systems(
               Update,
               ground_player.run_if(
                   in_state(GameState::InGame).or(in_state(GameState::Loading))
               ),
           )
           // Tout le reste est strictement InGame : pas de mouvement, caméra
           // ou animations dans les menus / loading.
           .add_systems(
               Update,
               (
                   toggle_cursor_grab,
                   toggle_inventory,
                   toggle_camera_mode,
                   camera_look,
                   update_third_person_camera,
                   player_move,
                   apply_gravity,
                   apply_velocity,
                   animation::sys_player_animation,
                   animation::sys_arm_view_anim,
                   animation::sys_arm_view_visibility,
                   animation::sys_body_visibility,
               )
               .chain()
               .run_if(in_state(GameState::InGame)),
           )
           .add_systems(
               Update,
               close_inventory_key.run_if(in_state(GameState::Inventory)),
           )
           .add_systems(
               Update,
               relock_on_focus.run_if(in_state(GameState::InGame)),
           );
    }
}

/// Marqueur : le joueur n'a pas encore été placé sur le terrain. Tant qu'il
/// est présent, la gravité et les inputs de déplacement sont désactivés.
#[derive(Component)]
pub struct NeedsGrounding;

#[derive(Component)]
pub struct Player {
    pub velocity:  Vec3,
    pub on_ground: bool,
    pub pitch:     f32,
    pub yaw:       f32,
}

#[derive(Component)]
pub struct PlayerCamera;

/// Marqueur sur chaque cuboïde du corps 3D (tête, torse, bras, jambes).
#[derive(Component)]
pub struct PlayerBodyPart;

#[derive(Component)]
pub struct PlayerStats {
    pub current_hp:   f32,
    pub max_hp:       f32,
    pub current_mana: f32,
    pub max_mana:     f32,
    pub speed:        f32,
    pub melee_dmg:    f32,
}

/// Crée l'entité joueur, lui attache la caméra existante et construit les
/// parties du corps 3D. Lit la sauvegarde en attente si elle existe, en
/// forçant quand même une re-grounding (voir note sur `spawn_pos`).
fn spawn_player(
    mut commands:   Commands,
    mut meshes:     ResMut<Assets<Mesh>>,
    mut materials:  ResMut<Assets<StandardMaterial>>,
    camera_query:   Query<Entity, (With<Camera3d>, Without<preview::PreviewCam>, Without<animation::ArmCam>)>,
    player_config:  Res<crate::PlayerConfig>,
    pending_load:   Option<Res<crate::systems::PendingLoad>>,
) {
    let base = player_config.race.base_stats();
    let mut stats = PlayerStats {
        current_hp:   base.max_hp as f32,
        max_hp:       base.max_hp as f32,
        current_mana: base.max_mana as f32,
        max_mana:     base.max_mana as f32,
        speed:        base.speed * 5.0,
        melee_dmg:    base.melee_dmg * 8.0,
    };

    // On force toujours un Y=200 au spawn : le terrain sous-jacent peut ne
    // pas être identique à celui de la sauvegarde (bug de génération, hauteur
    // différente après édition), et spawner à l'ancien Y risquait de nous
    // coincer dans un bloc ou sous la map. NeedsGrounding fera la correction.
    let mut spawn_pos = Vec3::new(8.0, 200.0, 8.0);
    let needs_grounding = true;
    if let Some(pl) = pending_load.as_ref() {
        spawn_pos = Vec3::new(pl.px, 200.0, pl.pz);
        stats.current_hp   = pl.hp;
        stats.current_mana = pl.mana;
    }

    let mut player_cmds = commands.spawn((
        Transform::from_translation(spawn_pos),
        Visibility::default(),
        Player { velocity: Vec3::ZERO, on_ground: false, pitch: 0.0, yaw: 0.0 },
        stats,
    ));
    if needs_grounding {
        player_cmds.insert(NeedsGrounding);
    }
    let player = player_cmds.id();

    // Une sauvegarde en attente ne doit jamais s'appliquer deux fois — on la
    // consomme dès qu'on l'a utilisée, sinon un respawn récupèrerait ses HP
    // au lieu de repartir au max.
    if pending_load.is_some() {
        commands.remove_resource::<crate::systems::PendingLoad>();
    }

    if let Ok(cam) = camera_query.get_single() {
        commands.entity(cam)
            .insert((Transform::from_xyz(0.0, 1.7, 0.0), PlayerCamera))
            .set_parent(player);
    } else {
        warn!("spawn_player: no Camera3d found");
    }

    // Corps 3D blocky style Minecraft, visible par la caméra principale ET
    // celle de la preview d'inventaire. Les couleurs dépendent de la race.
    let body_color  = race_body_color(&player_config.race);
    let skin_color  = race_skin_color(&player_config.race);
    let pants_color = darken(body_color, 0.55);

    let skin_mat  = materials.add(StandardMaterial { base_color: skin_color,  perceptual_roughness: 0.9, ..default() });
    let body_mat  = materials.add(StandardMaterial { base_color: body_color,  perceptual_roughness: 0.9, ..default() });
    let pants_mat = materials.add(StandardMaterial { base_color: pants_color, perceptual_roughness: 0.9, ..default() });

    let head_mesh = meshes.add(Cuboid::new(0.50, 0.50, 0.50));
    let body_mesh = meshes.add(Cuboid::new(0.38, 0.75, 0.25));
    let arm_mesh  = meshes.add(Cuboid::new(0.20, 0.65, 0.20));
    let leg_mesh  = meshes.add(Cuboid::new(0.18, 0.65, 0.22));

    use animation::BodyPartTag;

    let parts: &[(Handle<Mesh>, Handle<StandardMaterial>, Vec3, BodyPartTag)] = &[
        (head_mesh,          skin_mat.clone(),  Vec3::new( 0.00, 1.55,  0.00), BodyPartTag::Head),
        (body_mesh,          body_mat.clone(),  Vec3::new( 0.00, 0.975, 0.00), BodyPartTag::Torso),
        (arm_mesh.clone(),   skin_mat.clone(),  Vec3::new(-0.30, 0.975, 0.00), BodyPartTag::ArmLeft),
        (arm_mesh,           skin_mat,          Vec3::new( 0.30, 0.975, 0.00), BodyPartTag::ArmRight),
        (leg_mesh.clone(),   pants_mat.clone(), Vec3::new(-0.10, 0.325, 0.00), BodyPartTag::LegLeft),
        (leg_mesh,           pants_mat,         Vec3::new( 0.10, 0.325, 0.00), BodyPartTag::LegRight),
    ];

    for (mesh, mat, pos, tag) in parts {
        let part = commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(*pos),
            PlayerBodyPart,
            *tag,
        )).id();
        commands.entity(player).add_child(part);
    }
}

/// Couleur principale (vêtements) selon la race.
pub fn race_body_color(race: &crate::race::Race) -> Color {
    match race {
        crate::race::Race::Sylvaris  => Color::srgb(0.15, 0.45, 0.15),
        crate::race::Race::Ignaar    => Color::srgb(0.55, 0.18, 0.06),
        crate::race::Race::Aethyn    => Color::srgb(0.18, 0.32, 0.60),
        crate::race::Race::Vorkai    => Color::srgb(0.32, 0.08, 0.48),
        crate::race::Race::Crysthari => Color::srgb(0.45, 0.45, 0.70),
    }
}

/// Couleur de la peau selon la race.
pub fn race_skin_color(race: &crate::race::Race) -> Color {
    match race {
        crate::race::Race::Sylvaris  => Color::srgb(0.72, 0.85, 0.55),
        crate::race::Race::Ignaar    => Color::srgb(0.80, 0.38, 0.18),
        crate::race::Race::Aethyn    => Color::srgb(0.78, 0.82, 0.95),
        crate::race::Race::Vorkai    => Color::srgb(0.22, 0.18, 0.28),
        crate::race::Race::Crysthari => Color::srgb(0.88, 0.88, 0.98),
    }
}

fn darken(c: Color, factor: f32) -> Color {
    let l = c.to_linear();
    Color::linear_rgb(l.red * factor, l.green * factor, l.blue * factor)
}

/// Attend que le chunk sous le joueur soit chargé, scanne la colonne pour
/// trouver la hauteur réelle du sol, puis téléporte le joueur dessus avant de
/// retirer le marqueur `NeedsGrounding`.
fn ground_player(
    mut commands:  Commands,
    mut player_q:  Query<(Entity, &mut Transform, &mut Player), With<NeedsGrounding>>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
) {
    let Ok((entity, mut transform, mut player)) = player_q.get_single_mut() else { return };

    let wx = transform.translation.x;
    let wz = transform.translation.z;

    let cx = (wx / CHUNK_SIZE as f32).floor() as i32;
    let cz = (wz / CHUNK_SIZE as f32).floor() as i32;

    // On lit le chunk réel (composant) plutôt qu'une régénération : si le
    // joueur a édité le monde, la régénération donnerait une hauteur fausse
    // et on pourrait atterrir dans un trou qui a été comblé depuis.
    let Some(&chunk_entity) = chunk_manager.loaded.get(&(cx, cz)) else { return };
    let Ok(real_chunk) = chunk_q.get(chunk_entity) else { return };

    let lx = (wx.floor() as i32).rem_euclid(CHUNK_SIZE as i32) as usize;
    let lz = (wz.floor() as i32).rem_euclid(CHUNK_SIZE as i32) as usize;

    let mut surface_y: i32 = -1;
    for y in (0..CHUNK_HEIGHT).rev() {
        if real_chunk.blocks[lx][y][lz].is_solid() {
            surface_y = y as i32;
            break;
        }
    }

    if surface_y < 0 {
        warn!("ground_player: aucune colonne solide à lx={lx} lz={lz} chunk=({cx},{cz})");
        return;
    }

    // +1.05 plutôt que +1.0 pile : petite marge flottante pour que le joueur
    // ne touche pas "le bloc du dessous" au premier tick et ne se fasse pas
    // éjecter par la collision.
    let target_y = surface_y as f32 + 1.05;
    transform.translation.y = target_y;
    player.velocity = Vec3::ZERO;
    player.on_ground = true;

    commands.entity(entity).remove::<NeedsGrounding>();
}

/// Sur Windows (iGPU surtout), winit peut libérer le grab du curseur
/// silencieusement à la perte de focus. On le ré-applique quand la fenêtre
/// reprend le focus pour éviter que le joueur ait besoin de cliquer.
fn relock_on_focus(
    mut events:  EventReader<WindowFocused>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    for ev in events.read() {
        if ev.focused {
            if let Ok(mut win) = windows.get_single_mut() {
                win.cursor_options.grab_mode = CursorGrabMode::Confined;
                win.cursor_options.visible   = false;
            }
        }
    }
}

fn toggle_cursor_grab(
    keys:           Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut windows:    Query<&mut Window, With<PrimaryWindow>>,
) {
    if !keys.just_pressed(KeyCode::Escape) { return; }
    if let Ok(mut win) = windows.get_single_mut() {
        win.cursor_options.grab_mode = CursorGrabMode::None;
        win.cursor_options.visible   = true;
    }
    next_state.set(GameState::Paused);
}

fn toggle_inventory(
    keys:           Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut windows:    Query<&mut Window, With<PrimaryWindow>>,
) {
    if !keys.just_pressed(KeyCode::KeyE) { return; }
    if let Ok(mut win) = windows.get_single_mut() {
        win.cursor_options.grab_mode = CursorGrabMode::None;
        win.cursor_options.visible   = true;
    }
    next_state.set(GameState::Inventory);
}

fn close_inventory_key(
    keys:           Res<ButtonInput<KeyCode>>,
    mut next_state: ResMut<NextState<GameState>>,
    mut windows:    Query<&mut Window, With<PrimaryWindow>>,
) {
    if keys.just_pressed(KeyCode::KeyE) || keys.just_pressed(KeyCode::Escape) {
        if let Ok(mut win) = windows.get_single_mut() {
            win.cursor_options.grab_mode = CursorGrabMode::Confined;
            win.cursor_options.visible   = false;
        }
        next_state.set(GameState::InGame);
    }
}

fn toggle_camera_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<CameraMode>,
) {
    if keys.just_pressed(KeyCode::F5) {
        *mode = match *mode {
            CameraMode::FirstPerson  => CameraMode::ThirdPerson,
            CameraMode::ThirdPerson  => CameraMode::FirstPerson,
        };
    }
}

/// Place la caméra selon le mode actif. En 3ème personne, elle recule et
/// monte, et prend un léger angle vers le bas pour voir le personnage.
fn update_third_person_camera(
    mode:            Res<CameraMode>,
    mut camera_q:    Query<&mut Transform, With<PlayerCamera>>,
    player_q:        Query<&Player>,
) {
    let Ok(mut ctrans) = camera_q.get_single_mut() else { return };
    let Ok(player)     = player_q.get_single()     else { return };

    match *mode {
        CameraMode::FirstPerson => {
            ctrans.translation = Vec3::new(0.0, 1.7, 0.0);
            ctrans.rotation    = Quat::from_rotation_x(player.pitch);
        }
        CameraMode::ThirdPerson => {
            ctrans.translation = Vec3::new(0.0, 2.8, 4.5);
            ctrans.rotation    = Quat::from_rotation_x(-0.22);
        }
    }
}

fn camera_look(
    mode:              Res<CameraMode>,
    mut mouse_motion:  EventReader<MouseMotion>,
    mut player_query:  Query<(&mut Transform, &mut Player)>,
    mut camera_query:  Query<&mut Transform, (With<PlayerCamera>, Without<Player>)>,
) {
    // On ne checke PAS `cursor_options.visible` : sur certains drivers Windows
    // winit peut laisser visible=true après un relock, ce qui bloquerait le
    // mouse-look à tort. Le `run_if(InGame)` du plugin est la seule garde.
    let mut delta = Vec2::ZERO;
    for ev in mouse_motion.read() {
        delta += ev.delta;
    }
    if delta == Vec2::ZERO { return; }

    const SENS: f32 = 0.002;

    if let (Ok((mut ptrans, mut player)), Ok(mut ctrans)) =
        (player_query.get_single_mut(), camera_query.get_single_mut())
    {
        player.yaw -= delta.x * SENS;
        ptrans.rotation = Quat::from_rotation_y(player.yaw);

        // En 3ème personne c'est `update_third_person_camera` qui pilote le
        // pitch — on ne le touche pas ici.
        if *mode == CameraMode::FirstPerson {
            player.pitch = (player.pitch - delta.y * SENS).clamp(-1.55, 1.55);
            ctrans.rotation = Quat::from_rotation_x(player.pitch);
        }
    }
}

fn player_move(
    keys:         Res<ButtonInput<KeyCode>>,
    mut player_q: Query<(&Transform, &mut Player, &PlayerStats), Without<NeedsGrounding>>,
) {
    let Ok((_, mut player, stats)) = player_q.get_single_mut() else { return };

    let speed = stats.speed;
    let yaw   = player.yaw;
    let fwd   = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new( yaw.cos(), 0.0, -yaw.sin());

    let mut dir = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) { dir += fwd;   }
    if keys.pressed(KeyCode::KeyS) { dir -= fwd;   }
    if keys.pressed(KeyCode::KeyA) { dir -= right;  }
    if keys.pressed(KeyCode::KeyD) { dir += right;  }

    if dir.length_squared() > 0.0 { dir = dir.normalize() * speed; }

    player.velocity.x = dir.x;
    player.velocity.z = dir.z;

    if keys.just_pressed(KeyCode::Space) && player.on_ground {
        player.velocity.y = 8.5;
        player.on_ground  = false;
    }
}

/// Applique la gravité hors-sol et recharge le mana en continu. La vitesse
/// de chute est clampée pour limiter les dégâts de fall (et les tunnels de
/// collision à haute vitesse).
fn apply_gravity(
    mut player_q: Query<(&mut Player, &mut PlayerStats), Without<NeedsGrounding>>,
    time:         Res<Time>,
) {
    let Ok((mut player, mut stats)) = player_q.get_single_mut() else { return };

    if !player.on_ground {
        player.velocity.y -= 22.0 * time.delta_secs();
        player.velocity.y  = player.velocity.y.max(-40.0);
    }
    let dt = time.delta_secs();
    stats.current_mana = (stats.current_mana + 3.0 * dt).min(stats.max_mana);
}

/// Déplace le joueur en résolvant la collision axe par axe (Y, puis X, puis
/// Z). Tester les axes séparément évite de se coincer dans un coin de mur.
fn apply_velocity(
    mut player_q:  Query<(&mut Transform, &mut Player), Without<NeedsGrounding>>,
    chunk_manager: Res<ChunkManager>,
    chunk_q:       Query<&Chunk>,
    time:          Res<Time>,
) {
    let Ok((mut transform, mut player)) = player_q.get_single_mut() else { return };

    let dt  = time.delta_secs();
    let vel = player.velocity;

    let new_y = transform.translation.y + vel.y * dt;

    if vel.y <= 0.0 {
        // Descente : snap sur le dessus du bloc dès qu'un bloc solide est
        // sous les pieds. Le -0.05 est la marge de tolérance pour accrocher
        // le sol sans "flotter" d'un pixel.
        let feet = Vec3::new(transform.translation.x, new_y - 0.05, transform.translation.z);
        if is_solid(&chunk_manager, &chunk_q, feet) {
            transform.translation.y = feet.y.floor() + 1.01;
            player.velocity.y       = 0.0;
            player.on_ground        = true;
        } else {
            transform.translation.y = new_y;
            player.on_ground        = false;
        }
    } else {
        // Montée : si la tête heurte un plafond, on stoppe la Y mais on
        // laisse la position inchangée pour éviter de rester coincé.
        let head = Vec3::new(transform.translation.x, new_y + 1.80, transform.translation.z);
        if is_solid(&chunk_manager, &chunk_q, head) {
            player.velocity.y = 0.0;
        } else {
            transform.translation.y = new_y;
            player.on_ground        = false;
        }
    }

    // Plancher de sécurité : si un chunk n'est pas chargé, `is_solid` renvoie
    // false et on tomberait à l'infini. y=0 évite ça le temps que ça charge.
    if transform.translation.y < 0.0 {
        transform.translation.y = 0.0;
        player.velocity.y       = 0.0;
        player.on_ground        = true;
    }

    // Axes X et Z : on teste deux points (bas et haut du torse) pour ne pas
    // pouvoir passer par-dessus un mur d'un seul bloc en collant le bord.
    let new_x = transform.translation.x + vel.x * dt;
    let test_x_lo = Vec3::new(new_x + vel.x.signum() * 0.30, transform.translation.y + 0.1, transform.translation.z);
    let test_x_hi = Vec3::new(new_x + vel.x.signum() * 0.30, transform.translation.y + 1.6, transform.translation.z);
    if is_solid(&chunk_manager, &chunk_q, test_x_lo)
    || is_solid(&chunk_manager, &chunk_q, test_x_hi) {
        player.velocity.x = 0.0;
    } else {
        transform.translation.x = new_x;
    }

    let new_z = transform.translation.z + vel.z * dt;
    let test_z_lo = Vec3::new(transform.translation.x, transform.translation.y + 0.1, new_z + vel.z.signum() * 0.30);
    let test_z_hi = Vec3::new(transform.translation.x, transform.translation.y + 1.6, new_z + vel.z.signum() * 0.30);
    if is_solid(&chunk_manager, &chunk_q, test_z_lo)
    || is_solid(&chunk_manager, &chunk_q, test_z_hi) {
        player.velocity.z = 0.0;
    } else {
        transform.translation.z = new_z;
    }
}

/// Renvoie `true` si la position monde est dans un bloc solide. Y<0 est
/// traité comme solide (plancher implicite) et Y>=CHUNK_HEIGHT comme vide.
pub fn is_solid(
    chunk_manager: &ChunkManager,
    chunk_q:       &Query<&Chunk>,
    pos:           Vec3,
) -> bool {
    let x = pos.x.floor() as i32;
    let y = pos.y.floor() as i32;
    let z = pos.z.floor() as i32;

    if y < 0               { return true;  }
    if y >= CHUNK_HEIGHT as i32 { return false; }

    let cx = x.div_euclid(CHUNK_SIZE as i32);
    let cz = z.div_euclid(CHUNK_SIZE as i32);
    let lx = x.rem_euclid(CHUNK_SIZE as i32) as usize;
    let lz = z.rem_euclid(CHUNK_SIZE as i32) as usize;

    if let Some(&entity) = chunk_manager.loaded.get(&(cx, cz)) {
        if let Ok(chunk) = chunk_q.get(entity) {
            return chunk.blocks[lx][y as usize][lz].is_solid();
        }
    }
    false
}
