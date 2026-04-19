//! Système de sorts : barre de 5 slots, lancer via R, cooldowns et états de
//! buffs.
//!
//! Tous les sorts partagent la même boucle : consommer le mana, armer le
//! cooldown du slot, puis exécuter l'effet dans un `match` sur `SpellId`. Les
//! buffs durables (Voile d'Ombre, Bouclier d'Eau, Flash) sont stockés dans
//! des ressources dédiées parce qu'ils survivent au frame de cast et doivent
//! être décomptés chaque frame.
//!
//! Les projectiles sont des entités `Projectile` avec une `velocity`, une
//! `lifetime` et un `ProjectileKind` qui dicte à la fois leur apparence et
//! leur comportement au contact (Fire/Ice/Drain/Wind creusent le terrain,
//! Light traverse sans dégât).

use bevy::prelude::*;
use crate::GameState;
use crate::player::{Player, PlayerCamera, PlayerStats};
use crate::race::Race;
use crate::world::chunk::{build_mesh, Block, Chunk, ChunkManager, CHUNK_HEIGHT, CHUNK_SIZE};
use crate::particles::SpawnParticleBurst;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpellId {
    Fireball,
    IceShard,
    EarthWall,
    WindDash,
    LightHeal,
    LightBlind,
    ShadowCloak,
    ShadowDrain,
    WaterShield,
    FireNova,
    WindBlade,
    EarthSpike,
}

impl SpellId {
    pub fn name(self) -> &'static str {
        match self {
            SpellId::Fireball    => "Boule de Feu",
            SpellId::IceShard    => "Eclat de Glace",
            SpellId::EarthWall   => "Mur de Terre",
            SpellId::WindDash    => "Dash du Vent",
            SpellId::LightHeal   => "Soin Lumiere",
            SpellId::LightBlind  => "Eclat Aveuglant",
            SpellId::ShadowCloak => "Voile d'Ombre",
            SpellId::ShadowDrain => "Drain d'Ombre",
            SpellId::WaterShield => "Bouclier d'Eau",
            SpellId::FireNova    => "Nova de Feu",
            SpellId::WindBlade   => "Lame de Vent",
            SpellId::EarthSpike  => "Pic de Pierre",
        }
    }
    pub fn key_label(self) -> &'static str { "" }
    pub fn mana_cost(self) -> f32 {
        match self {
            SpellId::Fireball    => 20.,
            SpellId::IceShard    => 18.,
            SpellId::EarthWall   => 30.,
            SpellId::WindDash    => 15.,
            SpellId::LightHeal   => 35.,
            SpellId::LightBlind  => 22.,
            SpellId::ShadowCloak => 28.,
            SpellId::ShadowDrain => 25.,
            SpellId::WaterShield => 32.,
            SpellId::FireNova    => 40.,
            SpellId::WindBlade   => 16.,
            SpellId::EarthSpike  => 26.,
        }
    }
    pub fn cooldown_secs(self) -> f32 {
        match self {
            SpellId::Fireball    => 2.0,
            SpellId::IceShard    => 1.6,
            SpellId::EarthWall   => 8.0,
            SpellId::WindDash    => 3.0,
            SpellId::LightHeal   => 6.0,
            SpellId::LightBlind  => 5.0,
            SpellId::ShadowCloak => 12.0,
            SpellId::ShadowDrain => 4.0,
            SpellId::WaterShield => 10.0,
            SpellId::FireNova    => 9.0,
            SpellId::WindBlade   => 1.4,
            SpellId::EarthSpike  => 5.0,
        }
    }
    /// Teinte de fond du slot dans la barre de sorts. `dimmed=true` baisse
    /// l'alpha quand le sort est en cooldown pour un rendu "grisé".
    pub fn slot_color(self, dimmed: bool) -> Color {
        let a: f32 = if dimmed { 0.35 } else { 0.88 };
        match self {
            SpellId::Fireball    => Color::srgba(0.95, 0.35, 0.05, a),
            SpellId::IceShard    => Color::srgba(0.45, 0.85, 1.00, a),
            SpellId::EarthWall   => Color::srgba(0.55, 0.38, 0.18, a),
            SpellId::WindDash    => Color::srgba(0.75, 0.95, 0.95, a),
            SpellId::LightHeal   => Color::srgba(1.00, 0.92, 0.30, a),
            SpellId::LightBlind  => Color::srgba(1.00, 1.00, 0.85, a),
            SpellId::ShadowCloak => Color::srgba(0.50, 0.10, 0.85, a),
            SpellId::ShadowDrain => Color::srgba(0.35, 0.05, 0.55, a),
            SpellId::WaterShield => Color::srgba(0.20, 0.55, 0.95, a),
            SpellId::FireNova    => Color::srgba(1.00, 0.55, 0.15, a),
            SpellId::WindBlade   => Color::srgba(0.60, 1.00, 0.80, a),
            SpellId::EarthSpike  => Color::srgba(0.45, 0.30, 0.12, a),
        }
    }

    /// Loadout de départ selon la race. Chaque race démarre avec un kit
    /// cohérent avec son identité (Sylvaris = défensif/soin, Ignaar = feu…).
    pub fn default_loadout(race: &Race) -> [Option<SpellId>; 5] {
        match race {
            Race::Sylvaris  => [Some(SpellId::EarthWall),   Some(SpellId::LightHeal),   Some(SpellId::WaterShield), Some(SpellId::WindDash),    Some(SpellId::Fireball)],
            Race::Ignaar    => [Some(SpellId::Fireball),    Some(SpellId::EarthWall),   Some(SpellId::LightHeal),   Some(SpellId::ShadowDrain), Some(SpellId::WindDash)],
            Race::Aethyn    => [Some(SpellId::WindDash),    Some(SpellId::IceShard),    Some(SpellId::LightBlind),  Some(SpellId::LightHeal),   Some(SpellId::Fireball)],
            Race::Vorkai    => [Some(SpellId::ShadowCloak), Some(SpellId::ShadowDrain), Some(SpellId::Fireball),    Some(SpellId::WindDash),    Some(SpellId::LightHeal)],
            Race::Crysthari => [Some(SpellId::LightBlind),  Some(SpellId::LightHeal),   Some(SpellId::IceShard),    Some(SpellId::WaterShield), Some(SpellId::Fireball)],
        }
    }
}

