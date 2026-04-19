//! Point d'entrée du jeu et orchestration des états.
//!
//! Le flux principal est un simple pipeline d'états : `MainMenu` →
//! `SaveSelect` (optionnel) → `RaceSelect` → `Loading` → `InGame`. Depuis
//! `InGame` on peut ouvrir des overlays qui suspendent le jeu : `Paused`,
//! `Options`, `Inventory`, ou `Dead` quand le joueur meurt.
//!
//! Tout ce qui est spécifique à une mécanique (monde, joueur, magie, mobs,
//! craft, sauvegardes, particules…) vit dans son propre module/plugin. Ce
//! fichier se contente de câbler les plugins, d'exposer les ressources de
//! configuration (FOV, distance de rendu, race choisie), et de construire
//! l'UI des menus.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow, PresentMode, WindowResizeConstraints};
use bevy::winit::{WinitSettings, UpdateMode};
use bevy::ui::widget::ImageNode;

mod race;
mod magic;
mod world;
mod player;
mod ui;
mod systems;
mod combat;
mod crafting;
mod particles;

use world::WorldPlugin;
use player::PlayerPlugin;
use ui::HudPlugin;
use systems::{DayNightPlugin, SavePlugin};
use magic::MagicPlugin;
use race::ability::RaceAbilityPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "My Magic Adventure — Alpha v0.1".into(),
                resolution: (1280., 720.).into(),
                // Empêche le démarrage en fenêtre trop petite : sur Windows,
                // winit perd silencieusement le clavier/curseur jusqu'au
                // premier resize si la fenêtre s'ouvre trop petite.
                resize_constraints: WindowResizeConstraints {
                    min_width:  1024.,
                    min_height: 600.,
                    ..default()
                },
                // Fifo (vsync on) évite les stalls de swapchain sur les iGPU
                // Intel sous Windows. Le cap à 60 Hz est sans impact puisque
                // l'iGPU plafonne bien en dessous de toute façon.
                present_mode: PresentMode::Fifo,
                ..default()
            }),
            ..default()
        }))
        .add_plugins((WorldPlugin, PlayerPlugin, HudPlugin, DayNightPlugin, MagicPlugin, RaceAbilityPlugin, combat::MobPlugin, SavePlugin, crafting::CraftingPlugin, particles::ParticlesPlugin))
        // Force la boucle de mise à jour à tourner même sans focus fenêtre.
        // Sans ça, le rendu peut se figer après Loading→InGame jusqu'au
        // premier événement winit.
        .insert_resource(WinitSettings {
            focused_mode:   UpdateMode::Continuous,
            unfocused_mode: UpdateMode::Continuous,
        })
        .insert_resource(ClearColor(Color::srgb(0.52, 0.73, 0.95)))
        .init_state::<GameState>()
        .init_resource::<RenderDistanceConfig>()
        .init_resource::<FovConfig>()
        .init_resource::<SelectedRacePreview>()
        .init_resource::<OptionsFrom>()
        .init_resource::<PixelFont>()
        .init_resource::<BgHandle>()
        .add_systems(Startup, setup)
        .add_systems(OnEnter(GameState::MainMenu),   (cleanup_world_on_main_menu, spawn_main_menu).chain())
        .add_systems(OnExit(GameState::MainMenu),    despawn_main_menu)
        .add_systems(OnEnter(GameState::SaveSelect), spawn_save_select)
        .add_systems(OnExit(GameState::SaveSelect),  despawn_save_select)
        .add_systems(OnEnter(GameState::RaceSelect), spawn_race_select)
        .add_systems(OnExit(GameState::RaceSelect),  (player::preview::sys_destroy_preview, despawn_race_select))
        .add_systems(OnEnter(GameState::Loading),    spawn_loading_screen)
        // On ne despawne PAS l'écran de chargement sur OnExit(Loading) : il
        // reste visible pendant les premières frames d'InGame pour masquer
        // le freeze GPU (compilation des pipelines, build des shadow maps…).
        .add_systems(OnEnter(GameState::InGame),     (spawn_world, relock_cursor, start_ingame_warmup))
        .add_systems(OnEnter(GameState::Paused),     spawn_pause_menu)
        .add_systems(OnExit(GameState::Paused),      despawn_pause_menu)
        .add_systems(OnEnter(GameState::Options),    spawn_options_menu)
        .add_systems(OnExit(GameState::Options),     despawn_options_menu)
        .add_systems(OnEnter(GameState::Inventory),  spawn_inventory_ui)
        .add_systems(OnExit(GameState::Inventory),   (despawn_inventory_ui, player::preview::sys_destroy_preview, cleanup_inventory_tab, cleanup_spell_slot, cleanup_held_item))
        .add_systems(OnEnter(GameState::Dead),       spawn_death_screen)
        .add_systems(OnExit(GameState::Dead),        despawn_death_screen)
        .add_systems(Update, (
            handle_main_menu       .run_if(in_state(GameState::MainMenu)),
            handle_save_select     .run_if(in_state(GameState::SaveSelect)),
            handle_race_list_click .run_if(in_state(GameState::RaceSelect)),
            handle_confirm_race    .run_if(in_state(GameState::RaceSelect)),
            update_race_info_panel .run_if(in_state(GameState::RaceSelect)),
            player::preview::sys_update_preview .run_if(in_state(GameState::RaceSelect)),
            handle_loading         .run_if(in_state(GameState::Loading)),
            handle_pause_menu      .run_if(in_state(GameState::Paused)),
            handle_options_menu    .run_if(in_state(GameState::Options)),
            fade_loading_screen    .run_if(in_state(GameState::InGame)),
            apply_resize_kick      .run_if(resource_exists::<PendingResizeKick>),
            check_player_death     .run_if(in_state(GameState::InGame)),
            handle_death_screen    .run_if(in_state(GameState::Dead)),
            update_camera_fov,
            button_hover_system,
        ))
        .add_systems(Update, (
            handle_inventory_tabs,
            handle_craft_buttons,
            handle_inv_slot_click,
            update_inv_slot_visuals,
            update_held_cursor,
            handle_spell_loadout_click,
            handle_spell_assign_click,
        ).run_if(in_state(GameState::Inventory)))
        .run();
}

#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum GameState {
    #[default] MainMenu,
    SaveSelect,
    RaceSelect, Loading, InGame, Paused, Options, Inventory, Dead,
}

#[derive(Resource, Default)]
pub struct PlayerConfig { pub race: race::Race, pub name: String }

#[derive(Resource)]
pub struct RenderDistanceConfig { pub distance: i32 }
impl Default for RenderDistanceConfig { fn default() -> Self { Self { distance: 3 } } }

#[derive(Resource)]
pub struct FovConfig { pub fov: f32 }
impl Default for FovConfig { fn default() -> Self { Self { fov: 70.0 } } }

/// Mémorise d'où le menu Options a été ouvert, pour savoir où revenir en fermant.
#[derive(Resource, Default, Clone, PartialEq, Eq)]
enum OptionsFrom { #[default] MainMenu, Paused }

#[derive(Resource, Default)]
pub struct SelectedRacePreview { pub race: Option<race::Race> }

#[derive(Resource)]
pub struct PixelFont(pub Handle<Font>);

impl FromWorld for PixelFont {
    fn from_world(world: &mut World) -> Self {
        Self(world.resource::<AssetServer>().load("fonts/PressStart2P-Regular.ttf"))
    }
}

#[derive(Resource)]
struct BgHandle(Handle<Image>);

impl FromWorld for BgHandle {
    fn from_world(world: &mut World) -> Self {
        Self(world.resource::<AssetServer>().load("téléchargement.png"))
    }
}

fn setup(mut commands: Commands) {
    // Caméra minimaliste : on évite SSAO / Bloom / Prepass. Ces effets
    // sont des killers de FPS sur un voxel game et n'apportent rien
    // visuellement ici.
    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
    ));
    commands.insert_resource(PlayerConfig::default());
}

fn update_camera_fov(
    fov:      Res<FovConfig>,
    mut cams: Query<&mut Projection, (With<Camera3d>, Without<player::preview::PreviewCam>, Without<player::animation::ArmCam>)>,
) {
    if !fov.is_changed() { return; }
    for mut proj in &mut cams {
        if let Projection::Perspective(ref mut p) = *proj {
            p.fov = fov.fov.to_radians();
        }
    }
}

fn pf(font: &Handle<Font>, size: f32) -> TextFont {
    TextFont { font: font.clone(), font_size: size, ..default() }
}

#[derive(Component, Clone)]
pub struct ButtonColors { pub normal: Color, pub hovered: Color, pub pressed: Color }

impl ButtonColors {
    fn new(n: Color, h: Color, p: Color) -> Self { Self { normal: n, hovered: h, pressed: p } }

    /// Bouton blanc semi-transparent (menu principal sur image de fond).
    fn white() -> Self { Self::new(
        Color::srgba(1.00, 1.00, 1.00, 0.82),
        Color::srgba(1.00, 1.00, 1.00, 1.00),
        Color::srgba(0.80, 0.80, 0.80, 1.00),
    )}
    fn gray()  -> Self { Self::new(Color::srgb(0.333,0.333,0.333), Color::srgb(0.420,0.420,0.580), Color::srgb(0.200,0.200,0.200)) }
    fn green() -> Self { Self::new(Color::srgb(0.180,0.380,0.180), Color::srgb(0.250,0.520,0.250), Color::srgb(0.110,0.240,0.110)) }
    fn red()   -> Self { Self::new(Color::srgb(0.440,0.110,0.110), Color::srgb(0.580,0.150,0.150), Color::srgb(0.270,0.070,0.070)) }
}

fn button_hover_system(
    mut q: Query<(&Interaction, &ButtonColors, &mut BackgroundColor), Changed<Interaction>>,
) {
    for (int, c, mut bg) in &mut q {
        bg.0 = match int {
            Interaction::Hovered => c.hovered,
            Interaction::Pressed => c.pressed,
            Interaction::None    => c.normal,
        };
    }
}

const MC_YELLOW: Color = Color::srgb(1.000, 1.000, 0.333);
const MC_BG:     Color = Color::srgb(0.100, 0.100, 0.130);
const MC_GRAY:   Color = Color::srgb(0.490, 0.490, 0.490);
const MC_BORDER: Color = Color::srgb(0.600, 0.600, 0.600);
const MC_SHADOW: Color = Color::srgb(0.060, 0.060, 0.060);
const MC_SEP:    Color = Color::srgb(0.380, 0.380, 0.380);

fn mc_panel(w: f32, pad: f32) -> (Node, BackgroundColor, BorderColor) {
    (
        Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, padding: UiRect::all(Val::Px(pad)), border: UiRect::all(Val::Px(2.)), width: Val::Px(w), ..default() },
        BackgroundColor(MC_BG),
        BorderColor(MC_BORDER),
    )
}

