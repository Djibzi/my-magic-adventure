//! HUD : vie/mana, hotbar, barre de sorts, overlays flash et buffs, compteur FPS.
//!
//! Le HUD est un unique arbre de `Node` parenté à `HudRoot`, construit au
//! passage à `InGame` et détruit à l'entrée du `MainMenu` — volontairement
//! pas sur `OnExit(InGame)` parce que Pause et Inventory sortent aussi de
//! InGame mais doivent garder le HUD visible. Les overlays (FPS, flash,
//! buffs) sont des entités séparées pour pouvoir les toggler indépendamment.

use bevy::prelude::*;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};

use bevy::ui::widget::ImageNode;

use crate::GameState;
use crate::player::PlayerStats;
use crate::world::interaction::Hotbar;
use crate::world::chunk::{Block, BlockAtlas};
use crate::world::BlockIconAtlas;
use crate::magic::{SpellBar, SpellCooldowns};
use crate::magic::spells::{CloakState, FlashState, WaterShieldState};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
           .add_systems(OnEnter(GameState::InGame), (spawn_hud, spawn_fps_overlay, spawn_flash_overlay, spawn_buff_indicator))
           .add_systems(OnEnter(GameState::MainMenu), despawn_all_game_hud)
           .add_systems(
               Update,
               (update_hud, update_hotbar_visuals, update_spell_bar_ui, update_flash_overlay, update_buff_indicator)
                   .run_if(
                       in_state(GameState::InGame)
                       .or(in_state(GameState::Paused))
                       .or(in_state(GameState::Inventory))
                   ),
           )
           .add_systems(Update, (toggle_fps_overlay, update_fps_overlay));
    }
}

#[derive(Component)] pub struct HudRoot;
#[derive(Component)] pub struct HpBarFill;
#[derive(Component)] pub struct ManaBarFill;
#[derive(Component)] pub struct HotbarSlot(pub usize);
#[derive(Component)] pub struct HotbarIcon(pub usize);
#[derive(Component)] pub struct HotbarCount(pub usize);
#[derive(Component)] pub struct SelectedBlockText;
#[derive(Component)] pub struct SpellBarSlot(pub usize);
#[derive(Component)] pub struct SpellBarName(pub usize);
#[derive(Component)] pub struct SpellBarCooldownOverlay(pub usize);

fn block_name(block: Option<Block>) -> &'static str {
    match block {
        Some(Block::Grass)  => "Herbe",
        Some(Block::Dirt)   => "Terre",
        Some(Block::Stone)  => "Pierre",
        Some(Block::Sand)   => "Sable",
        Some(Block::Snow)   => "Neige",
        Some(Block::Wood)   => "Bois",
        Some(Block::Leaves) => "Feuilles",
        Some(Block::Planks) => "Planches",
        Some(Block::Ice)    => "Glace",
        Some(Block::Air)    => "Air",
        None                => "",
    }
}