#[derive(Resource)]
pub struct SpellBar {
    pub slots:    [Option<SpellId>; 5],
    pub selected: usize,
}

impl Default for SpellBar {
    fn default() -> Self {
        Self {
            slots: [
                Some(SpellId::Fireball),
                Some(SpellId::WindDash),
                Some(SpellId::LightHeal),
                Some(SpellId::ShadowCloak),
                Some(SpellId::EarthWall),
            ],
            selected: 0,
        }
    }
}

/// Cooldowns restants par slot (en secondes). 0 = prêt à lancer.
#[derive(Resource, Default)]
pub struct SpellCooldowns {
    pub remaining: [f32; 5],
}

/// État du Voile d'Ombre : tant que `remaining > 0`, le joueur gagne un boost
/// de vitesse (et à terme une réduction de dégâts contre les mobs).
#[derive(Resource, Default)]
pub struct CloakState {
    pub remaining: f32,
}

/// État du Bouclier d'Eau : régénération HP tant que `remaining > 0`.
#[derive(Resource, Default)]
pub struct WaterShieldState {
    pub remaining: f32,
}

/// Effet d'éblouissement : voile blanc plein écran dont l'alpha décroit de
/// `remaining / duration`. `duration` est conservée pour pouvoir calculer le
/// ratio sans que l'UI ait à connaître la durée initiale.
#[derive(Resource, Default)]
pub struct FlashState {
    pub remaining: f32,
    pub duration:  f32,
}

#[derive(Component)]
pub struct Projectile {
    pub velocity: Vec3,
    pub lifetime: f32,
    pub kind:     ProjectileKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProjectileKind {
    Fire,
    Ice,
    Drain,
    Light,
    Wind,
}

pub struct MagicPlugin;

impl Plugin for MagicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpellBar>()
           .init_resource::<SpellCooldowns>()
           .init_resource::<CloakState>()
           .init_resource::<WaterShieldState>()
           .init_resource::<FlashState>()
           .add_systems(OnEnter(GameState::Loading), reset_spellbar_for_race)
           .add_systems(
               Update,
               (
                   spell_select,
                   spell_cast,
                   update_cooldowns,
                   tick_cloak,
                   tick_water_shield,
                   tick_flash,
                   move_projectiles,
               )
               .chain()
               .run_if(in_state(GameState::InGame)),
           );
    }
}