fn mc_sep(p: &mut ChildBuilder) {
    p.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), margin: UiRect::vertical(Val::Px(12.)), ..default() }, BackgroundColor(MC_SEP)));
}

/// Bouton style Minecraft avec ombre 3D. `dark_text` passe le texte en
/// sombre quand le fond est clair (bouton blanc du menu principal).
fn mc_btn<A: Component>(
    parent:    &mut ChildBuilder,
    label:     &str,
    colors:    ButtonColors,
    action:    A,
    font:      &Handle<Font>,
    dark_text: bool,
) {
    let text_color = if dark_text { Color::srgb(0.08, 0.08, 0.08) } else { Color::WHITE };
    parent.spawn((
        Node { width: Val::Percent(100.), padding: UiRect { bottom: Val::Px(3.), right: Val::Px(3.), ..default() }, margin: UiRect::vertical(Val::Px(4.)), ..default() },
        BackgroundColor(MC_SHADOW),
    ))
    .with_children(|w| {
        w.spawn((
            Button,
            Node { width: Val::Percent(100.), padding: UiRect::axes(Val::Px(0.), Val::Px(11.)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
            BackgroundColor(colors.normal),
            colors,
            action,
        ))
        .with_child((Text::new(label), pf(font, 13.), TextColor(text_color)));
    });
}

#[derive(Component)] struct MainMenuUI;
#[derive(Component)] struct MenuButton(MenuAction);
#[derive(Clone, Copy)] enum MenuAction { NewGame, LoadGame, Options, Quit }

fn spawn_main_menu(mut commands: Commands, font: Res<PixelFont>, bg: Res<BgHandle>) {
    let f = &font.0;
    commands.spawn((
        Node { width: Val::Percent(100.), height: Val::Percent(100.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        ImageNode::new(bg.0.clone()),
        MainMenuUI,
    ))
    .with_children(|root| {
        // Overlay sombre pour simuler le flou et garder le texte lisible
        // par-dessus l'image de fond.
        root.spawn((
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.), height: Val::Percent(100.), ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.52)),
        ));
        // Second calque blanc très léger pour l'effet « frosted glass ».
        root.spawn((
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.), height: Val::Percent(100.), ..default() },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.04)),
        ));

        root.spawn((Text::new("My Magic Adventure"), pf(f, 26.), TextColor(Color::WHITE)));
        root.spawn((Node { height: Val::Px(6.), ..default() },));
        root.spawn((Text::new("Alpha v0.1"), pf(f, 8.), TextColor(Color::srgba(0.85,0.85,0.85,0.80))));

        root.spawn((Node { width: Val::Px(360.), height: Val::Px(1.), margin: UiRect::vertical(Val::Px(30.)), ..default() }, BackgroundColor(Color::srgba(1.,1.,1.,0.40))));

        root.spawn((
            Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, padding: UiRect::all(Val::Px(28.)), border: UiRect::all(Val::Px(1.)), width: Val::Px(360.), ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            BorderColor(Color::srgba(1., 1., 1., 0.20)),
        ))
        .with_children(|panel| {
            // S'il existe au moins une sauvegarde, on propose « Charger
            // Partie » qui ouvre le gestionnaire (où on peut aussi créer /
            // supprimer). Sinon accès direct à la création d'une partie.
            if systems::any_save_exists() {
                mc_btn(panel, "Charger Partie",  ButtonColors::white(), MenuButton(MenuAction::LoadGame), f, true);
            } else {
                mc_btn(panel, "Nouvelle Partie", ButtonColors::white(), MenuButton(MenuAction::NewGame),  f, true);
            }
            mc_btn(panel, "Options",         ButtonColors::white(), MenuButton(MenuAction::Options),  f, true);
            mc_btn(panel, "Quitter",         ButtonColors::white(), MenuButton(MenuAction::Quit),     f, true);
        });
    });
}

fn despawn_main_menu(mut commands: Commands, q: Query<Entity, With<MainMenuUI>>) {
    for e in &q { commands.entity(e).despawn_recursive(); }
}

fn handle_main_menu(
    mut commands: Commands,
    mut next:     ResMut<NextState<GameState>>,
    mut exit:     EventWriter<AppExit>,
    mut opt_from: ResMut<OptionsFrom>,
    mut slot:     ResMut<systems::CurrentSaveSlot>,
    q: Query<(&Interaction, &MenuButton), (Changed<Interaction>, With<Button>)>,
) {
    for (int, btn) in &q {
        if *int != Interaction::Pressed { continue; }
        match btn.0 {
            MenuAction::NewGame  => {
                slot.0 = None;
                commands.remove_resource::<systems::PendingLoad>();
                next.set(GameState::RaceSelect);
            }
            MenuAction::LoadGame => next.set(GameState::SaveSelect),
            MenuAction::Options  => { *opt_from = OptionsFrom::MainMenu; next.set(GameState::Options); }
            MenuAction::Quit     => { exit.send(AppExit::Success); }
        }
    }
}

#[derive(Component)] struct SaveSelectUI;
#[derive(Component)] struct SaveLoadButton(u32);
#[derive(Component)] struct SaveDeleteButton(u32);
#[derive(Component)] struct SaveNewButton;
#[derive(Component)] struct SaveBackButton;

fn spawn_save_select(mut commands: Commands, font: Res<PixelFont>, bg: Res<BgHandle>) {
    let f = &font.0;
    let saves = systems::list_saves();

    commands.spawn((
        Node { width: Val::Percent(100.), height: Val::Percent(100.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        ImageNode::new(bg.0.clone()),
        SaveSelectUI,
    ))
    .with_children(|root| {
        root.spawn((
            Node { position_type: PositionType::Absolute, width: Val::Percent(100.), height: Val::Percent(100.), ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
        ));

        root.spawn((Text::new("Selectionner une partie"), pf(f, 18.), TextColor(Color::WHITE)));
        root.spawn((Node { width: Val::Px(520.), height: Val::Px(1.), margin: UiRect::vertical(Val::Px(20.)), ..default() }, BackgroundColor(Color::srgba(1.,1.,1.,0.40))));

        root.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Stretch,
                padding: UiRect::all(Val::Px(20.)),
                border: UiRect::all(Val::Px(1.)),
                width: Val::Px(520.),
                row_gap: Val::Px(8.),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            BorderColor(Color::srgba(1., 1., 1., 0.20)),
        ))
        .with_children(|panel| {
            if saves.is_empty() {
                panel.spawn((
                    Node { width: Val::Percent(100.), justify_content: JustifyContent::Center, align_items: AlignItems::Center, padding: UiRect::all(Val::Px(20.)), ..default() },
                    BackgroundColor(Color::srgba(0., 0., 0., 0.30)),
                ))
                .with_child((Text::new("Aucune sauvegarde"), pf(f, 10.), TextColor(Color::srgba(0.75, 0.75, 0.75, 0.9))));
            } else {
                for save in &saves {
                    let race_name = race_info(&save.data.race).0;
                    let label = format!("Slot {}  |  {}  |  {}", save.id, save.data.name, race_name);

                    panel.spawn((
                        Node {
                            width: Val::Percent(100.),
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.),
                            ..default()
                        },
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Node { flex_grow: 1., padding: UiRect { bottom: Val::Px(3.), right: Val::Px(3.), ..default() }, ..default() },
                            BackgroundColor(MC_SHADOW),
                        ))
                        .with_children(|sw| {
                            sw.spawn((
                                Button,
                                Node { width: Val::Percent(100.), padding: UiRect::axes(Val::Px(10.), Val::Px(10.)), justify_content: JustifyContent::FlexStart, align_items: AlignItems::Center, ..default() },
                                BackgroundColor(ButtonColors::white().normal),
                                ButtonColors::white(),
                                SaveLoadButton(save.id),
                            ))
                            .with_child((Text::new(label), pf(f, 10.), TextColor(Color::srgb(0.08,0.08,0.08))));
                        });

                        row.spawn((
                            Node { padding: UiRect { bottom: Val::Px(3.), right: Val::Px(3.), ..default() }, ..default() },
                            BackgroundColor(MC_SHADOW),
                        ))
                        .with_children(|sw| {
                            sw.spawn((
                                Button,
                                Node { width: Val::Px(48.), padding: UiRect::axes(Val::Px(0.), Val::Px(10.)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                                BackgroundColor(ButtonColors::red().normal),
                                ButtonColors::red(),
                                SaveDeleteButton(save.id),
                            ))
                            .with_child((Text::new("X"), pf(f, 12.), TextColor(Color::WHITE)));
                        });
                    });
                }
            }

            panel.spawn((Node { width: Val::Percent(100.), height: Val::Px(1.), margin: UiRect::vertical(Val::Px(10.)), ..default() }, BackgroundColor(Color::srgba(1.,1.,1.,0.20))));

            mc_btn(panel, "Nouvelle Partie", ButtonColors::green(), SaveNewButton,  f, false);
            mc_btn(panel, "Retour",          ButtonColors::gray(),  SaveBackButton, f, false);
        });
    });
}

fn despawn_save_select(mut commands: Commands, q: Query<Entity, With<SaveSelectUI>>) {
    for e in &q { commands.entity(e).despawn_recursive(); }
}

fn handle_save_select(
    mut commands: Commands,
    mut next:     ResMut<NextState<GameState>>,
    mut cfg:      ResMut<PlayerConfig>,
    mut slot:     ResMut<systems::CurrentSaveSlot>,
    load_q:   Query<(&Interaction, &SaveLoadButton),   (Changed<Interaction>, With<Button>)>,
    del_q:    Query<(&Interaction, &SaveDeleteButton), (Changed<Interaction>, With<Button>)>,
    new_q:    Query<&Interaction, (Changed<Interaction>, With<SaveNewButton>)>,
    back_q:   Query<&Interaction, (Changed<Interaction>, With<SaveBackButton>)>,
    keys:     Res<ButtonInput<KeyCode>>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        next.set(GameState::MainMenu);
        return;
    }

    for (int, btn) in &load_q {
        if *int != Interaction::Pressed { continue; }
        match systems::read_save_slot(btn.0) {
            Some(data) => {
                cfg.race = data.race.clone();
                cfg.name = data.name.clone();
                slot.0   = Some(btn.0);
                commands.insert_resource(data);
                // Charge le sidecar .edits : les modifications de blocs
                // persistées pour cette sauvegarde.
                commands.insert_resource(systems::read_edits_file(btn.0));
                next.set(GameState::Loading);
                return;
            }
            None => warn!("Slot {} illisible", btn.0),
        }
    }

    for (int, btn) in &del_q {
        if *int != Interaction::Pressed { continue; }
        systems::delete_save_slot(btn.0);
        // On repasse par MainMenu pour reconstruire la liste proprement,
        // plutôt que de faire un re-spawn en place.
        next.set(GameState::MainMenu);
        return;
    }

    for int in &new_q {
        if *int != Interaction::Pressed { continue; }
        slot.0 = None;
        commands.remove_resource::<systems::PendingLoad>();
        next.set(GameState::RaceSelect);
        return;
    }

    for int in &back_q {
        if *int == Interaction::Pressed {
            next.set(GameState::MainMenu);
        }
    }
}

