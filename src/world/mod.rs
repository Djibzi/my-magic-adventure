pub mod breaking;
pub mod chunk;
pub mod drops;
pub mod generation;
pub mod interaction;
pub mod lod;
pub mod texture_atlas;

use bevy::prelude::*;
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use chunk::ChunkPlugin;
use breaking::BlockBreakingPlugin;
use drops::DropsPlugin;
use interaction::BlockInteractionPlugin;
use lod::LodPlugin;

// Re-export pour les autres modules
pub use chunk::BlockAtlas;

/// Layout atlas pour les icônes de blocs en UI (11 tuiles 16×16).
#[derive(Resource)]
pub struct BlockIconAtlas {
    pub layout: Handle<TextureAtlasLayout>,
}

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ChunkPlugin)
           .add_plugins(BlockInteractionPlugin)
           .add_plugins(BlockBreakingPlugin)
           .add_plugins(DropsPlugin)
           .add_plugins(LodPlugin)
           .add_systems(Startup, (init_block_atlas, init_block_icon_atlas))
           .add_systems(Update, set_atlas_sampler.run_if(resource_exists::<BlockAtlas>));
    }
}

/// Charge l'atlas PNG depuis assets/blocks.png
fn init_block_atlas(mut commands: Commands, asset_server: Res<AssetServer>) {
    let handle = asset_server.load("blocks.png");
    commands.insert_resource(BlockAtlas { handle });
}

/// Crée le TextureAtlasLayout pour les icônes de blocs (11 tuiles 16×16).
fn init_block_icon_atlas(
    mut commands: Commands,
    mut layouts:  ResMut<Assets<TextureAtlasLayout>>,
) {
    let layout = TextureAtlasLayout::from_grid(UVec2::new(16, 16), 11, 1, None, None);
    commands.insert_resource(BlockIconAtlas { layout: layouts.add(layout) });
}

/// Force le sampler nearest-neighbor sur l'atlas dès qu'il est chargé (une seule fois)
fn set_atlas_sampler(
    atlas:    Res<BlockAtlas>,
    mut images: ResMut<Assets<Image>>,
    mut done: Local<bool>,
) {
    if *done { return; }
    if let Some(img) = images.get_mut(&atlas.handle) {
        img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::nearest());
        *done = true;
    }
}