/// Réinitialise la barre de sorts avec le loadout de la race choisie.
/// Déclenché à chaque nouvelle partie pour qu'un changement de race entre
/// deux parties soit bien pris en compte.
fn reset_spellbar_for_race(
    mut bar:       ResMut<SpellBar>,
    mut cooldowns: ResMut<SpellCooldowns>,
    cfg:           Res<crate::PlayerConfig>,
) {
    bar.slots    = SpellId::default_loadout(&cfg.race);
    bar.selected = 0;
    cooldowns.remaining = [0.0; 5];
}

/// Navigation dans la barre de sorts : Z précédent, X suivant. Wrap-around
/// aux deux bouts. (Sur AZERTY, Z physique = KeyW, mais on garde KeyZ pour
/// qu'un layout QWERTY réponde à la touche à gauche du X physique.)
fn spell_select(
    keys:       Res<ButtonInput<KeyCode>>,
    mut spells: ResMut<SpellBar>,
) {
    let n = spells.slots.len();
    if n == 0 { return; }
    if keys.just_pressed(KeyCode::KeyZ) {
        spells.selected = (spells.selected + n - 1) % n;
    }
    if keys.just_pressed(KeyCode::KeyX) {
        spells.selected = (spells.selected + 1) % n;
    }
}

/// Lance le sort actuellement sélectionné à l'appui de R. Vérifie cooldown
/// puis mana, débite les deux, puis exécute l'effet dans un match dédié.
fn spell_cast(
    keys:           Res<ButtonInput<KeyCode>>,
    mut player_q:   Query<(&mut Player, &mut PlayerStats, &Transform)>,
    camera_q:       Query<&GlobalTransform, With<PlayerCamera>>,
    mut commands:   Commands,
    mut meshes:     ResMut<Assets<Mesh>>,
    mut materials:  ResMut<Assets<StandardMaterial>>,
    spells:         Res<SpellBar>,
    mut cooldowns:  ResMut<SpellCooldowns>,
    mut cloak:      ResMut<CloakState>,
    mut shield:     ResMut<WaterShieldState>,
    mut flash:      ResMut<FlashState>,
    chunk_manager:  Res<ChunkManager>,
    mut chunk_q:    Query<(&mut Chunk, &Mesh3d)>,
    mut edits:      ResMut<crate::world::chunk::BlockEdits>,
    mut bursts:     EventWriter<SpawnParticleBurst>,
) {
    if !keys.just_pressed(KeyCode::KeyR) { return; }
    let Some(spell_id) = spells.slots[spells.selected] else { return };
    let Ok((mut player, mut stats, ptf)) = player_q.get_single_mut() else { return };
    let Ok(cam_gt)                        = camera_q.get_single()     else { return };

    let idx = spells.selected;
    if cooldowns.remaining[idx] > 0.0 { return; }
    if stats.current_mana < spell_id.mana_cost() { return; }

    stats.current_mana       -= spell_id.mana_cost();
    cooldowns.remaining[idx]  = spell_id.cooldown_secs();

    let origin    = cam_gt.translation();
    let direction = cam_gt.forward().as_vec3();

    // Burst de particules au point de départ — signale visuellement le cast
    // même pour les sorts sans projectile (heal, cloak…).
    let cast_color = spell_cast_color(spell_id);
    bursts.send(SpawnParticleBurst {
        position: origin + direction * 1.0,
        color:    cast_color,
        count:    18,
        speed:    3.5,
        lifetime: 0.6,
    });

    match spell_id {
        SpellId::Fireball => {
            spawn_projectile(&mut commands, &mut meshes, &mut materials,
                origin + direction * 1.2, direction * 24.0, ProjectileKind::Fire);
        }
        SpellId::IceShard => {
            spawn_projectile(&mut commands, &mut meshes, &mut materials,
                origin + direction * 1.2, direction * 30.0, ProjectileKind::Ice);
        }
        SpellId::ShadowDrain => {
            // Soin instantané partiel + projectile. Le gros du "drain" sera
            // appliqué à l'impact dans une version future ; pour l'instant
            // le soin immédiat compense le fait que les mobs ne renvoient rien.
            spawn_projectile(&mut commands, &mut meshes, &mut materials,
                origin + direction * 1.2, direction * 18.0, ProjectileKind::Drain);
            stats.current_hp = (stats.current_hp + 8.0).min(stats.max_hp);
        }
        SpellId::WindDash => {
            player.velocity.x += direction.x * 20.0;
            player.velocity.z += direction.z * 20.0;
            // +5 min en Y pour assurer un saut même si le regard est plat.
            player.velocity.y  = (player.velocity.y + direction.y * 8.0 + 5.0).max(5.0);
        }
        SpellId::LightHeal => {
            stats.current_hp = (stats.current_hp + 30.0).min(stats.max_hp);
        }
        SpellId::LightBlind => {
            flash.remaining = 0.6;
            flash.duration  = 0.6;
            spawn_projectile(&mut commands, &mut meshes, &mut materials,
                origin + direction * 1.2, direction * 28.0, ProjectileKind::Light);
        }
        SpellId::ShadowCloak => {
            cloak.remaining = 8.0;
        }
        SpellId::WaterShield => {
            shield.remaining = 6.0;
        }
        SpellId::FireNova => {
            // Explosion sphérique centrée sur le joueur. Rayon 2.5, décalée
            // d'1m vers le haut pour ne pas effacer uniquement les pieds.
            let center = ptf.translation + Vec3::Y * 1.0;
            let r = 2.5f32;
            let rr = (r * r) as i32;
            let cx0 = center.x.floor() as i32;
            let cy0 = center.y.floor() as i32;
            let cz0 = center.z.floor() as i32;
            for dx in -3..=3i32 {
                for dy in -3..=3i32 {
                    for dz in -3..=3i32 {
                        if dx * dx + dy * dy + dz * dz > rr { continue; }
                        let bp = IVec3::new(cx0 + dx, cy0 + dy, cz0 + dz);
                        if set_block_at(bp, Block::Air, &chunk_manager, &mut chunk_q, &mut meshes) {
                            edits.record(bp, Block::Air);
                        }
                    }
                }
            }
            // Petit flash partagé avec LightBlind pour l'effet "détonation".
            flash.remaining = 0.35;
            flash.duration  = 0.35;
        }
        SpellId::WindBlade => {
            spawn_projectile(&mut commands, &mut meshes, &mut materials,
                origin + direction * 1.2, direction * 42.0, ProjectileKind::Wind);
        }
        SpellId::EarthSpike => {
            // Colonne verticale de 4 blocs de pierre à 2.5m devant le joueur.
            // On part du Y du joueur et on monte — pas de détection de sol
            // précise pour garder l'effet "instantané" du sort.
            let mut horiz = Vec3::new(direction.x, 0.0, direction.z);
            if horiz.length_squared() < 0.001 { horiz = Vec3::Z; }
            horiz = horiz.normalize();
            let base = origin + horiz * 2.5;
            let bx = base.x.floor() as i32;
            let bz = base.z.floor() as i32;
            let start_y = origin.y.floor() as i32;
            for dy in 0..=3i32 {
                let bp = IVec3::new(bx, start_y + dy, bz);
                if set_block_at(bp, Block::Stone, &chunk_manager, &mut chunk_q, &mut meshes) {
                    edits.record(bp, Block::Stone);
                }
            }
        }
        SpellId::EarthWall => {
            // Mur 3x3 en pierre devant le joueur, perpendiculaire à la
            // direction horizontale du regard. `right` est le vecteur droit
            // obtenu par rotation 90° horizontale de `horiz`.
            let mut horiz = Vec3::new(direction.x, 0.0, direction.z);
            if horiz.length_squared() < 0.001 { horiz = Vec3::Z; }
            horiz = horiz.normalize();
            let right = Vec3::new(-horiz.z, 0.0, horiz.x);
            let center = origin + horiz * 2.5;

            for du in -1..=1i32 {
                for dy in 0..=2i32 {
                    let p = center + right * (du as f32) + Vec3::Y * (dy as f32);
                    let bp = IVec3::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                    if set_block_at(bp, Block::Stone, &chunk_manager, &mut chunk_q, &mut meshes) {
                        edits.record(bp, Block::Stone);
                    }
                }
            }
            let _ = &mut materials;
        }
    }
}