#[derive(Component)] struct RaceSelectUI;
#[derive(Component)] struct RaceListButton(race::Race);
#[derive(Component)] struct ConfirmRaceButton;
#[derive(Component)] struct RaceInfoName;
#[derive(Component)] struct RaceInfoDesc;
#[derive(Component)] struct RaceInfoAbility;
#[derive(Component)] struct RaceInfoStats;
#[derive(Component)] struct RaceColorBar;

/// Retourne (nom, description, capacité, couleur d'accent) pour une race.
fn race_info(r: &race::Race) -> (&'static str, &'static str, &'static str, Color) {
    match r {
        race::Race::Sylvaris  => ("Sylvaris",  "Gardiennes de la foret. Leur lien avec la nature leur confere une regeneration naturelle en milieu boise.", "Regeneration acceleree en foret", Color::srgb(0.15,0.45,0.15)),
        race::Race::Ignaar    => ("Ignaar",    "Forges dans le volcan, les Ignaar sont des guerriers implacables resistants au feu et a la chaleur extreme.", "Souffle enflamme",               Color::srgb(0.55,0.18,0.06)),
        race::Race::Aethyn    => ("Aethyn",    "Legers comme la brise. Leurs ailes vestigiales leur permettent de planer au-dessus des abimes et des gouffres.", "Vol plane prolonge",           Color::srgb(0.18,0.32,0.60)),
        race::Race::Vorkai    => ("Vorkai",    "Creatures du crepuscule. Ils voient dans l'obscurite totale et se fondent dans les ombres sans faire de bruit.", "Invisibilite momentanee",    Color::srgb(0.32,0.08,0.48)),
        race::Race::Crysthari => ("Crysthari", "Maitres des flux arcaniques. Leur reserve de mana depasse de loin celle de toutes les autres races connues.", "Projectile magique",             Color::srgb(0.45,0.45,0.70)),
    }
}

fn spawn_race_select(
    mut commands:  Commands,
    font:          Res<PixelFont>,
    mut images:    ResMut<Assets<Image>>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    use race::Race;
    let f = &font.0;
    let races = [Race::Sylvaris, Race::Ignaar, Race::Aethyn, Race::Vorkai, Race::Crysthari];

    // Scène RTT créée avec une race par défaut : `sys_update_preview`
    // la remplacera dès qu'une race est sélectionnée dans la liste.
    let preview_img = player::preview::create_preview_scene(
        &mut commands, &mut images, &mut meshes, &mut materials,
        &Race::Sylvaris, 130, 260,
    );

    commands.spawn((
        Node { width: Val::Percent(100.), height: Val::Percent(100.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgb(0.05,0.05,0.08)),
        RaceSelectUI,
    ))
    .with_children(|root| {
        root.spawn((Text::new("Choisis ton Destin"), pf(f, 17.), TextColor(MC_YELLOW)));
        root.spawn((Node { width: Val::Px(720.), height: Val::Px(2.), margin: UiRect::vertical(Val::Px(20.)), ..default() }, BackgroundColor(MC_SEP)));

        root.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(12.), ..default() })
        .with_children(|row| {
            // Panneau gauche : la liste des races cliquables.
            row.spawn((Node { flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(10.)), border: UiRect::all(Val::Px(2.)), width: Val::Px(260.), row_gap: Val::Px(4.), ..default() }, BackgroundColor(MC_BG), BorderColor(MC_BORDER)))
            .with_children(|left| {
                for race in &races {
                    let (name, _, _, color) = race_info(race);
                    left.spawn((
                        Button,
                        Node { width: Val::Percent(100.), flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: Val::Px(10.), padding: UiRect::axes(Val::Px(8.), Val::Px(8.)), border: UiRect::all(Val::Px(2.)), ..default() },
                        BackgroundColor(Color::srgb(0.18,0.18,0.23)),
                        BorderColor(MC_BORDER),
                        RaceListButton(race.clone()),
                        ButtonColors::new(Color::srgb(0.18,0.18,0.23), Color::srgb(0.26,0.26,0.34), Color::srgb(0.11,0.11,0.15)),
                    ))
                    .with_children(|btn| {
                        btn.spawn((Node { width: Val::Px(26.), height: Val::Px(26.), border: UiRect::all(Val::Px(2.)), flex_shrink: 0., ..default() }, BackgroundColor(color), BorderColor(Color::srgba(1.,1.,1.,0.25))));
                        btn.spawn((Text::new(name), pf(f, 11.), TextColor(Color::WHITE)));
                    });
                }
                left.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), margin: UiRect::vertical(Val::Px(2.)), ..default() }, BackgroundColor(MC_SEP)));
                left.spawn((
                    Button,
                    Node { width: Val::Percent(100.), justify_content: JustifyContent::Center, align_items: AlignItems::Center, padding: UiRect::axes(Val::Px(8.), Val::Px(8.)), border: UiRect::all(Val::Px(2.)), ..default() },
                    BackgroundColor(Color::srgb(0.22,0.22,0.30)),
                    BorderColor(MC_BORDER),
                    RaceListButton(Race::random()),
                    ButtonColors::new(Color::srgb(0.22,0.22,0.30), Color::srgb(0.32,0.32,0.42), Color::srgb(0.14,0.14,0.19)),
                ))
                .with_child((Text::new("Aleatoire"), pf(f, 11.), TextColor(Color::WHITE)));
            });

            // Panneau droit : nom, preview 3D, description, capacité, stats.
            row.spawn((Node { flex_direction: FlexDirection::Column, border: UiRect::all(Val::Px(2.)), width: Val::Px(420.), overflow: Overflow::clip(), ..default() }, BackgroundColor(MC_BG), BorderColor(MC_BORDER)))
            .with_children(|right| {
                // Barre d'en-tête avec le nom (et une teinte race en fond).
                right.spawn((
                    Node { width: Val::Percent(100.), padding: UiRect::axes(Val::Px(16.), Val::Px(14.)), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
                    BackgroundColor(Color::srgb(0.18,0.18,0.23)),
                    RaceColorBar,
                ))
                .with_child((Text::new("< Selectionne une race"), pf(f, 9.), TextColor(Color::srgba(0.65,0.65,0.65,0.9)), RaceInfoName));

                // Zone centrale : preview 3D à gauche, texte à droite.
                right.spawn(Node { flex_direction: FlexDirection::Row, flex_grow: 1., ..default() })
                .with_children(|mid| {
                    mid.spawn((
                        Node { width: Val::Px(130.), flex_shrink: 0., ..default() },
                        ImageNode::new(preview_img),
                    ));

                    mid.spawn(Node { flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(14.)), row_gap: Val::Px(10.), flex_grow: 1., ..default() })
                    .with_children(|body| {
                        body.spawn((Text::new(""), pf(f, 9.), TextColor(Color::srgba(0.85,0.85,0.85,1.)), RaceInfoDesc));
                        body.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), ..default() }, BackgroundColor(MC_SEP)));
                        body.spawn((Text::new(""), pf(f, 9.), TextColor(MC_YELLOW), RaceInfoAbility));
                        body.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), ..default() }, BackgroundColor(MC_SEP)));
                        body.spawn((Text::new(""), pf(f, 8.), TextColor(Color::srgba(0.70,0.85,0.70,1.)), RaceInfoStats));
                    });
                });

                right.spawn((Node { width: Val::Percent(100.), padding: UiRect::all(Val::Px(12.)), border: UiRect { top: Val::Px(2.), ..default() }, ..default() }, BorderColor(MC_SEP)))
                .with_children(|footer| {
                    footer.spawn((Node { width: Val::Percent(100.), padding: UiRect { bottom: Val::Px(3.), right: Val::Px(3.), ..default() }, ..default() }, BackgroundColor(MC_SHADOW)))
                    .with_children(|sw| {
                        sw.spawn((
                            Button,
                            Node { width: Val::Percent(100.), padding: UiRect::axes(Val::Px(0.), Val::Px(11.)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                            BackgroundColor(ButtonColors::green().normal),
                            ButtonColors::green(),
                            ConfirmRaceButton,
                        ))
                        .with_child((Text::new("Choisir"), pf(f, 13.), TextColor(Color::WHITE)));
                    });
                });
            });
        });
    });
}

fn despawn_race_select(mut commands: Commands, q: Query<Entity, With<RaceSelectUI>>) {
    for e in &q { commands.entity(e).despawn_recursive(); }
}

fn handle_race_list_click(q: Query<(&Interaction, &RaceListButton), (Changed<Interaction>, With<Button>)>, mut preview: ResMut<SelectedRacePreview>) {
    for (int, btn) in &q { if *int != Interaction::Pressed { continue; } preview.race = Some(btn.0.clone()); }
}

fn handle_confirm_race(q: Query<&Interaction, (Changed<Interaction>, With<ConfirmRaceButton>)>, preview: Res<SelectedRacePreview>, mut config: ResMut<PlayerConfig>, mut next: ResMut<NextState<GameState>>) {
    for int in &q {
        if *int != Interaction::Pressed { continue; }
        if let Some(race) = &preview.race {
            config.race = race.clone(); config.name = "Aventurier".to_string();
            next.set(GameState::Loading);
        }
    }
}

fn update_race_info_panel(
    preview:    Res<SelectedRacePreview>,
    mut name_q: Query<&mut Text, (With<RaceInfoName>,  Without<RaceInfoDesc>, Without<RaceInfoAbility>, Without<RaceInfoStats>)>,
    mut desc_q: Query<&mut Text, (With<RaceInfoDesc>,  Without<RaceInfoName>, Without<RaceInfoAbility>, Without<RaceInfoStats>)>,
    mut abil_q: Query<&mut Text, (With<RaceInfoAbility>, Without<RaceInfoName>, Without<RaceInfoDesc>, Without<RaceInfoStats>)>,
    mut stat_q: Query<&mut Text, (With<RaceInfoStats>, Without<RaceInfoName>, Without<RaceInfoDesc>, Without<RaceInfoAbility>)>,
    mut bar_q:  Query<&mut BackgroundColor, With<RaceColorBar>>,
) {
    if !preview.is_changed() { return; }
    let Some(race) = &preview.race else { return };
    let (name, desc, abil, color) = race_info(race);
    let s = race.base_stats();
    if let Ok(mut t) = name_q.get_single_mut() { t.0 = name.into(); }
    if let Ok(mut t) = desc_q.get_single_mut() { t.0 = desc.into(); }
    if let Ok(mut t) = abil_q.get_single_mut() { t.0 = format!(">> {}", abil); }
    if let Ok(mut t) = stat_q.get_single_mut() { t.0 = format!("PV {}  Mana {}  Vit {:.1}", s.max_hp, s.max_mana, s.speed); }
    if let Ok(mut bg) = bar_q.get_single_mut() { bg.0 = color; }
}