/// Construit l'arbre HUD complet : crosshair, barres HP/mana, hotbar 9 slots,
/// barre de sorts 5 slots et libellé du bloc sélectionné.
fn spawn_hud(mut commands: Commands, hotbar: Res<Hotbar>, spell_bar: Res<SpellBar>, atlas: Res<BlockAtlas>, icon_atlas: Res<BlockIconAtlas>, existing: Query<Entity, With<HudRoot>>) {
    if !existing.is_empty() { return; }
    let atlas_img    = atlas.handle.clone();
    let atlas_layout = icon_atlas.layout.clone();
    commands
        .spawn((
            Node {
                width:         Val::Percent(100.),
                height:        Val::Percent(100.),
                position_type: PositionType::Absolute,
                ..default()
            },
            HudRoot,
        ))
        .with_children(|root| {
            // Crosshair : deux barres fines centrées. Bevy 0.15 n'a pas de
            // `transform: translate(-50%, -50%)` → on compense avec une marge
            // négative égale à la moitié de la taille du node.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width:  Val::Px(14.),
                    height: Val::Px(2.),
                    left:   Val::Percent(50.),
                    top:    Val::Percent(50.),
                    margin: UiRect {
                        left: Val::Px(-7.),
                        top:  Val::Px(-1.),
                        ..default()
                    },
                    ..default()
                },
                BackgroundColor(Color::srgba(1., 1., 1., 0.7)),
            ));
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width:  Val::Px(2.),
                    height: Val::Px(14.),
                    left:   Val::Percent(50.),
                    top:    Val::Percent(50.),
                    margin: UiRect {
                        left: Val::Px(-1.),
                        top:  Val::Px(-7.),
                        ..default()
                    },
                    ..default()
                },
                BackgroundColor(Color::srgba(1., 1., 1., 0.7)),
            ));

            // Barre de vie (rouge) en haut-gauche.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width:  Val::Px(200.),
                    height: Val::Px(16.),
                    top:    Val::Px(50.),
                    left:   Val::Px(20.),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
            ))
            .with_children(|bar| {
                bar.spawn((
                    Node {
                        width:  Val::Percent(100.),
                        height: Val::Percent(100.),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.8, 0.1, 0.1)),
                    HpBarFill,
                ));
            });

            // Barre de mana (bleu) juste sous la barre de vie.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width:  Val::Px(200.),
                    height: Val::Px(16.),
                    top:    Val::Px(72.),
                    left:   Val::Px(20.),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.2, 0.2, 0.2)),
            ))
            .with_children(|bar| {
                bar.spawn((
                    Node {
                        width:  Val::Percent(100.),
                        height: Val::Percent(100.),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.1, 0.2, 0.85)),
                    ManaBarFill,
                ));
            });

            // Hotbar bas-centre. 9 slots × 48px + 8 gaps × 4px = 464px de
            // large, donc décalage à gauche de la moitié = 232px.
            root.spawn((
                Node {
                    position_type:   PositionType::Absolute,
                    bottom:          Val::Px(20.),
                    left:            Val::Percent(50.),
                    margin:          UiRect { left: Val::Px(-232.), ..default() },
                    flex_direction:  FlexDirection::Row,
                    column_gap:      Val::Px(4.),
                    ..default()
                },
            ))
            .with_children(|hotbar_row| {
                for i in 0..9usize {
                    let sel_border = if i == hotbar.selected {
                        Color::WHITE
                    } else {
                        Color::srgba(0.3, 0.3, 0.3, 0.7)
                    };
                    let empty_bg = Color::srgba(0.15, 0.15, 0.15, 0.40);
                    let count = hotbar.slots[i].map(|(_, n)| n).unwrap_or(0);

                    hotbar_row.spawn((
                        Node {
                            width:           Val::Px(48.),
                            height:          Val::Px(48.),
                            align_items:     AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            border:          UiRect::all(Val::Px(2.)),
                            overflow:        Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(empty_bg),
                        BorderColor(sel_border),
                        HotbarSlot(i),
                    ))
                    .with_children(|slot| {
                        let (icon_display, icon_index) = if let Some((block, _)) = hotbar.slots[i] {
                            (Display::Flex, block.tile_side())
                        } else {
                            (Display::None, 0)
                        };
                        slot.spawn((
                            ImageNode {
                                image: atlas_img.clone(),
                                texture_atlas: Some(TextureAtlas {
                                    layout: atlas_layout.clone(),
                                    index:  icon_index,
                                }),
                                ..default()
                            },
                            Node {
                                width:  Val::Px(28.),
                                height: Val::Px(28.),
                                display: icon_display,
                                ..default()
                            },
                            HotbarIcon(i),
                        ));
                        let count_str = if count > 0 { format!("{}", count) } else { String::new() };
                        slot.spawn((
                            Text::new(count_str),
                            TextFont { font_size: 10., ..default() },
                            TextColor(Color::srgba(1., 1., 0.7, 0.95)),
                            Node {
                                position_type: PositionType::Absolute,
                                right:  Val::Px(2.),
                                bottom: Val::Px(1.),
                                ..default()
                            },
                            HotbarCount(i),
                        ));
                    });
                }
            });

            // Libellé du bloc sélectionné, juste au-dessus de la hotbar.
            let selected_name = block_name(hotbar.slots[hotbar.selected].map(|(b, _)| b));
            root.spawn((
                Node {
                    position_type:   PositionType::Absolute,
                    bottom:          Val::Px(74.),
                    left:            Val::Percent(50.),
                    margin:          UiRect { left: Val::Px(-60.), ..default() },
                    width:           Val::Px(120.),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(selected_name),
                    TextFont { font_size: 16., ..default() },
                    TextColor(Color::WHITE),
                    SelectedBlockText,
                ));
            });

            // Barre de sorts bas-gauche, 5 slots. Touches F1-F5 pour
            // sélectionner, Q pour lancer (gérés dans `magic`).
            root.spawn(Node {
                position_type:  PositionType::Absolute,
                bottom:         Val::Px(20.),
                left:           Val::Px(20.),
                flex_direction: FlexDirection::Row,
                column_gap:     Val::Px(4.),
                ..default()
            })
            .with_children(|bar| {
                for i in 0..5usize {
                    let spell  = spell_bar.slots[i];
                    let bg_col = spell.map(|s| s.slot_color(false))
                                      .unwrap_or(Color::srgba(0.12, 0.12, 0.12, 0.70));

                    bar.spawn((
                        Node {
                            width:           Val::Px(52.),
                            height:          Val::Px(52.),
                            flex_direction:  FlexDirection::Column,
                            align_items:     AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            border:          UiRect::all(Val::Px(2.)),
                            overflow:        Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(bg_col),
                        BorderColor(Color::srgba(0.5, 0.5, 0.5, 0.7)),
                        SpellBarSlot(i),
                    ))
                    .with_children(|slot| {
                        // Overlay de cooldown : sa hauteur (en %) correspond
                        // au cooldown restant, et il descend depuis le haut.
                        slot.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                width:         Val::Percent(100.),
                                height:        Val::Percent(0.),
                                top:           Val::Px(0.),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
                            SpellBarCooldownOverlay(i),
                        ));
                        let key = spell.map(|s| s.key_label()).unwrap_or("");
                        slot.spawn((
                            Text::new(key),
                            TextFont { font_size: 9., ..default() },
                            TextColor(Color::srgba(1., 1., 1., 0.6)),
                        ));
                        let name = spell.map(|s| s.name()).unwrap_or("");
                        slot.spawn((
                            Text::new(name),
                            TextFont { font_size: 7., ..default() },
                            TextColor(Color::WHITE),
                            SpellBarName(i),
                        ));
                    });
                }
            });
        });
}