fn spell_cast_color(spell: SpellId) -> Color {
    match spell {
        SpellId::Fireball | SpellId::FireNova                 => Color::srgb(1.0, 0.45, 0.05),
        SpellId::IceShard                                     => Color::srgb(0.5, 0.85, 1.0),
        SpellId::EarthWall | SpellId::EarthSpike              => Color::srgb(0.55, 0.45, 0.30),
        SpellId::WindDash | SpellId::WindBlade                => Color::srgb(0.70, 1.0, 0.85),
        SpellId::LightHeal | SpellId::LightBlind              => Color::srgb(1.0, 1.0, 0.85),
        SpellId::ShadowCloak | SpellId::ShadowDrain           => Color::srgb(0.55, 0.10, 0.75),
        SpellId::WaterShield                                  => Color::srgb(0.30, 0.55, 0.95),
    }
}

fn projectile_burst_color(kind: ProjectileKind) -> Color {
    match kind {
        ProjectileKind::Fire  => Color::srgb(1.0, 0.45, 0.05),
        ProjectileKind::Ice   => Color::srgb(0.5, 0.85, 1.0),
        ProjectileKind::Drain => Color::srgb(0.55, 0.10, 0.75),
        ProjectileKind::Light => Color::srgb(1.0, 1.0, 0.85),
        ProjectileKind::Wind  => Color::srgb(0.70, 1.0, 0.85),
    }
}