#[derive(Component)] struct LoadingScreenUI;
#[derive(Component)] struct LoadingStatusText;

/// Compte le nombre de frames restantes avant de retirer l'écran de
/// chargement. L'écran persiste PENDANT les premières frames d'InGame
/// pour masquer le freeze causé par la compilation des pipelines GPU,
/// la construction des shadow maps, le spawn du HUD, etc.
#[derive(Resource)]
struct InGameWarmup { frames_left: u32 }

fn spawn_loading_screen(mut commands: Commands, cfg: Res<PlayerConfig>, font: Res<PixelFont>) {
    let f = &font.0;
    commands.spawn((
        Node { width: Val::Percent(100.), height: Val::Percent(100.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, row_gap: Val::Px(20.), ..default() },
        BackgroundColor(Color::srgb(0.05,0.05,0.08)),
        // Z-index élevé : l'écran de chargement reste au-dessus de tout
        // (HUD, monde 3D) pendant le warmup InGame.
        GlobalZIndex(100),
        LoadingScreenUI,
    ))
    .with_children(|p| {
        p.spawn((Text::new("My Magic Adventure"), pf(f, 20.), TextColor(MC_YELLOW)));
        p.spawn((Node { width: Val::Px(460.), height: Val::Px(2.), ..default() }, BackgroundColor(MC_SEP)));
        p.spawn((Text::new(format!("Race : {:?}", cfg.race)), pf(f, 12.), TextColor(Color::WHITE)));
        p.spawn((Text::new("Generation du monde..."), pf(f, 9.), TextColor(Color::srgba(0.55,0.55,0.55,1.)), LoadingStatusText));
    });
}

fn handle_loading(
    mgr:      Res<world::chunk::ChunkManager>,
    rd:       Res<RenderDistanceConfig>,
    player_q: Query<&Transform, With<player::Player>>,
    mut next: ResMut<NextState<GameState>>,
    mut status_q: Query<&mut Text, With<LoadingStatusText>>,
) {
    let Ok(tf) = player_q.get_single() else { return };
    let cs = world::chunk::CHUNK_SIZE as f32;
    let px = (tf.translation.x / cs).floor() as i32;
    let pz = (tf.translation.z / cs).floor() as i32;
    let r = rd.distance;

    let total = ((2 * r + 1) * (2 * r + 1)) as usize;
    let loaded = (-r..=r)
        .flat_map(|dx| (-r..=r).map(move |dz| (dx, dz)))
        .filter(|&(dx, dz)| mgr.loaded.contains_key(&(px + dx, pz + dz)))
        .count();
    let all_ready = loaded == total && mgr.pending.is_empty();

    info!(
        "handle_loading: player chunk=({px},{pz}) loaded={loaded}/{total} pending={} all_ready={all_ready}",
        mgr.pending.len()
    );

    if let Ok(mut text) = status_q.get_single_mut() {
        if !all_ready {
            text.0 = format!("Generation du monde... {}/{}", loaded, total);
        } else {
            text.0 = "Preparation du rendu...".into();
        }
    }

    if !all_ready {
        return;
    }

    next.set(GameState::InGame);
}

/// Démarre le compteur de warmup à l'entrée d'InGame.
fn start_ingame_warmup(mut commands: Commands) {
    // 30 frames ≈ 0.5 s à 60 fps. C'est le temps qu'il faut au GPU pour
    // compiler les pipelines de rendu (StandardMaterial, shadow maps,
    // alpha blend) et traiter les premiers draw calls AVANT qu'on retire
    // l'écran de chargement.
    commands.insert_resource(InGameWarmup { frames_left: 30 });
}

/// Tourne à chaque frame d'InGame : décompte le warmup puis retire
/// l'écran de chargement quand le GPU est prêt.
fn fade_loading_screen(
    mut commands: Commands,
    q: Query<Entity, With<LoadingScreenUI>>,
    warmup: Option<ResMut<InGameWarmup>>,
) {
    let Some(mut warmup) = warmup else { return };
    if warmup.frames_left > 0 {
        warmup.frames_left -= 1;
        return;
    }
    for e in &q { commands.entity(e).despawn_recursive(); }
    commands.remove_resource::<InGameWarmup>();
}

#[derive(Component)] struct PauseMenuUI;
#[derive(Component)] struct PauseButton(PauseAction);
#[derive(Clone, Copy)] enum PauseAction { Resume, Options, MainMenu, Quit }

fn spawn_pause_menu(mut commands: Commands, font: Res<PixelFont>) {
    let f = &font.0;
    commands.spawn((
        Node { width: Val::Percent(100.), height: Val::Percent(100.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgba(0.,0.,0.,0.70)),
        PauseMenuUI,
    ))
    .with_children(|root| {
        let (node, bg, border) = mc_panel(320., 32.);
        root.spawn((node, bg, border)).with_children(|p| {
            p.spawn((Text::new("PAUSE"), pf(f, 22.), TextColor(MC_YELLOW)));
            mc_sep(p);
            mc_btn(p, "Reprendre",      ButtonColors::green(), PauseButton(PauseAction::Resume),   f, false);
            mc_btn(p, "Options",        ButtonColors::gray(),  PauseButton(PauseAction::Options),  f, false);
            mc_btn(p, "Retour au menu", ButtonColors::gray(),  PauseButton(PauseAction::MainMenu), f, false);
            mc_btn(p, "Quitter le jeu", ButtonColors::red(),   PauseButton(PauseAction::Quit),     f, false);
            p.spawn(Node { margin: UiRect { top: Val::Px(8.), ..default() }, ..default() })
             .with_child((Text::new("[Echap] Reprendre"), pf(f, 7.), TextColor(Color::srgba(0.5,0.5,0.5,0.8))));
        });
    });
}

fn despawn_pause_menu(mut commands: Commands, q: Query<Entity, With<PauseMenuUI>>) {
    for e in &q { commands.entity(e).despawn_recursive(); }
}

fn handle_pause_menu(
    mut next:     ResMut<NextState<GameState>>,
    mut exit:     EventWriter<AppExit>,
    mut opt_from: ResMut<OptionsFrom>,
    keys:         Res<ButtonInput<KeyCode>>,
    q:            Query<(&Interaction, &PauseButton), (Changed<Interaction>, With<Button>)>,
) {
    if keys.just_pressed(KeyCode::Escape) { next.set(GameState::InGame); return; }
    for (int, btn) in &q {
        if *int != Interaction::Pressed { continue; }
        match btn.0 {
            PauseAction::Resume   => { next.set(GameState::InGame); }
            PauseAction::Options  => { *opt_from = OptionsFrom::Paused; next.set(GameState::Options); }
            PauseAction::MainMenu => next.set(GameState::MainMenu),
            PauseAction::Quit     => { exit.send(AppExit::Success); }
        }
    }
}

#[derive(Component)] struct OptionsMenuUI;
#[derive(Component)] struct RdButton(i32);
#[derive(Component)] struct RdValueText;
#[derive(Component)] struct FovButton(i32);
#[derive(Component)] struct FovValueText;
#[derive(Component)] struct HandSideButton;
#[derive(Component)] struct HandSideValueText;
#[derive(Component)] struct OptionsBackButton;

fn spawn_options_menu(
    mut commands: Commands,
    rd:   Res<RenderDistanceConfig>,
    fov:  Res<FovConfig>,
    hand: Res<player::animation::HandSide>,
    font: Res<PixelFont>,
) {
    let f = &font.0;
    let hand_label = match *hand {
        player::animation::HandSide::Right => "Droite",
        player::animation::HandSide::Left  => "Gauche",
    };
    commands.spawn((
        Node { width: Val::Percent(100.), height: Val::Percent(100.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgba(0.,0.,0.,0.70)),
        OptionsMenuUI,
    ))
    .with_children(|root| {
        let (node, bg, border) = mc_panel(380., 32.);
        root.spawn((node, bg, border)).with_children(|p| {
            p.spawn((Text::new("OPTIONS"), pf(f, 20.), TextColor(MC_YELLOW)));
            mc_sep(p);

            p.spawn((Text::new("Distance de rendu"), pf(f, 9.), TextColor(Color::WHITE)));
            spawn_stepper(p, f, format!("{}", rd.distance), RdButton(-1), RdButton(1), RdValueText);

            mc_sep(p);

            p.spawn((Text::new("Champ de vision (FOV)"), pf(f, 9.), TextColor(Color::WHITE)));
            spawn_stepper(p, f, format!("{:.0}", fov.fov), FovButton(-5), FovButton(5), FovValueText);

            mc_sep(p);

            p.spawn((Text::new("Main dominante"), pf(f, 9.), TextColor(Color::WHITE)));
            p.spawn((
                Node { padding: UiRect { bottom: Val::Px(3.), right: Val::Px(3.), ..default() }, margin: UiRect::vertical(Val::Px(6.)), width: Val::Percent(100.), ..default() },
                BackgroundColor(MC_SHADOW),
            ))
            .with_children(|w| {
                w.spawn((
                    Button,
                    Node { width: Val::Percent(100.), padding: UiRect::axes(Val::Px(0.), Val::Px(11.)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
                    BackgroundColor(ButtonColors::gray().normal),
                    ButtonColors::gray(),
                    HandSideButton,
                ))
                .with_child((Text::new(hand_label), pf(f, 13.), TextColor(MC_YELLOW), HandSideValueText));
            });

            mc_sep(p);
            mc_btn(p, "Retour", ButtonColors::gray(), OptionsBackButton, f, false);
        });
    });
}

/// Ligne `-` / valeur / `+` réutilisable pour les options numériques.
fn spawn_stepper<A: Component, B: Component, C: Component>(
    parent: &mut ChildBuilder,
    font:   &Handle<Font>,
    value:  String,
    dec:    A,
    inc:    B,
    text_marker: C,
) {
    parent.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, justify_content: JustifyContent::Center, column_gap: Val::Px(10.), margin: UiRect::vertical(Val::Px(6.)), width: Val::Percent(100.), ..default() })
    .with_children(|row| {
        row.spawn((Node { padding: UiRect { bottom: Val::Px(3.), right: Val::Px(3.), ..default() }, ..default() }, BackgroundColor(MC_SHADOW)))
        .with_children(|sw| {
            sw.spawn((Button, Node { width: Val::Px(44.), height: Val::Px(44.), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, BackgroundColor(ButtonColors::gray().normal), ButtonColors::gray(), dec))
            .with_child((Text::new("-"), pf(font, 18.), TextColor(Color::WHITE)));
        });
        row.spawn((Node { width: Val::Px(90.), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border: UiRect::all(Val::Px(2.)), ..default() }, BackgroundColor(Color::srgb(0.08,0.08,0.12)), BorderColor(MC_SEP)))
        .with_child((Text::new(value), pf(font, 14.), TextColor(MC_YELLOW), text_marker));
        row.spawn((Node { padding: UiRect { bottom: Val::Px(3.), right: Val::Px(3.), ..default() }, ..default() }, BackgroundColor(MC_SHADOW)))
        .with_children(|sw| {
            sw.spawn((Button, Node { width: Val::Px(44.), height: Val::Px(44.), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, BackgroundColor(ButtonColors::gray().normal), ButtonColors::gray(), inc))
            .with_child((Text::new("+"), pf(font, 18.), TextColor(Color::WHITE)));
        });
    });
}

fn despawn_options_menu(mut commands: Commands, q: Query<Entity, With<OptionsMenuUI>>) {
    for e in &q { commands.entity(e).despawn_recursive(); }
}

fn handle_options_menu(
    mut next:      ResMut<NextState<GameState>>,
    mut rd:        ResMut<RenderDistanceConfig>,
    mut fov:       ResMut<FovConfig>,
    mut hand:      ResMut<player::animation::HandSide>,
    opt_from:      Res<OptionsFrom>,
    keys:          Res<ButtonInput<KeyCode>>,
    rd_q:          Query<(&Interaction, &RdButton),  (Changed<Interaction>, With<Button>)>,
    fov_q:         Query<(&Interaction, &FovButton), (Changed<Interaction>, With<Button>)>,
    hand_q:        Query<&Interaction, (Changed<Interaction>, With<HandSideButton>)>,
    back_q:        Query<&Interaction, (Changed<Interaction>, With<OptionsBackButton>)>,
    mut rd_text:   Query<&mut Text, (With<RdValueText>, Without<FovValueText>, Without<HandSideValueText>)>,
    mut fov_text:  Query<&mut Text, (With<FovValueText>, Without<RdValueText>, Without<HandSideValueText>)>,
    mut hand_text: Query<&mut Text, (With<HandSideValueText>, Without<RdValueText>, Without<FovValueText>)>,
) {
    let go_back = |next: &mut ResMut<NextState<GameState>>, from: &OptionsFrom| {
        match from { OptionsFrom::MainMenu => next.set(GameState::MainMenu), OptionsFrom::Paused => next.set(GameState::Paused) }
    };

    if keys.just_pressed(KeyCode::Escape) { go_back(&mut next, &opt_from); return; }

    for (int, btn) in &rd_q {
        if *int != Interaction::Pressed { continue; }
        rd.distance = (rd.distance + btn.0).clamp(2, 8);
        if let Ok(mut t) = rd_text.get_single_mut() { t.0 = format!("{}", rd.distance); }
    }
    for (int, btn) in &fov_q {
        if *int != Interaction::Pressed { continue; }
        fov.fov = (fov.fov + btn.0 as f32).clamp(50., 120.);
        if let Ok(mut t) = fov_text.get_single_mut() { t.0 = format!("{:.0}", fov.fov); }
    }
    for int in &hand_q {
        if *int != Interaction::Pressed { continue; }
        *hand = match *hand {
            player::animation::HandSide::Right => player::animation::HandSide::Left,
            player::animation::HandSide::Left  => player::animation::HandSide::Right,
        };
        if let Ok(mut t) = hand_text.get_single_mut() {
            t.0 = match *hand {
                player::animation::HandSide::Right => "Droite".into(),
                player::animation::HandSide::Left  => "Gauche".into(),
            };
        }
    }
    for int in &back_q {
        if *int == Interaction::Pressed { go_back(&mut next, &opt_from); }
    }
}

#[derive(Component)] struct InventoryUI;

/// Onglet actuellement actif dans l'inventaire.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
enum InventoryTab { #[default] Items, Spells, Crafting, Stats }

#[derive(Component, Clone, Copy)]
struct CraftButton(usize);

/// Type de slot dans l'inventaire (backpack ou hotbar).
#[derive(Clone, Copy, PartialEq, Eq)]
enum InvSlotKind { Backpack, Hotbar }

/// Bouton de slot dans l'écran d'inventaire.
#[derive(Component, Clone, Copy)]
struct InvSlot(InvSlotKind, usize);

/// Icône dans un slot d'inventaire.
#[derive(Component, Clone, Copy)]
struct InvSlotIcon(InvSlotKind, usize);

/// Compteur dans un slot d'inventaire.
#[derive(Component, Clone, Copy)]
struct InvSlotCount(InvSlotKind, usize);

/// Texte affichant l'item tenu en main.
#[derive(Component)]
struct HeldItemDisplay;

/// Icône flottante qui suit le curseur quand on tient un item.
#[derive(Component)]
struct HeldItemCursor;

/// Item actuellement tenu par le joueur dans l'inventaire.
#[derive(Resource, Default)]
struct HeldItem(Option<(world::chunk::Block, u32)>);

/// Bouton d'onglet, stocke l'onglet qu'il représente.
#[derive(Component, Clone, Copy)] struct InvTabButton(InventoryTab);

/// Zone de contenu de chaque onglet.
#[derive(Component, Clone, Copy)] struct InvTabContent(InventoryTab);

/// Slot du loadout de sorts (clic pour sélectionner le slot cible).
#[derive(Component, Clone, Copy)] struct SpellLoadoutSlot(usize);

/// Bouton dans la liste « Tous les sorts » — clic pour assigner au slot sélectionné.
#[derive(Component, Clone, Copy)] struct SpellAssignButton(magic::spells::SpellId);

/// Indique quel slot du loadout est sélectionné pour l'assignation.
#[derive(Resource, Default)]
struct SelectedSpellSlot(usize);

fn cleanup_inventory_tab(mut commands: Commands) {
    commands.remove_resource::<InventoryTab>();
}

fn spawn_inventory_ui(
    mut commands:  Commands,
    cfg:           Res<PlayerConfig>,
    font:          Res<PixelFont>,
    mut images:    ResMut<Assets<Image>>,
    mut meshes:    ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    stats_q:       Query<&player::PlayerStats>,
    spell_bar:     Res<magic::SpellBar>,
    inventory:     Res<crafting::BlockInventory>,
    hotbar_res:    Res<world::interaction::Hotbar>,
    atlas:         Res<world::BlockAtlas>,
    icon_atlas:    Res<world::BlockIconAtlas>,
) {
    commands.insert_resource(InventoryTab::Items);
    commands.insert_resource(SelectedSpellSlot(0));
    commands.insert_resource(HeldItem::default());
    let f = &font.0;
    let slot_bd = Color::srgb(0.48,0.48,0.48);
    let active_tab_bg  = Color::srgb(0.30, 0.30, 0.45);
    let inactive_tab_bg = Color::srgb(0.15, 0.15, 0.20);
    let atlas_img    = atlas.handle.clone();
    let atlas_layout = icon_atlas.layout.clone();

    // Scène RTT : aperçu 3D du personnage dans la race choisie.
    let preview_img = player::preview::create_preview_scene(
        &mut commands, &mut images, &mut meshes, &mut materials,
        &cfg.race, 116, 230,
    );

    commands.spawn((
        Node { width: Val::Percent(100.), height: Val::Percent(100.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgba(0.,0.,0.,0.55)),
        InventoryUI,
    ))
    .with_children(|root| {
        // Barre d'onglets
        root.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(4.), margin: UiRect::bottom(Val::Px(8.)), ..default() })
        .with_children(|tabs| {
            for (tab, label) in [(InventoryTab::Items, "Inventaire"), (InventoryTab::Spells, "Sorts"), (InventoryTab::Crafting, "Crafting"), (InventoryTab::Stats, "Stats")] {
                let bg = if tab == InventoryTab::Items { active_tab_bg } else { inactive_tab_bg };
                tabs.spawn((
                    Button,
                    Node { padding: UiRect::axes(Val::Px(18.), Val::Px(8.)), ..default() },
                    BackgroundColor(bg),
                    InvTabButton(tab),
                ))
                .with_child((Text::new(label), pf(f, 11.), TextColor(MC_YELLOW)));
            }
        });

        // Onglet Inventaire : preview 3D + sac à dos (2 rangées) + hotbar.
        let hotbar = &hotbar_res;
        root.spawn((
            Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(10.), align_items: AlignItems::Stretch, display: Display::Flex, ..default() },
            InvTabContent(InventoryTab::Items),
        ))
        .with_children(|row| {
            row.spawn((
                Node { width: Val::Px(116.), border: UiRect::all(Val::Px(2.)), ..default() },
                BackgroundColor(Color::srgb(0.10, 0.10, 0.16)),
                BorderColor(MC_BORDER),
                ImageNode::new(preview_img),
            ));

            row.spawn((Node { flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(12.)), border: UiRect::all(Val::Px(2.)), row_gap: Val::Px(8.), ..default() }, BackgroundColor(MC_GRAY), BorderColor(MC_BORDER)))
            .with_children(|grid| {
                grid.spawn((Text::new(""), pf(f, 9.), TextColor(Color::srgba(1., 0.9, 0.5, 0.9)), HeldItemDisplay));

                grid.spawn((Text::new("Sac a dos — clic pour deplacer"), pf(f, 9.), TextColor(MC_YELLOW)));
                for row_start in [0usize, 9] {
                    grid.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(4.), ..default() })
                    .with_children(|r| {
                        for i in row_start..row_start + 9 {
                            let item = inventory.backpack[i];
                            spawn_inv_slot(r, InvSlotKind::Backpack, i, item, slot_bd, &atlas_img, &atlas_layout, f);
                        }
                    });
                }

                grid.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), ..default() }, BackgroundColor(MC_SEP)));

                grid.spawn((Text::new("Hotbar — clic pour deplacer"), pf(f, 9.), TextColor(MC_YELLOW)));
                grid.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(4.), ..default() })
                .with_children(|r| {
                    for i in 0..9usize {
                        let item = hotbar.slots[i];
                        spawn_inv_slot(r, InvSlotKind::Hotbar, i, item, slot_bd, &atlas_img, &atlas_layout, f);
                    }
                });
            });
        });

        // Onglet Sorts : loadout (5 slots) + liste cliquable pour assigner.
        root.spawn((
            Node { flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(14.)), border: UiRect::all(Val::Px(2.)), row_gap: Val::Px(6.), width: Val::Px(620.), display: Display::None, ..default() },
            BackgroundColor(Color::srgb(0.10, 0.10, 0.16)),
            BorderColor(MC_BORDER),
            InvTabContent(InventoryTab::Spells),
        ))
        .with_children(|spells_panel| {
            use magic::spells::SpellId;
            let all_spells = [
                SpellId::Fireball, SpellId::IceShard, SpellId::EarthWall,
                SpellId::WindDash, SpellId::LightHeal, SpellId::LightBlind,
                SpellId::ShadowCloak, SpellId::ShadowDrain, SpellId::WaterShield,
                SpellId::FireNova, SpellId::WindBlade, SpellId::EarthSpike,
            ];

            spells_panel.spawn((Text::new("Loadout — clic pour selectionner un slot"), pf(f, 11.), TextColor(MC_YELLOW)));
            spells_panel.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(6.), margin: UiRect::bottom(Val::Px(8.)), ..default() })
            .with_children(|row| {
                for i in 0..5usize {
                    let spell = spell_bar.slots[i];
                    let bg = spell.map(|s| s.slot_color(false)).unwrap_or(Color::srgba(0.12, 0.12, 0.12, 0.70));
                    let bd = if i == 0 { Color::WHITE } else { Color::srgba(0.5,0.5,0.5,0.7) };
                    row.spawn((
                        Button,
                        Node { width: Val::Px(52.), height: Val::Px(52.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, border: UiRect::all(Val::Px(2.)), row_gap: Val::Px(2.), ..default() },
                        BackgroundColor(bg),
                        BorderColor(bd),
                        SpellLoadoutSlot(i),
                    ))
                    .with_children(|slot| {
                        slot.spawn((Text::new(spell.map(|s| s.name()).unwrap_or("-")), pf(f, 7.), TextColor(Color::WHITE)));
                        slot.spawn((Text::new(format!("F{}", i + 1)), pf(f, 7.), TextColor(Color::srgba(1., 1., 1., 0.5))));
                    });
                }
            });

            spells_panel.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), ..default() }, BackgroundColor(MC_SEP)));
            spells_panel.spawn((Text::new("Tous les sorts — clic pour assigner au slot"), pf(f, 11.), TextColor(MC_YELLOW)));

            for spell in all_spells {
                let equipped = spell_bar.slots.contains(&Some(spell));
                spells_panel.spawn((
                    Button,
                    Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(10.), align_items: AlignItems::Center, padding: UiRect::all(Val::Px(4.)), ..default() },
                    BackgroundColor(Color::srgba(0.15, 0.15, 0.20, 0.0)),
                    SpellAssignButton(spell),
                ))
                .with_children(|row| {
                    row.spawn((
                        Node { width: Val::Px(28.), height: Val::Px(28.), border: UiRect::all(Val::Px(1.)), align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
                        BackgroundColor(spell.slot_color(false)),
                        BorderColor(Color::srgba(0.5,0.5,0.5,0.6)),
                    ));
                    row.spawn(Node { flex_direction: FlexDirection::Column, ..default() })
                    .with_children(|col| {
                        let name_col = if equipped { Color::srgba(0.6, 1.0, 0.6, 1.0) } else { Color::WHITE };
                        col.spawn((Text::new(spell.name()), pf(f, 10.), TextColor(name_col)));
                        let info = if equipped {
                            format!("Mana: {:.0}   CD: {:.1}s   [equipe]", spell.mana_cost(), spell.cooldown_secs())
                        } else {
                            format!("Mana: {:.0}   CD: {:.1}s", spell.mana_cost(), spell.cooldown_secs())
                        };
                        col.spawn((Text::new(info), pf(f, 7.), TextColor(Color::srgba(0.7, 0.7, 0.7, 0.9))));
                    });
                });
            }
        });

        // Onglet Crafting : liste des recettes avec bouton de craft.
        root.spawn((
            Node { flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(14.)), border: UiRect::all(Val::Px(2.)), row_gap: Val::Px(8.), width: Val::Px(620.), display: Display::None, ..default() },
            BackgroundColor(Color::srgb(0.10, 0.10, 0.16)),
            BorderColor(MC_BORDER),
            InvTabContent(InventoryTab::Crafting),
        ))
        .with_children(|panel| {
            panel.spawn((Text::new("Recettes"), pf(f, 12.), TextColor(MC_YELLOW)));
            panel.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), ..default() }, BackgroundColor(MC_SEP)));
            for (idx, recipe) in crafting::RECIPES.iter().enumerate() {
                let craftable = crafting::can_craft(&inventory, &hotbar_res, recipe);
                let (out_block, out_n) = recipe.output;
                let inputs_text: Vec<String> = recipe.inputs.iter()
                    .map(|(b, n)| format!("{}x{}", n, block_short_name(*b)))
                    .collect();
                let line = format!("{}: {} -> {}x{}", recipe.name, inputs_text.join(" + "), out_n, block_short_name(out_block));
                panel.spawn(Node { flex_direction: FlexDirection::Row, column_gap: Val::Px(10.), align_items: AlignItems::Center, ..default() })
                .with_children(|row| {
                    row.spawn((
                        Node { width: Val::Px(28.), height: Val::Px(28.), border: UiRect::all(Val::Px(1.)), overflow: Overflow::clip(), ..default() },
                        BorderColor(Color::srgba(0.5,0.5,0.5,0.6)),
                    ))
                    .with_child((
                        ImageNode {
                            image: atlas_img.clone(),
                            texture_atlas: Some(TextureAtlas {
                                layout: atlas_layout.clone(),
                                index:  out_block.tile_side(),
                            }),
                            ..default()
                        },
                        Node { width: Val::Percent(100.), height: Val::Percent(100.), ..default() },
                    ));
                    row.spawn((Text::new(line), pf(f, 9.), TextColor(if craftable { Color::WHITE } else { Color::srgba(0.55,0.55,0.55,1.0) })));
                    let btn_bg = if craftable { Color::srgb(0.18, 0.50, 0.20) } else { Color::srgb(0.30, 0.30, 0.30) };
                    row.spawn((
                        Button,
                        Node { padding: UiRect::axes(Val::Px(10.), Val::Px(4.)), border: UiRect::all(Val::Px(1.)), ..default() },
                        BackgroundColor(btn_bg),
                        BorderColor(Color::srgba(0.6,0.6,0.6,0.7)),
                        CraftButton(idx),
                    ))
                    .with_child((Text::new("Crafter"), pf(f, 8.), TextColor(Color::WHITE)));
                });
            }
        });

        // Onglet Stats : PV/mana courants + stats de base de la race.
        let stats = stats_q.get_single().ok();
        let base = cfg.race.base_stats();
        root.spawn((
            Node { flex_direction: FlexDirection::Column, padding: UiRect::all(Val::Px(14.)), border: UiRect::all(Val::Px(2.)), row_gap: Val::Px(8.), width: Val::Px(620.), display: Display::None, ..default() },
            BackgroundColor(Color::srgb(0.10, 0.10, 0.16)),
            BorderColor(MC_BORDER),
            InvTabContent(InventoryTab::Stats),
        ))
        .with_children(|stats_panel| {
            let race_name = match &cfg.race {
                race::Race::Sylvaris  => "Sylvaris",
                race::Race::Ignaar    => "Ignaar",
                race::Race::Aethyn    => "Aethyn",
                race::Race::Vorkai    => "Vorkai",
                race::Race::Crysthari => "Crysthari",
            };
            stats_panel.spawn((Text::new(format!("Race : {}", race_name)), pf(f, 12.), TextColor(MC_YELLOW)));
            stats_panel.spawn((Text::new(format!("Nom : {}", cfg.name)), pf(f, 11.), TextColor(Color::WHITE)));
            stats_panel.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), ..default() }, BackgroundColor(MC_SEP)));

            if let Some(st) = stats {
                let stat_lines = [
                    format!("PV : {:.0} / {:.0}", st.current_hp, st.max_hp),
                    format!("Mana : {:.0} / {:.0}", st.current_mana, st.max_mana),
                    format!("Vitesse : {:.1}", st.speed),
                ];
                for line in stat_lines {
                    stats_panel.spawn((Text::new(line), pf(f, 10.), TextColor(Color::WHITE)));
                }
            }

            stats_panel.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), margin: UiRect::vertical(Val::Px(4.)), ..default() }, BackgroundColor(MC_SEP)));
            stats_panel.spawn((Text::new("Stats de base (race)"), pf(f, 11.), TextColor(MC_YELLOW)));

            let base_lines = [
                format!("PV max : {}", base.max_hp),
                format!("Mana max : {}", base.max_mana),
                format!("Vitesse : {:.1}", base.speed),
                format!("Degats melee : {:.2}", base.melee_dmg),
            ];
            for line in base_lines {
                stats_panel.spawn((Text::new(line), pf(f, 10.), TextColor(Color::srgba(0.7, 0.85, 1.0, 1.0))));
            }

            stats_panel.spawn((Node { width: Val::Percent(100.), height: Val::Px(2.), margin: UiRect::vertical(Val::Px(4.)), ..default() }, BackgroundColor(MC_SEP)));
            stats_panel.spawn((Text::new(format!("Competence raciale : {}", cfg.race.ability_name())), pf(f, 10.), TextColor(Color::srgba(1.0, 0.85, 0.5, 1.0))));
            stats_panel.spawn((Text::new(format!("Cooldown : {:.0}s", cfg.race.ability_cooldown())), pf(f, 8.), TextColor(Color::srgba(0.7, 0.7, 0.7, 0.9))));
        });

        root.spawn(Node { margin: UiRect { top: Val::Px(10.), ..default() }, ..default() })
        .with_child((Text::new("[E] / [Echap]  Fermer"), pf(f, 7.), TextColor(Color::srgba(0.5,0.5,0.5,0.8))));
    });

    // Icône flottante qui suit la souris quand le joueur tient un item.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width:  Val::Px(36.),
            height: Val::Px(36.),
            left:   Val::Px(-100.),
            top:    Val::Px(-100.),
            display: Display::None,
            ..default()
        },
        ImageNode {
            image: atlas_img.clone(),
            texture_atlas: Some(TextureAtlas {
                layout: atlas_layout.clone(),
                index:  0,
            }),
            ..default()
        },
        GlobalZIndex(100),
        HeldItemCursor,
        InventoryUI,
    ));
}