fn update_hud(
    stats_q:    Query<&PlayerStats>,
    mut hp_q:   Query<&mut Node, (With<HpBarFill>, Without<ManaBarFill>)>,
    mut mana_q: Query<&mut Node, (With<ManaBarFill>, Without<HpBarFill>)>,
) {
    let Ok(stats) = stats_q.get_single() else { return };

    if let Ok(mut node) = hp_q.get_single_mut() {
        let pct = (stats.current_hp / stats.max_hp * 100.).clamp(0., 100.);
        node.width = Val::Percent(pct);
    }
    if let Ok(mut node) = mana_q.get_single_mut() {
        let pct = (stats.current_mana / stats.max_mana * 100.).clamp(0., 100.);
        node.width = Val::Percent(pct);
    }
}

/// Met à jour la barre de sorts : couleur de fond (grisée en cooldown),
/// bordure du slot sélectionné, hauteur de l'overlay cooldown et nom affiché.
fn update_spell_bar_ui(
    spells:     Res<SpellBar>,
    cooldowns:  Res<SpellCooldowns>,
    mut slot_q: Query<(&SpellBarSlot, &mut BackgroundColor, &mut BorderColor)>,
    mut cd_q:   Query<(&SpellBarCooldownOverlay, &mut Node)>,
    mut name_q: Query<(&SpellBarName, &mut Text)>,
) {
    for (slot, mut bg, mut border) in slot_q.iter_mut() {
        let i       = slot.0;
        let on_cd   = cooldowns.remaining[i] > 0.0;
        let color   = spells.slots[i]
            .map(|s| s.slot_color(on_cd))
            .unwrap_or(Color::srgba(0.12, 0.12, 0.12, 0.70));
        *bg     = BackgroundColor(color);
        *border = BorderColor(if i == spells.selected {
            Color::WHITE
        } else {
            Color::srgba(0.5, 0.5, 0.5, 0.7)
        });
    }
    for (cd_marker, mut node) in cd_q.iter_mut() {
        let i      = cd_marker.0;
        let max_cd = spells.slots[i].map(|s| s.cooldown_secs()).unwrap_or(1.0);
        let pct    = if max_cd > 0.0 { cooldowns.remaining[i] / max_cd * 100.0 } else { 0.0 };
        node.height = Val::Percent(pct);
    }
    for (name_marker, mut text) in name_q.iter_mut() {
        let i = name_marker.0;
        text.0 = spells.slots[i].map(|s| s.name()).unwrap_or("").to_string();
    }
}

