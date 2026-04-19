//! Sauvegarde / Chargement multi-slot Alpha.
//!
//! Format texte plat dans `saves/save_<id>.txt` :
//!
//! ```text
//! race=2
//! name=Aventurier
//! px=12.5
//! py=64.0
//! pz=-4.2
//! hp=85.0
//! mana=70.0
//! ```
//!
//! Touches : F2 = sauvegarder dans le slot courant.

use bevy::app::AppExit;
use bevy::prelude::*;
use std::fs;
use std::path::PathBuf;

use crate::GameState;
use crate::PlayerConfig;
use crate::player::{NeedsGrounding, Player, PlayerStats};
use crate::race::Race;
use crate::world::chunk::{BlockEdits, block_to_id, id_to_block};

pub const SAVE_DIR: &str = "saves";
pub const LEGACY_SAVE_FILE: &str = "save.txt";

fn slot_path(id: u32) -> PathBuf {
    PathBuf::from(SAVE_DIR).join(format!("save_{id}.txt"))
}

fn edits_path(id: u32) -> PathBuf {
    PathBuf::from(SAVE_DIR).join(format!("save_{id}.edits"))
}

/// Sérialise toutes les modifications de blocs au format texte plat :
/// `cx cz lx ly lz id` (un edit par ligne).
fn write_edits_file(id: u32, edits: &BlockEdits) {
    let mut body = String::new();
    for ((cx, cz), chunk_edits) in &edits.map {
        for ((lx, ly, lz), block) in chunk_edits {
            body.push_str(&format!("{} {} {} {} {} {}\n",
                cx, cz, lx, ly, lz, block_to_id(*block)));
        }
    }
    let path = edits_path(id);
    if let Err(e) = fs::write(&path, body) {
        warn!("Echec ecriture edits {} : {e}", path.display());
    }
}

/// Lit un fichier `.edits` (peut ne pas exister → renvoie des edits vides).
pub fn read_edits_file(id: u32) -> BlockEdits {
    let mut out = BlockEdits::default();
    let Ok(body) = fs::read_to_string(edits_path(id)) else { return out };
    for line in body.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 6 { continue; }
        let (Ok(cx), Ok(cz), Ok(lx), Ok(ly), Ok(lz), Ok(bid)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<i32>(),
            parts[2].parse::<u8>(),
            parts[3].parse::<u8>(),
            parts[4].parse::<u8>(),
            parts[5].parse::<u8>(),
        ) else { continue };
        out.map.entry((cx, cz)).or_default().insert((lx, ly, lz), id_to_block(bid));
    }
    out
}

/// Migration : si un ancien `save.txt` traîne à la racine (ancienne version
/// mono-slot), on le déplace vers `saves/save_0.txt` (ou prochain libre si
/// déjà occupé). Idempotent : ne fait rien si le fichier n'existe pas.
pub fn migrate_legacy_save() {
    let legacy = PathBuf::from(LEGACY_SAVE_FILE);
    if !legacy.exists() { return; }
    if let Err(e) = fs::create_dir_all(SAVE_DIR) {
        warn!("Migration save : echec creation dossier {SAVE_DIR} : {e}");
        return;
    }
    let id = next_free_slot();
    let dest = slot_path(id);
    match fs::rename(&legacy, &dest) {
        Ok(_)  => info!("Ancienne sauvegarde migree vers {}", dest.display()),
        Err(e) => warn!("Echec migration save.txt -> {} : {e}", dest.display()),
    }
}

/// Slot de sauvegarde actuellement utilisé pour la partie en cours.
/// `None` => nouvelle partie pas encore sauvegardée (le premier F2 lui
/// assignera un slot libre).
#[derive(Resource, Default, Clone, Copy)]
pub struct CurrentSaveSlot(pub Option<u32>);

/// Données extraites d'un fichier de sauvegarde.
#[derive(Resource, Clone, Debug)]
pub struct PendingLoad {
    pub race: Race,
    pub name: String,
    pub px: f32, pub py: f32, pub pz: f32,
    pub hp: f32, pub mana: f32,
}

