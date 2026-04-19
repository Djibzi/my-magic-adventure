pub mod ability;

use rand::Rng;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Race {
    #[default]
    Sylvaris,
    Ignaar,
    Aethyn,
    Vorkai,
    Crysthari,
}

impl Race {
    /// Choisit une race au hasard avec poids égaux
    pub fn random() -> Self {
        let mut rng = rand::thread_rng();
        match rng.gen_range(0..5) {
            0 => Race::Sylvaris,
            1 => Race::Ignaar,
            2 => Race::Aethyn,
            3 => Race::Vorkai,
            _ => Race::Crysthari,
        }
    }

    /// Nom court de la competence raciale active (touche F)
    pub fn ability_name(&self) -> &'static str {
        match self {
            Race::Sylvaris  => "Enracinement",
            Race::Ignaar    => "Eruption",
            Race::Aethyn    => "Bourrasque",
            Race::Vorkai    => "Voile d'Ombre",
            Race::Crysthari => "Eclat Prismatique",
        }
    }

    /// Cooldown de la competence raciale active (en secondes)
    pub fn ability_cooldown(&self) -> f32 {
        match self {
            Race::Sylvaris  => 45.0,
            Race::Ignaar    => 60.0,
            Race::Aethyn    => 20.0,
            Race::Vorkai    => 40.0,
            Race::Crysthari => 35.0,
        }
    }

    /// Stats de base selon la race
    pub fn base_stats(&self) -> RaceStats {
        match self {
            Race::Sylvaris  => RaceStats { max_hp: 100, max_mana: 80,  speed: 1.0, melee_dmg: 1.0 },
            Race::Ignaar    => RaceStats { max_hp: 130, max_mana: 60,  speed: 0.9, melee_dmg: 1.15 },
            Race::Aethyn    => RaceStats { max_hp: 80,  max_mana: 100, speed: 1.2, melee_dmg: 0.9 },
            Race::Vorkai    => RaceStats { max_hp: 90,  max_mana: 90,  speed: 1.1, melee_dmg: 1.0 },
            Race::Crysthari => RaceStats { max_hp: 85,  max_mana: 125, speed: 1.0, melee_dmg: 0.85 },
        }
    }
}

#[derive(Debug, Clone)]
pub struct RaceStats {
    pub max_hp:    u32,
    pub max_mana:  u32,
    pub speed:     f32,
    pub melee_dmg: f32,
}