fn update_hotbar_visuals(
    hotbar:      Res<Hotbar>,
    mut slot_q:  Query<(&HotbarSlot, &mut BorderColor)>,
    mut icon_q:  Query<(&HotbarIcon, &mut ImageNode, &mut Node), Without<HotbarSlot>>,
    mut count_q: Query<(&HotbarCount, &mut Text)>,
    mut sel_q:   Query<&mut Text, (With<SelectedBlockText>, Without<HotbarCount>)>,
) {
    for (slot, mut border) in slot_q.iter_mut() {
        *border = BorderColor(if slot.0 == hotbar.selected {
            Color::WHITE
        } else {
            Color::srgba(0.3, 0.3, 0.3, 0.7)
        });
    }
    for (icon, mut img, mut node) in icon_q.iter_mut() {
        if let Some((block, _)) = hotbar.slots[icon.0] {
            if let Some(ref mut atlas) = img.texture_atlas {
                atlas.index = block.tile_side();
            }
            node.display = Display::Flex;
        } else {
            node.display = Display::None;
        }
    }
    for (cnt, mut text) in count_q.iter_mut() {
        let n = hotbar.slots[cnt.0].map(|(_, n)| n).unwrap_or(0);
        text.0 = if n > 0 { format!("{}", n) } else { String::new() };
    }
    if let Ok(mut text) = sel_q.get_single_mut() {
        let name = block_name(hotbar.slots[hotbar.selected].map(|(b, _)| b));
        let count = hotbar.slots[hotbar.selected].map(|(_, n)| n).unwrap_or(0);
        text.0 = if count > 0 {
            format!("{} ({})", name, count)
        } else {
            name.to_string()
        };
    }
}

/// Compteur FPS en overlay haut-gauche, masqué par défaut. F3 pour basculer.
#[derive(Component)] pub struct FpsOverlay;

fn spawn_fps_overlay(mut commands: Commands, font: Res<crate::PixelFont>) {
    commands.spawn((
        Text::new("FPS: --"),
        TextFont { font: font.0.clone(), font_size: 14.0, ..default() },
        TextColor(Color::srgb(1.0, 1.0, 0.0)),
        Node {
            position_type: PositionType::Absolute,
            top:     Val::Px(8.0),
            left:    Val::Px(8.0),
            padding: UiRect::all(Val::Px(4.0)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        FpsOverlay,
    ));
}

fn toggle_fps_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    mut q:  Query<&mut Node, With<FpsOverlay>>,
) {
    if !keys.just_pressed(KeyCode::F3) { return; }
    for mut node in &mut q {
        node.display = if node.display == Display::None { Display::Flex } else { Display::None };
    }
}

fn update_fps_overlay(
    diagnostics: Res<DiagnosticsStore>,
    time:        Res<Time<Real>>,
    mut q: Query<&mut Text, With<FpsOverlay>>,
) {
    let Ok(mut text) = q.get_single_mut() else { return };

    // On essaie d'abord la valeur lissée de FrameTimeDiagnosticsPlugin, sinon
    // on retombe sur un calcul direct à partir du dt — utile au tout premier
    // frame où le plugin n'a pas encore accumulé de stats.
    let fps_val = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed().or_else(|| d.average()).or_else(|| d.value()))
        .or_else(|| {
            let dt = time.delta_secs_f64();
            if dt > 0.0 { Some(1.0 / dt) } else { None }
        });

    if let Some(v) = fps_val {
        text.0 = format!("FPS: {:.0}", v);
    }
}