/// Métadonnées d'un slot listable depuis le menu.
#[derive(Clone, Debug)]
pub struct SaveSlotInfo {
    pub id:   u32,
    pub data: PendingLoad,
}

/// Vrai s'il existe au moins une sauvegarde lisible.
pub fn any_save_exists() -> bool {
    !list_saves().is_empty()
}

/// Liste tous les slots sauvegardés (triés par id croissant).
pub fn list_saves() -> Vec<SaveSlotInfo> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(SAVE_DIR) else { return out };
    for entry in rd.flatten() {
        let path = entry.path();
        // Ne lire que les fichiers .txt (ignorer les .edits sidecar).
        if path.extension().and_then(|e| e.to_str()) != Some("txt") { continue; }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
        let Some(id_str) = stem.strip_prefix("save_") else { continue };
        let Ok(id) = id_str.parse::<u32>() else { continue };
        if let Some(data) = read_save_slot(id) {
            out.push(SaveSlotInfo { id, data });
        }
    }
    out.sort_by_key(|s| s.id);
    out
}

/// Renvoie le plus petit id non utilisé.
pub fn next_free_slot() -> u32 {
    let used: std::collections::HashSet<u32> = list_saves().iter().map(|s| s.id).collect();
    let mut id = 0u32;
    while used.contains(&id) { id += 1; }
    id
}

/// Lit `saves/save_<id>.txt`.
pub fn read_save_slot(id: u32) -> Option<PendingLoad> {
    let body = fs::read_to_string(slot_path(id)).ok()?;

    let mut race_id = 0u8;
    let mut name    = String::from("Aventurier");
    let mut px: f32 = 8.0; let mut py: f32 = 200.0; let mut pz: f32 = 8.0;
    let mut hp: f32 = 100.0; let mut mana: f32 = 100.0;

    for line in body.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "race" => race_id = v.trim().parse().unwrap_or(0),
            "name" => name    = v.trim().to_string(),
            "px"   => px      = v.trim().parse().unwrap_or(px),
            "py"   => py      = v.trim().parse().unwrap_or(py),
            "pz"   => pz      = v.trim().parse().unwrap_or(pz),
            "hp"   => hp      = v.trim().parse().unwrap_or(hp),
            "mana" => mana    = v.trim().parse().unwrap_or(mana),
            _      => {}
        }
    }

    Some(PendingLoad {
        race: id_to_race(race_id),
        name, px, py, pz, hp, mana,
    })
}

/// Supprime un slot. No-op si absent.
pub fn delete_save_slot(id: u32) {
    let _ = fs::remove_file(slot_path(id));
    let _ = fs::remove_file(edits_path(id));
}

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        // Migration mono-slot -> multi-slot. Une seule fois au démarrage.
        migrate_legacy_save();
        app.init_resource::<CurrentSaveSlot>()
           .add_systems(
               Update,
               save_on_key.run_if(in_state(GameState::InGame)),
           )
           // Autosave : avant le cleanup du monde lors du retour au menu.
           .add_systems(OnEnter(GameState::MainMenu), autosave_on_leave.before(crate::cleanup_world_on_main_menu))
           // Autosave : sur AppExit (Quitter depuis pause / fermeture fenêtre).
           .add_systems(Last, autosave_on_app_exit);
    }
}

pub fn race_to_id(r: &Race) -> u8 {
    match r {
        Race::Sylvaris  => 0,
        Race::Ignaar    => 1,
        Race::Aethyn    => 2,
        Race::Vorkai    => 3,
        Race::Crysthari => 4,
    }
}
pub fn id_to_race(id: u8) -> Race {
    match id {
        0 => Race::Sylvaris,
        1 => Race::Ignaar,
        2 => Race::Aethyn,
        3 => Race::Vorkai,
        _ => Race::Crysthari,
    }
}