fn spawn_inv_slot(
    parent:       &mut ChildBuilder,
    kind:         InvSlotKind,
    index:        usize,
    item:         Option<(world::chunk::Block, u32)>,
    border_color: Color,
    atlas_img:    &Handle<Image>,
    atlas_layout: &Handle<TextureAtlasLayout>,
    font:         &Handle<Font>,
) {
    let bg = if item.is_some() {
        Color::srgba(0.20, 0.20, 0.20, 0.70)
    } else {
        Color::srgba(0.12, 0.12, 0.12, 0.40)
    };

    parent.spawn((
        Button,
        Node {
            width: Val::Px(48.), height: Val::Px(48.),
            border: UiRect::all(Val::Px(2.)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(bg),
        BorderColor(border_color),
        InvSlot(kind, index),
    ))
    .with_children(|slot| {
        let (icon_display, icon_index) = if let Some((block, _)) = item {
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
            InvSlotIcon(kind, index),
        ));
        let count_str = item.map(|(_, n)| format!("{}", n)).unwrap_or_default();
        slot.spawn((
            Text::new(count_str),
            TextFont { font: font.clone(), font_size: 9., ..default() },
            TextColor(Color::srgba(1., 1., 0.7, 0.95)),
            Node {
                position_type: PositionType::Absolute,
                right:  Val::Px(2.),
                bottom: Val::Px(1.),
                ..default()
            },
            InvSlotCount(kind, index),
        ));
    });
}

fn block_short_name(b: world::chunk::Block) -> &'static str {
    use world::chunk::Block::*;
    match b {
        Grass => "Herbe", Dirt => "Terre", Stone => "Pierre",
        Sand  => "Sable", Snow => "Neige",
        Wood  => "Bois",  Leaves => "Feuilles", Planks => "Planches", Ice => "Glace",
        Air   => "Vide",
    }
}