/// Spawn un projectile avec son rendu et sa PointLight enfant. Chaque type a
/// son couple (base_color, emissive, light_color) pour que la trace lumineuse
/// corresponde visuellement au sort.
fn spawn_projectile(
    commands:  &mut Commands,
    meshes:    &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    pos:       Vec3,
    velocity:  Vec3,
    kind:      ProjectileKind,
) {
    let (color, emissive, light) = match kind {
        ProjectileKind::Fire  => (Color::srgb(1.0, 0.45, 0.05), LinearRgba::new(4.0, 1.2, 0.0, 1.0), Color::srgb(1.0, 0.5, 0.1)),
        ProjectileKind::Ice   => (Color::srgb(0.5, 0.85, 1.0), LinearRgba::new(0.5, 1.5, 4.0, 1.0), Color::srgb(0.4, 0.8, 1.0)),
        ProjectileKind::Drain => (Color::srgb(0.45, 0.05, 0.65), LinearRgba::new(1.5, 0.0, 2.5, 1.0), Color::srgb(0.6, 0.1, 0.9)),
        ProjectileKind::Light => (Color::srgb(1.0, 1.0, 0.85), LinearRgba::new(5.0, 5.0, 3.0, 1.0), Color::srgb(1.0, 1.0, 0.85)),
        ProjectileKind::Wind  => (Color::srgb(0.70, 1.0, 0.85), LinearRgba::new(1.2, 3.0, 2.0, 1.0), Color::srgb(0.7, 1.0, 0.85)),
    };
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(0.22))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: color,
            emissive,
            ..default()
        })),
        Transform::from_translation(pos),
        Projectile { velocity, lifetime: 4.0, kind },
    )).with_children(|fb| {
        fb.spawn((
            PointLight { color: light, intensity: 80_000., radius: 0.3, ..default() },
            Transform::default(),
        ));
    });
}

/// Pose un bloc dans le chunk correspondant et reconstruit son mesh.
/// Renvoie `false` si le Y est hors du chunk ou si le chunk n'est pas chargé.
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

fn update_cooldowns(time: Res<Time>, mut cooldowns: ResMut<SpellCooldowns>) {
    let dt = time.delta_secs();
    for cd in cooldowns.remaining.iter_mut() {
        if *cd > 0.0 { *cd = (*cd - dt).max(0.0); }
    }
}