/// Overlay plein écran pour le sort "Éclat aveuglant". Son alpha est piloté
/// par `FlashState.remaining / duration`.
#[derive(Component)] pub struct FlashOverlay;

fn spawn_flash_overlay(mut commands: Commands, existing: Query<Entity, With<FlashOverlay>>) {
    if !existing.is_empty() { return; }
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width:  Val::Percent(100.),
            height: Val::Percent(100.),
            ..default()
        },
        BackgroundColor(Color::srgba(1.0, 1.0, 0.95, 0.0)),
        FlashOverlay,
    ));
}

fn update_flash_overlay(
    flash:  Res<FlashState>,
    mut q:  Query<&mut BackgroundColor, With<FlashOverlay>>,
) {
    let Ok(mut bg) = q.get_single_mut() else { return };
    let alpha = if flash.duration > 0.0 {
        (flash.remaining / flash.duration).clamp(0.0, 1.0) * 0.85
    } else { 0.0 };
    *bg = BackgroundColor(Color::srgba(1.0, 1.0, 0.95, alpha));
}

/// Texte indicatif des buffs actifs (Voile d'Ombre / Bouclier d'Eau) avec
/// leur temps restant, affiché sous la barre de mana.
#[derive(Component)] pub struct BuffIndicator;

/// Nettoyage complet du HUD au retour au menu. Lié à `OnEnter(MainMenu)`
/// plutôt qu'à `OnExit(InGame)` parce que Pause et Inventory sortent aussi
/// de InGame mais doivent conserver le HUD.
fn despawn_all_game_hud(
    mut commands: Commands,
    hud_q:   Query<Entity, With<HudRoot>>,
    fps_q:   Query<Entity, With<FpsOverlay>>,
    flash_q: Query<Entity, With<FlashOverlay>>,
    buff_q:  Query<Entity, With<BuffIndicator>>,
) {
    for e in &hud_q   { commands.entity(e).despawn_recursive(); }
    for e in &fps_q   { commands.entity(e).despawn_recursive(); }
    for e in &flash_q { commands.entity(e).despawn_recursive(); }
    for e in &buff_q  { commands.entity(e).despawn_recursive(); }
}

fn spawn_buff_indicator(mut commands: Commands, font: Res<crate::PixelFont>, existing: Query<Entity, With<BuffIndicator>>) {
    if !existing.is_empty() { return; }
    commands.spawn((
        Text::new(""),
        TextFont { font: font.0.clone(), font_size: 11.0, ..default() },
        TextColor(Color::srgb(0.95, 0.95, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top:    Val::Px(60.0),
            left:   Val::Px(8.0),
            padding: UiRect::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
        BuffIndicator,
    ));
}

fn update_buff_indicator(
    cloak:  Res<CloakState>,
    shield: Res<WaterShieldState>,
    mut q:  Query<&mut Text, With<BuffIndicator>>,
) {
    let Ok(mut text) = q.get_single_mut() else { return };
    let mut s = String::new();
    if cloak.remaining  > 0.0 { s.push_str(&format!("Voile d'Ombre {:.1}s\n", cloak.remaining)); }
    if shield.remaining > 0.0 { s.push_str(&format!("Bouclier d'Eau {:.1}s",  shield.remaining)); }
    *text = Text::new(s);
}