fn handle_inv_slot_click(
    btn_q:       Query<(&Interaction, &InvSlot)>,
    mouse:       Res<ButtonInput<MouseButton>>,
    mut held:    ResMut<HeldItem>,
    mut inv:     ResMut<crafting::BlockInventory>,
    mut hotbar:  ResMut<world::interaction::Hotbar>,
) {
    let left_just  = mouse.just_pressed(MouseButton::Left);
    let right_just = mouse.just_pressed(MouseButton::Right);
    if !left_just && !right_just { return; }

    for (interaction, slot) in &btn_q {
        // Bevy déclenche Interaction::Pressed uniquement pour le clic gauche.
        // Pour le clic droit on retombe sur Hovered (ou Pressed si la souris
        // clique simultanément des deux boutons).
        let is_over = matches!(*interaction, Interaction::Pressed | Interaction::Hovered);
        let hit_left  = left_just && *interaction == Interaction::Pressed;
        let hit_right = right_just && is_over;
        if !hit_left && !hit_right { continue; }

        let slot_val = match slot.0 {
            InvSlotKind::Backpack => inv.backpack[slot.1],
            InvSlotKind::Hotbar   => hotbar.slots[slot.1],
        };

        let (new_held, new_slot) = if hit_right {
            compute_right_click(held.0, slot_val)
        } else {
            compute_slot_swap(held.0, slot_val)
        };
        held.0 = new_held;

        match slot.0 {
            InvSlotKind::Backpack => inv.backpack[slot.1] = new_slot,
            InvSlotKind::Hotbar   => hotbar.slots[slot.1] = new_slot,
        }
    }
}

