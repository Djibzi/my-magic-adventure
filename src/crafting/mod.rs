//! Inventaire et crafting Alpha.
//!
//! - `BlockInventory` : 18 slots de sac à dos, chacun une pile de 64 max.
//! - `BlockCollected` : event émis par le ramassage de drops.
//! - `Recipe` + `CraftRequest` : crafting simple déclenché depuis l'UI.
//!
//! La hotbar (9 slots visibles en bas d'écran) vit dans `world::interaction` ;
//! on la manipule ici pour que ramasser auto-range dans la hotbar si possible.

use bevy::prelude::*;

use crate::world::chunk::Block;
use crate::world::interaction::Hotbar;

pub const MAX_STACK: u32 = 64;

#[derive(Resource)]
pub struct BlockInventory {
    pub backpack: [Option<(Block, u32)>; 18],
}

impl Default for BlockInventory {
    fn default() -> Self {
        Self { backpack: [None; 18] }
    }
}

impl BlockInventory {
    /// Nombre d'exemplaires d'un bloc uniquement dans le sac à dos.
    pub fn count(&self, block: Block) -> u32 {
        self.backpack.iter()
            .filter_map(|s| s.as_ref())
            .filter(|(b, _)| *b == block)
            .map(|(_, n)| *n)
            .sum()
    }
}

/// Total d'un bloc dans hotbar + sac à dos réunis.
pub fn total_count(inv: &BlockInventory, hotbar: &Hotbar, block: Block) -> u32 {
    let bp: u32 = inv.backpack.iter()
        .filter_map(|s| s.as_ref())
        .filter(|(b, _)| *b == block)
        .map(|(_, n)| *n)
        .sum();
    let hb: u32 = hotbar.slots.iter()
        .filter_map(|s| s.as_ref())
        .filter(|(b, _)| *b == block)
        .map(|(_, n)| *n)
        .sum();
    bp + hb
}

/// Ajoute des blocs à l'inventaire en suivant un ordre intentionnel :
/// d'abord compléter les piles existantes dans la hotbar, puis dans le sac,
/// puis auto-assigner un slot hotbar vide (si le bloc n'y est pas déjà),
/// puis remplir les slots vides du sac, et enfin déborder dans la hotbar.
/// Ce séquencement garantit qu'un bloc déjà placé par le joueur dans la
/// hotbar y reste et que les nouveaux types n'y écrasent pas un slot rempli.
pub fn add_to_all(inv: &mut BlockInventory, hotbar: &mut Hotbar, block: Block, mut n: u32) {
    // 1. Complète les piles hotbar existantes du même bloc.
    for slot in hotbar.slots.iter_mut() {
        if n == 0 { return; }
        if let Some((b, count)) = slot {
            if *b == block && *count < MAX_STACK {
                let add = n.min(MAX_STACK - *count);
                *count += add;
                n -= add;
            }
        }
    }
    // 2. Pareil pour le sac.
    for slot in inv.backpack.iter_mut() {
        if n == 0 { return; }
        if let Some((b, count)) = slot {
            if *b == block && *count < MAX_STACK {
                let add = n.min(MAX_STACK - *count);
                *count += add;
                n -= add;
            }
        }
    }
    // 3. Auto-assignation : slot hotbar vide, seulement si le bloc n'est pas
    //    déjà assigné ailleurs dans la hotbar (évite les doublons visuels).
    let in_hotbar = hotbar.slots.iter().any(|s| s.map(|(b, _)| b) == Some(block));
    if !in_hotbar {
        for slot in hotbar.slots.iter_mut() {
            if n == 0 { return; }
            if slot.is_none() {
                let add = n.min(MAX_STACK);
                *slot = Some((block, add));
                n -= add;
                break;
            }
        }
    }
    // 4. Slots vides du sac.
    for slot in inv.backpack.iter_mut() {
        if n == 0 { return; }
        if slot.is_none() {
            let add = n.min(MAX_STACK);
            *slot = Some((block, add));
            n -= add;
        }
    }
    // 5. Débordement sur les slots hotbar restants.
    for slot in hotbar.slots.iter_mut() {
        if n == 0 { return; }
        if slot.is_none() {
            let add = n.min(MAX_STACK);
            *slot = Some((block, add));
            n -= add;
        }
    }
}

