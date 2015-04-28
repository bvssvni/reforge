#[cfg(feature = "client")]
use graphics::Context;
#[cfg(feature = "client")]
use opengl_graphics::Gl;

use battle_context::BattleContext;
use module;
use module::{IModule, Module, ModuleBase, ModuleRef, TargetManifest};
use net::{InPacket, OutPacket};
use ship::{Ship, ShipState};
use sim::SimEvents;
use vec::{Vec2, Vec2f};

#[cfg(feature = "client")]
use sim_visuals::SpriteVisual;
#[cfg(feature = "client")]
use sim::{SimEffects, SimVisual};
#[cfg(feature = "client")]
use sprite_sheet::{SpriteSheet, SpriteAnimation};
#[cfg(feature = "client")]
use asset_store::AssetStore;

#[derive(RustcEncodable, RustcDecodable, Clone)]
pub struct ShieldModule;

impl ShieldModule {
    pub fn new() -> Module<ShieldModule>{
        Module {
            base: ModuleBase::new(1, 1, 2, 2, 3),
            module: ShieldModule,
        }
    }
}

impl IModule for ShieldModule {    
    #[cfg(feature = "client")]
    fn add_plan_effects(&self, base: &ModuleBase, asset_store: &AssetStore, effects: &mut SimEffects, ship: &Ship) {
        let mut shield_sprite = SpriteSheet::new(asset_store.get_sprite_info_str("modules/shield_sprite.png"));
        
        if base.is_active() {
            shield_sprite.add_animation(SpriteAnimation::Loop(0.0, 7.0, 0, 9, 0.05));
        } else {
            shield_sprite.add_animation(SpriteAnimation::Stay(0.0, 7.0, 0));
        }
    
        effects.add_visual(ship.id, 0, box SpriteVisual {
            position: base.get_render_position().clone(),
            sprite_sheet: shield_sprite,
        });
    }
    
    #[cfg(feature = "client")]
    fn add_simulation_effects(&self, base: &ModuleBase, asset_store: &AssetStore, effects: &mut SimEffects, ship: &Ship, target: Option<TargetManifest>) {
        self.add_plan_effects(base, asset_store, effects, ship);
    }
    
    fn after_simulation(&mut self, base: &mut ModuleBase, ship_state: &mut ShipState) {
        if base.powered && ship_state.shields < ship_state.max_shields {
            ship_state.shields += 1; // charge shield
        }
    }
    
    fn on_activated(&mut self, ship_state: &mut ShipState) {
        ship_state.add_shields(2);
    }
    
    fn on_deactivated(&mut self, ship_state: &mut ShipState) {
        ship_state.remove_shields(2);
    }
    
    fn get_target_mode(&self, base: &ModuleBase) -> Option<module::TargetMode> {
        None
    }
}