fn compute_slot_swap(
    held: Option<(world::chunk::Block, u32)>,
    slot: Option<(world::chunk::Block, u32)>,
) -> (Option<(world::chunk::Block, u32)>, Option<(world::chunk::Block, u32)>) {
    match (held, slot) {
        (None, s) => (s, None),
        (h, None) => (None, h),
        (Some((hb, hc)), Some((sb, sc))) if hb == sb => {
            // Même bloc : on empile dans le slot jusqu'à MAX_STACK.
            let space = crafting::MAX_STACK - sc;
            let transfer = hc.min(space);
            let new_slot = Some((sb, sc + transfer));
            let new_held = if hc - transfer > 0 { Some((hb, hc - transfer)) } else { None };
            (new_held, new_slot)
        }
        (h, s) => (s, h),
    }
}

/// Clic droit sur un slot (style Minecraft) :
/// - Main vide + slot plein  → prendre la moitié (arrondi haut dans la main)
/// - Main pleine + slot vide → poser 1 seul bloc
/// - Main pleine + même bloc → poser 1 bloc dans le slot (si pas plein)
/// - Main pleine + bloc diff → swap (comme clic gauche)
fn compute_right_click(
    held: Option<(world::chunk::Block, u32)>,
    slot: Option<(world::chunk::Block, u32)>,
) -> (Option<(world::chunk::Block, u32)>, Option<(world::chunk::Block, u32)>) {
    match (held, slot) {
        (None, Some((b, n))) => {
            let take = (n + 1) / 2;
            let remain = n - take;
            let new_slot = if remain > 0 { Some((b, remain)) } else { None };
            (Some((b, take)), new_slot)
        }
        (Some((hb, hc)), None) => {
            let new_held = if hc > 1 { Some((hb, hc - 1)) } else { None };
            (new_held, Some((hb, 1)))
        }
        (Some((hb, hc)), Some((sb, sc))) if hb == sb => {
            if sc < crafting::MAX_STACK {
                let new_held = if hc > 1 { Some((hb, hc - 1)) } else { None };
                (new_held, Some((sb, sc + 1)))
            } else {
                (Some((hb, hc)), Some((sb, sc)))
            }
        }
        (h, s) => (s, h),
    }
}

fn update_inv_slot_visuals(
    inv:         Res<crafting::BlockInventory>,
    hotbar:      Res<world::interaction::Hotbar>,
    held:        Res<HeldItem>,
    mut slot_q:  Query<(&InvSlot, &mut BackgroundColor, &mut BorderColor)>,
    mut icon_q:  Query<(&InvSlotIcon, &mut ImageNode, &mut Node)>,
    mut count_q: Query<(&InvSlotCount, &mut Text), Without<HeldItemDisplay>>,
    mut held_q:  Query<&mut Text, With<HeldItemDisplay>>,
) {
    for (slot, mut bg, mut border) in slot_q.iter_mut() {
        let item = match slot.0 {
            InvSlotKind::Backpack => inv.backpack[slot.1],
            InvSlotKind::Hotbar   => hotbar.slots[slot.1],
        };
        *bg = BackgroundColor(if item.is_some() {
            Color::srgba(0.20, 0.20, 0.20, 0.70)
        } else {
            Color::srgba(0.12, 0.12, 0.12, 0.40)
        });
        let is_hotbar_selected = slot.0 == InvSlotKind::Hotbar && slot.1 == hotbar.selected;
        *border = BorderColor(if is_hotbar_selected {
            Color::WHITE
        } else {
            Color::srgb(0.48, 0.48, 0.48)
        });
    }

    for (icon, mut img, mut node) in icon_q.iter_mut() {
        let item = match icon.0 {
            InvSlotKind::Backpack => inv.backpack[icon.1],
            InvSlotKind::Hotbar   => hotbar.slots[icon.1],
        };
        if let Some((block, _)) = item {
            if let Some(ref mut atlas) = img.texture_atlas {
                atlas.index = block.tile_side();
            }
            node.display = Display::Flex;
        } else {
            node.display = Display::None;
        }
    }

    for (cnt, mut text) in count_q.iter_mut() {
        let item = match cnt.0 {
            InvSlotKind::Backpack => inv.backpack[cnt.1],
            InvSlotKind::Hotbar   => hotbar.slots[cnt.1],
        };
        text.0 = item.map(|(_, n)| format!("{}", n)).unwrap_or_default();
    }

    if let Ok(mut text) = held_q.get_single_mut() {
        text.0 = match held.0 {
            Some((block, n)) => format!("En main: {} x{}", block_short_name(block), n),
            None => String::new(),
        };
    }
}

/// À la fermeture de l'inventaire, on remet l'item tenu en main dans
/// le sac à dos pour ne pas le perdre.
fn cleanup_held_item(
    mut commands: Commands,
    held:         Res<HeldItem>,
    mut inv:      ResMut<crafting::BlockInventory>,
    mut hotbar:   ResMut<world::interaction::Hotbar>,
) {
    if let Some((block, n)) = held.0 {
        crafting::add_to_all(&mut inv, &mut hotbar, block, n);
    }
    commands.remove_resource::<HeldItem>();
}

fn update_held_cursor(
    held:        Res<HeldItem>,
    windows:     Query<&Window>,
    mut cursor_q: Query<(&mut Node, &mut ImageNode), With<HeldItemCursor>>,
) {
    let Ok((mut node, mut img)) = cursor_q.get_single_mut() else { return };
    match held.0 {
        Some((block, _)) => {
            node.display = Display::Flex;
            if let Some(ref mut atlas) = img.texture_atlas {
                atlas.index = block.tile_side();
            }
            if let Ok(window) = windows.get_single() {
                if let Some(pos) = window.cursor_position() {
                    // On décale de 18 px pour centrer l'icône 36×36 sur le curseur.
                    node.left = Val::Px(pos.x - 18.);
                    node.top  = Val::Px(pos.y - 18.);
                }
            }
        }
        None => {
            node.display = Display::None;
        }
    }
}

fn handle_craft_buttons(
    btn_q:        Query<(&Interaction, &CraftButton), Changed<Interaction>>,
    mut requests: EventWriter<crafting::CraftRequest>,
) {
    for (interaction, btn) in &btn_q {
        if *interaction == Interaction::Pressed {
            requests.send(crafting::CraftRequest { recipe_index: btn.0 });
        }
    }
}

fn handle_inventory_tabs(
    mut tab:       ResMut<InventoryTab>,
    btn_q:         Query<(&Interaction, &InvTabButton), Changed<Interaction>>,
    mut content_q: Query<(&InvTabContent, &mut Node)>,
    mut tab_btn_q: Query<(&InvTabButton, &mut BackgroundColor)>,
) {
    let active_bg   = Color::srgb(0.30, 0.30, 0.45);
    let inactive_bg = Color::srgb(0.15, 0.15, 0.20);

    let mut changed = false;
    for (interaction, btn) in &btn_q {
        if *interaction == Interaction::Pressed && *tab != btn.0 {
            *tab = btn.0;
            changed = true;
        }
    }
    if !changed { return; }

    for (content, mut node) in &mut content_q {
        node.display = if content.0 == *tab { Display::Flex } else { Display::None };
    }
    for (btn, mut bg) in &mut tab_btn_q {
        *bg = BackgroundColor(if btn.0 == *tab { active_bg } else { inactive_bg });
    }
}