/// Sérialise et écrit l'état courant du joueur vers un slot. Assigne un
/// slot libre si la partie n'en a pas encore. No-op si pas de joueur.
fn write_save(
    cfg:      &PlayerConfig,
    slot:     &mut CurrentSaveSlot,
    tf:       &Transform,
    stats:    &PlayerStats,
    edits:    &BlockEdits,
) {
    let id = match slot.0 {
        Some(id) => id,
        None => {
            let id = next_free_slot();
            slot.0 = Some(id);
            id
        }
    };

    let body = format!(
        "race={}\nname={}\npx={}\npy={}\npz={}\nhp={}\nmana={}\n",
        race_to_id(&cfg.race),
        cfg.name,
        tf.translation.x,
        tf.translation.y,
        tf.translation.z,
        stats.current_hp,
        stats.current_mana,
    );

    if let Err(e) = fs::create_dir_all(SAVE_DIR) {
        warn!("Echec creation dossier saves : {e}");
        return;
    }
    let path = slot_path(id);
    if let Err(e) = fs::write(&path, body) {
        warn!("Echec sauvegarde slot {id} : {e}");
    } else {
        info!("Partie sauvegardee dans {}", path.display());
    }
    write_edits_file(id, edits);
}

fn save_on_key(
    keys:     Res<ButtonInput<KeyCode>>,
    cfg:      Res<PlayerConfig>,
    mut slot: ResMut<CurrentSaveSlot>,
    player_q: Query<(&Transform, &PlayerStats), With<Player>>,
    edits:    Res<BlockEdits>,
) {
    if !keys.just_pressed(KeyCode::F2) { return; }
    let Ok((tf, stats)) = player_q.get_single() else { return };
    write_save(&cfg, &mut slot, tf, stats, &edits);
}

/// Autosave lors du retour au menu principal (avant le cleanup du joueur).
/// `PlayerConfig` / `CurrentSaveSlot` peuvent ne pas encore exister lors du
/// tout premier `OnEnter(MainMenu)` (état initial), d'où les `Option`.
fn autosave_on_leave(
    cfg:      Option<Res<PlayerConfig>>,
    slot:     Option<ResMut<CurrentSaveSlot>>,
    player_q: Query<(&Transform, &PlayerStats), With<Player>>,
    edits:    Option<Res<BlockEdits>>,
) {
    let (Some(cfg), Some(mut slot), Some(edits)) = (cfg, slot, edits) else { return };
    let Ok((tf, stats)) = player_q.get_single() else { return };
    write_save(&cfg, &mut slot, tf, stats, &edits);
}

/// Autosave si un AppExit a été émis cette frame et qu'une partie est en cours.
fn autosave_on_app_exit(
    mut events: EventReader<AppExit>,
    cfg:        Option<Res<PlayerConfig>>,
    slot:       Option<ResMut<CurrentSaveSlot>>,
    player_q:   Query<(&Transform, &PlayerStats), With<Player>>,
    edits:      Option<Res<BlockEdits>>,
) {
    if events.read().next().is_none() { return; }
    let (Some(cfg), Some(mut slot), Some(edits)) = (cfg, slot, edits) else { return };
    let Ok((tf, stats)) = player_q.get_single() else { return };
    write_save(&cfg, &mut slot, tf, stats, &edits);
}

/// Helper utilisé après chargement depuis le menu pour téléporter le
/// joueur (utile aussi pour un futur F4 in-game si on en remet un).
#[allow(dead_code)]
pub fn apply_pending_load_to_player(
    commands: &mut Commands,
    cfg:      &mut PlayerConfig,
    player_q: &mut Query<(Entity, &mut Transform, &mut PlayerStats), With<Player>>,
    data:     &PendingLoad,
) {
    cfg.race = data.race.clone();
    cfg.name = data.name.clone();
    if let Ok((entity, mut tf, mut stats)) = player_q.get_single_mut() {
        tf.translation = Vec3::new(data.px, 200.0, data.pz);
        stats.current_hp   = data.hp;
        stats.current_mana = data.mana;
        commands.entity(entity).insert(NeedsGrounding);
    }
}