/// Décompte le Voile d'Ombre et applique son effet de vitesse. Restaurer la
/// vitesse de base à la fin est important : sinon un respawn depuis un cast
/// précédent garderait le boost indéfiniment.
fn tick_cloak(
    time:        Res<Time>,
    mut cloak:   ResMut<CloakState>,
    mut stats_q: Query<&mut PlayerStats>,
    cfg:         Res<crate::PlayerConfig>,
) {
    if cloak.remaining <= 0.0 { return; }
    let dt = time.delta_secs();
    cloak.remaining = (cloak.remaining - dt).max(0.0);

    let Ok(mut stats) = stats_q.get_single_mut() else { return };
    let base_speed = cfg.race.base_stats().speed * 5.0;
    if cloak.remaining > 0.0 {
        stats.speed = base_speed * 1.5;
        // Petit coût mana progressif pour ne pas "gratuité totale".
        stats.current_mana = (stats.current_mana - 1.0 * dt).max(0.0);
    } else {
        stats.speed = base_speed;
    }
}

fn tick_water_shield(
    time:        Res<Time>,
    mut shield:  ResMut<WaterShieldState>,
    mut stats_q: Query<&mut PlayerStats>,
) {
    if shield.remaining <= 0.0 { return; }
    let dt = time.delta_secs();
    shield.remaining = (shield.remaining - dt).max(0.0);
    if let Ok(mut stats) = stats_q.get_single_mut() {
        stats.current_hp = (stats.current_hp + 4.0 * dt).min(stats.max_hp);
    }
}

fn tick_flash(time: Res<Time>, mut flash: ResMut<FlashState>) {
    if flash.remaining <= 0.0 { return; }
    flash.remaining = (flash.remaining - time.delta_secs()).max(0.0);
}

/// Avance les projectiles, détruit ceux qui touchent un bloc solide (avec
/// creusage du bloc pour Fire/Ice/Drain/Wind), et spawn un burst de
/// particules à l'impact. Light traverse sans rien détruire.
fn move_projectiles(
    mut commands:  Commands,
    mut fb_q:      Query<(Entity, &mut Transform, &mut Projectile)>,
    chunk_manager: Res<ChunkManager>,
    mut chunk_q:   Query<(&mut Chunk, &Mesh3d)>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut edits:     ResMut<crate::world::chunk::BlockEdits>,
    time:          Res<Time>,
    mut bursts:    EventWriter<SpawnParticleBurst>,
) {
    let dt = time.delta_secs();
    for (entity, mut tf, mut fb) in fb_q.iter_mut() {
        fb.lifetime -= dt;
        if fb.lifetime <= 0.0 {
            commands.entity(entity).despawn_recursive();
            continue;
        }

        tf.translation += fb.velocity * dt;

        let pos = tf.translation;
        let bx  = pos.x.floor() as i32;
        let by  = pos.y.floor() as i32;
        let bz  = pos.z.floor() as i32;

        if by < 0 || by >= CHUNK_HEIGHT as i32 {
            commands.entity(entity).despawn_recursive();
            continue;
        }

        let cx = bx.div_euclid(CHUNK_SIZE as i32);
        let cz = bz.div_euclid(CHUNK_SIZE as i32);
        let lx = bx.rem_euclid(CHUNK_SIZE as i32) as usize;
        let lz = bz.rem_euclid(CHUNK_SIZE as i32) as usize;

        if let Some(&chunk_ent) = chunk_manager.loaded.get(&(cx, cz)) {
            if let Ok((mut chunk, mesh3d)) = chunk_q.get_mut(chunk_ent) {
                if chunk.blocks[lx][by as usize][lz].is_solid() {
                    // Light traverse sans détruire — les autres kinds creusent.
                    if matches!(fb.kind, ProjectileKind::Fire | ProjectileKind::Ice | ProjectileKind::Drain | ProjectileKind::Wind) {
                        chunk.blocks[lx][by as usize][lz] = crate::world::chunk::Block::Air;
                        let new_mesh = build_mesh(&chunk.blocks);
                        if let Some(mesh) = meshes.get_mut(&mesh3d.0) {
                            *mesh = new_mesh;
                        }
                        edits.record(IVec3::new(bx, by, bz), crate::world::chunk::Block::Air);
                    }
                    bursts.send(SpawnParticleBurst {
                        position: tf.translation,
                        color:    projectile_burst_color(fb.kind),
                        count:    20,
                        speed:    4.5,
                        lifetime: 0.5,
                    });
                    commands.entity(entity).despawn_recursive();
                }
            }
        }
    }
}