fn despawn_inventory_ui(mut commands: Commands, q: Query<Entity, With<InventoryUI>>) {
    for e in &q { commands.entity(e).despawn_recursive(); }
}

fn cleanup_spell_slot(mut commands: Commands) {
    commands.remove_resource::<SelectedSpellSlot>();
}

/// Clic sur un slot du loadout → sélectionne ce slot comme cible d'assignation.
fn handle_spell_loadout_click(
    btn_q:     Query<(&Interaction, &SpellLoadoutSlot), Changed<Interaction>>,
    mut sel:   ResMut<SelectedSpellSlot>,
    mut all_q: Query<(&SpellLoadoutSlot, &mut BorderColor)>,
) {
    let mut changed = false;
    for (interaction, slot) in &btn_q {
        if *interaction == Interaction::Pressed {
            sel.0 = slot.0;
            changed = true;
        }
    }
    if !changed { return; }
    for (slot, mut bd) in &mut all_q {
        *bd = BorderColor(if slot.0 == sel.0 { Color::WHITE } else { Color::srgba(0.5, 0.5, 0.5, 0.7) });
    }
}

/// Clic sur un sort dans la liste → l'assigne au slot sélectionné du
/// loadout. Si le sort était déjà équipé ailleurs, on échange (swap).
fn handle_spell_assign_click(
    btn_q:         Query<(&Interaction, &SpellAssignButton), Changed<Interaction>>,
    sel:           Res<SelectedSpellSlot>,
    mut spell_bar: ResMut<magic::SpellBar>,
    mut slot_q:    Query<(&SpellLoadoutSlot, &mut BackgroundColor, &Children)>,
    mut text_q:    Query<&mut Text>,
) {
    for (interaction, btn) in &btn_q {
        if *interaction != Interaction::Pressed { continue; }
        let spell = btn.0;
        let slot_idx = sel.0;

        // Si le sort était déjà équipé dans un autre slot, on y met
        // l'ancien contenu du slot cible (comportement de swap).
        for i in 0..5 {
            if i != slot_idx && spell_bar.slots[i] == Some(spell) {
                spell_bar.slots[i] = spell_bar.slots[slot_idx];
            }
        }
        spell_bar.slots[slot_idx] = Some(spell);

        for (ls, mut bg, children) in &mut slot_q {
            let s = spell_bar.slots[ls.0];
            *bg = BackgroundColor(s.map(|sp| sp.slot_color(false)).unwrap_or(Color::srgba(0.12, 0.12, 0.12, 0.70)));
            if let Some(&child) = children.iter().next() {
                if let Ok(mut text) = text_q.get_mut(child) {
                    text.0 = s.map(|sp| sp.name()).unwrap_or("-").to_string();
                }
            }
        }
    }
}

#[derive(Component)] struct DeathScreenUI;
#[derive(Component)] struct RespawnButton;

fn check_player_death(
    mut next:  ResMut<NextState<GameState>>,
    stats_q:   Query<&player::PlayerStats>,
    mut win_q: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Ok(stats) = stats_q.get_single() else { return };
    if stats.current_hp <= 0.0 {
        if let Ok(mut win) = win_q.get_single_mut() {
            win.cursor_options.grab_mode = CursorGrabMode::None;
            win.cursor_options.visible   = true;
        }
        next.set(GameState::Dead);
    }
}

fn spawn_death_screen(mut commands: Commands, font: Res<PixelFont>) {
    let f = &font.0;
    commands.spawn((
        Node { width: Val::Percent(100.), height: Val::Percent(100.), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, justify_content: JustifyContent::Center, ..default() },
        BackgroundColor(Color::srgba(0.20, 0.0, 0.0, 0.78)),
        DeathScreenUI,
    ))
    .with_children(|root| {
        let (node, bg, border) = mc_panel(360., 32.);
        root.spawn((node, bg, border)).with_children(|p| {
            p.spawn((Text::new("VOUS ETES MORT"), pf(f, 20.), TextColor(Color::srgb(1.0, 0.25, 0.25))));
            mc_sep(p);
            p.spawn((Text::new("Votre aventure prend fin... pour l'instant."), pf(f, 8.), TextColor(Color::srgba(0.85, 0.85, 0.85, 1.0))));
            p.spawn((Node { height: Val::Px(12.), ..default() },));
            mc_btn(p, "Reapparaitre",     ButtonColors::green(), RespawnButton,                      f, false);
            mc_btn(p, "Menu Principal",   ButtonColors::gray(),  PauseButton(PauseAction::MainMenu), f, false);
        });
    });
}

fn despawn_death_screen(mut commands: Commands, q: Query<Entity, With<DeathScreenUI>>) {
    for e in &q { commands.entity(e).despawn_recursive(); }
}

fn handle_death_screen(
    mut commands: Commands,
    mut next:     ResMut<NextState<GameState>>,
    cfg:          Res<PlayerConfig>,
    mut stats_q:  Query<(Entity, &mut player::PlayerStats, &mut Transform), With<player::Player>>,
    mut win_q:    Query<&mut Window, With<PrimaryWindow>>,
    respawn_q:    Query<&Interaction, (Changed<Interaction>, With<RespawnButton>)>,
    mut pause_q:  Query<(&Interaction, &PauseButton), (Changed<Interaction>, With<Button>)>,
) {
    for int in &respawn_q {
        if *int != Interaction::Pressed { continue; }
        if let Ok((entity, mut stats, mut tf)) = stats_q.get_single_mut() {
            let base = cfg.race.base_stats();
            stats.current_hp   = base.max_hp as f32;
            stats.current_mana = base.max_mana as f32;
            tf.translation = Vec3::new(8.0, 200.0, 8.0);
            // NeedsGrounding fera tomber le joueur sur la surface à la
            // prochaine frame (évite de respawn coincé dans un bloc).
            commands.entity(entity).insert(player::NeedsGrounding);
        }
        if let Ok(mut win) = win_q.get_single_mut() {
            win.cursor_options.grab_mode = CursorGrabMode::Confined;
            win.cursor_options.visible   = false;
        }
        next.set(GameState::InGame);
        return;
    }
    for (int, btn) in &mut pause_q {
        if *int != Interaction::Pressed { continue; }
        if matches!(btn.0, PauseAction::MainMenu) {
            next.set(GameState::MainMenu);
        }
    }
}

fn spawn_world(_commands: Commands, _cfg: Res<PlayerConfig>) {}

/// Nettoyage quand on revient au menu principal depuis une partie : on
/// despawne le joueur et tous les chunks. Idempotent (no-op au premier
/// lancement). La caméra est détachée du joueur avant despawn pour qu'elle
/// ne parte pas avec l'arbre d'entités.
fn cleanup_world_on_main_menu(
    mut commands:      Commands,
    mut chunk_manager: ResMut<world::chunk::ChunkManager>,
    mut block_edits:   ResMut<world::chunk::BlockEdits>,
    mut lod_mgr:       ResMut<world::lod::LodManager>,
    player_q:          Query<Entity, With<player::Player>>,
    camera_q:          Query<Entity, (With<Camera3d>, With<player::PlayerCamera>)>,
    chunk_q:           Query<Entity, With<world::chunk::Chunk>>,
    lod_q:             Query<Entity, With<world::lod::LodChunk>>,
    arm_view_q:        Query<Entity, With<player::animation::ArmViewMesh>>,
    arm_cam_q:         Query<Entity, With<player::animation::ArmCam>>,
    loading_q:         Query<Entity, With<LoadingScreenUI>>,
    preview_chars:     Query<Entity, With<player::preview::PreviewCharacter>>,
    preview_cams:      Query<Entity, With<player::preview::PreviewCam>>,
) {
    // Retire l'écran de chargement s'il est encore visible (warmup en cours).
    for e in &loading_q { commands.entity(e).despawn_recursive(); }
    commands.remove_resource::<InGameWarmup>();
    // Nettoie la preview RTT si elle traîne encore.
    for e in &preview_chars { commands.entity(e).despawn_recursive(); }
    for e in &preview_cams  { commands.entity(e).despawn_recursive(); }
    commands.remove_resource::<player::preview::PreviewTarget>();
    // Détache la caméra du joueur pour la conserver pour la prochaine partie.
    for cam in &camera_q {
        commands.entity(cam)
            .remove_parent()
            .remove::<player::PlayerCamera>()
            .insert(Transform::default());
    }
    // Despawn le bras first-person et sa caméra dédiée : ils seront
    // recréés à la prochaine entrée InGame avec la couleur de peau de la
    // race choisie.
    for e in &arm_view_q { commands.entity(e).despawn_recursive(); }
    for e in &arm_cam_q  { commands.entity(e).despawn_recursive(); }
    for player in &player_q {
        commands.entity(player).despawn_recursive();
    }
    for chunk in &chunk_q {
        commands.entity(chunk).despawn_recursive();
    }
    for lod in &lod_q {
        commands.entity(lod).despawn_recursive();
    }
    chunk_manager.full_clear();
    lod_mgr.clear();
    // Reset des modifications de blocs : la prochaine partie repartira soit
    // sur des chunks vierges (nouvelle partie), soit avec les edits chargés
    // depuis le sidecar (chargement de slot).
    block_edits.clear();
}

fn relock_cursor(
    mut commands: Commands,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if let Ok(mut win) = windows.get_single_mut() {
        win.cursor_options.grab_mode = CursorGrabMode::Confined;
        win.cursor_options.visible   = false;
        // Deux .set() dans la même frame sont coalescés par Bevy avant
        // d'être envoyés à winit : un kick inline ne produirait aucun
        // event. On étale le delta sur 2 frames via PendingResizeKick.
        let w = win.resolution.width();
        let h = win.resolution.height();
        commands.insert_resource(PendingResizeKick { phase: 0, original_w: w, original_h: h });
    }
}

/// Kick swapchain : force un vrai `WindowEvent::Resized` à winit en
/// appliquant un delta de 1 px sur une frame, puis en restaurant la
/// taille d'origine. Sans ça, sur Intel HD 520 + Windows + Fifo, le
/// premier present peut rester bloqué jusqu'à ce qu'un vrai resize
/// utilisateur survienne.
#[derive(Resource)]
struct PendingResizeKick { phase: u8, original_w: f32, original_h: f32 }

fn apply_resize_kick(
    mut commands: Commands,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    kick: Option<ResMut<PendingResizeKick>>,
) {
    let Some(mut k) = kick else { return };
    let Ok(mut win) = windows.get_single_mut() else { return };
    match k.phase {
        0 => {
            win.resolution.set(k.original_w + 1.0, k.original_h);
            k.phase = 1;
        }
        _ => {
            win.resolution.set(k.original_w, k.original_h);
            commands.remove_resource::<PendingResizeKick>();
        }
    }
}