/// Retire `n` exemplaires d'un bloc en commençant par le sac (pour garder la
/// hotbar intacte le plus longtemps possible). Renvoie `false` sans rien
/// toucher si la quantité demandée n'est pas dispo — crafting atomique.
pub fn take_from_all(inv: &mut BlockInventory, hotbar: &mut Hotbar, block: Block, mut n: u32) -> bool {
    if total_count(inv, hotbar, block) < n { return false; }
    for slot in inv.backpack.iter_mut().rev() {
        if n == 0 { break; }
        if let Some((b, count)) = slot {
            if *b == block {
                let take = n.min(*count);
                *count -= take;
                n -= take;
                if *count == 0 { *slot = None; }
            }
        }
    }
    for slot in hotbar.slots.iter_mut().rev() {
        if n == 0 { break; }
        if let Some((b, count)) = slot {
            if *b == block {
                let take = n.min(*count);
                *count -= take;
                n -= take;
                if *count == 0 { *slot = None; }
            }
        }
    }
    true
}

#[derive(Event)]
pub struct BlockCollected { pub block: Block }

#[derive(Event)]
pub struct CraftRequest { pub recipe_index: usize }

/// Une recette de crafting : un ou plusieurs inputs consommés, un output
/// produit. Les quantités sont en blocs unitaires.
#[derive(Clone)]
pub struct Recipe {
    pub name:    &'static str,
    pub inputs:  &'static [(Block, u32)],
    pub output:  (Block, u32),
}

pub const RECIPES: &[Recipe] = &[
    Recipe {
        name: "Planches",
        inputs: &[(Block::Wood, 1)],
        output: (Block::Planks, 4),
    },
    Recipe {
        name: "Bois comprime",
        inputs: &[(Block::Planks, 4)],
        output: (Block::Wood, 1),
    },
    Recipe {
        name: "Bloc de glace",
        inputs: &[(Block::Snow, 4)],
        output: (Block::Ice, 1),
    },
    Recipe {
        name: "Sable vitrifie",
        inputs: &[(Block::Sand, 2), (Block::Stone, 1)],
        output: (Block::Ice, 2),
    },
    Recipe {
        name: "Herbe",
        inputs: &[(Block::Dirt, 4), (Block::Leaves, 1)],
        output: (Block::Grass, 4),
    },
    Recipe {
        name: "Pierre taillee",
        inputs: &[(Block::Stone, 4)],
        output: (Block::Stone, 4),
    },
];

pub struct CraftingPlugin;

impl Plugin for CraftingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BlockInventory>()
           .add_event::<BlockCollected>()
           .add_event::<CraftRequest>()
           .add_systems(Update, (
               apply_block_events,
               apply_craft_requests,
           ));
    }
}

/// Route les events `BlockCollected` vers l'inventaire (sauf Air qui ne devrait
/// jamais arriver ici mais on blinde quand même).
fn apply_block_events(
    mut inv:       ResMut<BlockInventory>,
    mut collected: EventReader<BlockCollected>,
    mut hotbar:    ResMut<Hotbar>,
) {
    for ev in collected.read() {
        if ev.block != Block::Air {
            add_to_all(&mut inv, &mut hotbar, ev.block, 1);
        }
    }
}

/// Applique une recette : vérifie que tous les inputs sont disponibles (en
/// tenant compte hotbar+sac), les retire, puis ajoute l'output. Si un input
/// manque, l'event est ignoré silencieusement — l'UI empêche normalement de
/// cliquer sur une recette impossible.
fn apply_craft_requests(
    mut inv:      ResMut<BlockInventory>,
    mut requests: EventReader<CraftRequest>,
    mut hotbar:   ResMut<Hotbar>,
) {
    for req in requests.read() {
        let Some(recipe) = RECIPES.get(req.recipe_index) else { continue };
        let ok = recipe.inputs.iter().all(|(b, n)| total_count(&inv, &hotbar, *b) >= *n);
        if !ok { continue; }
        for (b, n) in recipe.inputs { take_from_all(&mut inv, &mut hotbar, *b, *n); }
        let (ob, on) = recipe.output;
        add_to_all(&mut inv, &mut hotbar, ob, on);
    }
}

/// Prédicat utilisé par l'UI pour griser les recettes non réalisables.
pub fn can_craft(inv: &BlockInventory, hotbar: &Hotbar, recipe: &Recipe) -> bool {
    recipe.inputs.iter().all(|(b, n)| total_count(inv, hotbar, *b) >= *n)
}
