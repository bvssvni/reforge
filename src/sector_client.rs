use std::cell::RefCell;
use std::cmp;
use std::io;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::thread;
use std::time::Duration;
use time;

use event::Events;
use opengl_graphics::Gl;
use opengl_graphics::glyph_cache::GlyphCache;
use sdl2_window::Sdl2Window;

use asset_store::AssetStore;
use battle_context::{BattleContext, ClientPacketId, ServerPacketId, TICKS_PER_SECOND};
use net::{Client, InPacket, OutPacket};
use sector_data::SectorData;
use ship::{Ship, ShipId, ShipIndex, ShipRef};
use sim::{SimEvents, SimEffects};
use space_gui::SpaceGui;

#[derive(PartialEq)]
pub enum ExitMode {
    Jump,
    Logout,
}

pub struct ClientBattleState<'a> {
    client: &'a mut Client,
    
    // Context holding all the things involved in this battle
    context: BattleContext,
    
    // The player's ship
    player_ship: ShipRef,
    
    new_ships_pre: Option<InPacket>,
    results: Option<InPacket>,
    new_ships_post: Option<InPacket>,
    
    last_tick: bool,
}

impl<'a> ClientBattleState<'a> {
    pub fn new(client: &'a mut Client, context: BattleContext) -> ClientBattleState<'a> {
        let player_ship = context.get_ship_by_client_id(client.get_id()).clone();
        ClientBattleState {
            client: client,
            context: context,
            player_ship: player_ship,
            new_ships_pre: None,
            results: None,
            new_ships_post: None,
            last_tick: false,
        }
    }
    
    pub fn run(&mut self, window: &Rc<RefCell<Sdl2Window>>, gl: &mut Gl, glyph_cache: &mut GlyphCache, asset_store: &AssetStore, sectors: Vec<SectorData>, server_results_sent: bool) -> ExitMode {
        use window::ShouldClose;
        use quack::Get;
    
        let ref mut gui = SpaceGui::new(asset_store, &self.context, sectors, self.player_ship.borrow().index);
    
        let ref mut sim_effects = SimEffects::new();
        
        // TODO display joining screen here
        
        if server_results_sent {
            // Wait for the tick
            let mut tick_packet = self.client.receive();
            self.handle_tick_packet(&mut tick_packet);
        }
        
        // Get first turn's results
        self.receive_new_ships(gui);
        self.receive_simulation_results();
        let mut new_ships_post = self.client.receive();
        
        // Receive tick
        let mut tick_packet = self.client.receive();
        self.handle_tick_packet(&mut tick_packet);
        
        self.run_simulation_phase(window, gl, glyph_cache, asset_store, gui, sim_effects);
        
        self.handle_new_ships_packet(gui, &mut new_ships_post);
        
        // Check if player jumped
        if self.player_ship.borrow().jumping {
            return ExitMode::Jump;
        }
    
        loop {
            ////////////////////////////////
            // Simulate
            
            // Receive simulation results
            let mut new_ships_pre = self.new_ships_pre.take().expect("New ships pre packet must exist here");
            let mut results = self.results.take().expect("Results packet must exist here");
            let mut new_ships_post = self.new_ships_post.take().expect("New ships post packet must exist here");
            
            self.handle_new_ships_packet(gui, &mut new_ships_pre);
            self.handle_simulation_results(&mut results);
            
            self.run_simulation_phase(window, gl, glyph_cache, asset_store, gui, sim_effects);
            
            // Receive ships after sim
            self.handle_new_ships_packet(gui, &mut new_ships_post);
            
            // Check if it's time to exit
            let ShouldClose(should_close) = window.borrow().get();
            if should_close { break; }
            
            // Check if player jumped
            if self.player_ship.borrow().jumping {
                break;
            }
        }
        
        ExitMode::Jump
    }
    
    fn run_simulation_phase(&mut self, window: &Rc<RefCell<Sdl2Window>>, gl: &mut Gl, glyph_cache: &mut GlyphCache, asset_store: &AssetStore, gui: &mut SpaceGui, mut sim_effects: &mut SimEffects) {
        // Unlock any exploding or jumping ships
        for ship in self.context.ships_iter() {
            let ship_index = ship.borrow().index;
            
            if ship.borrow().jumping || ship.borrow().exploding {
                // Remove all locks
                self.context.on_ship_removed(ship_index);
            }
        }
        
        let mut sim_events = SimEvents::new();
            
        // Before simulation
        sim_effects.reset();
        self.context.before_simulation(&mut sim_events);
        self.context.add_simulation_effects(asset_store, &mut sim_effects);
        
        // Simulation
        let start_time = time::now().to_timespec();
        let mut next_tick = 0;
        let mut plans_sent = false;
        let mut new_ships_pre_received = false;
        let mut results_received = false;
        let mut new_ships_post_received = false;
        for e in Events::new(window.clone()) {
            use event;
            use input;
            use event::*;

            let e: event::Event<input::Input> = e;
        
            // Calculate a bunch of time stuff
            let current_time = time::now().to_timespec();
            let elapsed_time = current_time - start_time;
            let elapsed_seconds = (elapsed_time.num_milliseconds() as f64)/1000.0;
            
            if !self.last_tick && !self.player_ship.borrow().exploding && !plans_sent && elapsed_seconds >= 2.5 {
                // Send plans
                let packet = self.build_plans_packet();
                self.client.send(&packet);
                plans_sent = true;
                println!("Sent plans at {}", elapsed_seconds);
            }
            
            if !self.last_tick {
                if plans_sent || self.player_ship.borrow().exploding {
                    if !new_ships_pre_received {
                        if let Ok(packet) = self.client.try_receive() {
                            self.new_ships_pre = Some(packet);
                            new_ships_pre_received = true;
                        }
                    } else if !results_received {
                        if let Ok(packet) = self.client.try_receive() {
                            self.results = Some(packet);
                            results_received = true;
                        }
                    } else if !new_ships_post_received {
                        if let Ok(packet) = self.client.try_receive() {
                            self.new_ships_post = Some(packet);
                            new_ships_post_received = true;
                        }
                    } else {
                        // Wait for tick
                        if let Ok(mut packet) = self.client.try_receive() {
                            self.handle_tick_packet(&mut packet);
                            
                            // If the tick we got isn't the last tick, this turn is done.
                            if !self.last_tick {
                                println!("Finished turn at {}", elapsed_seconds);
                                break;
                            }
                        }
                    }
                }
            } else if elapsed_seconds >= 5.0 {
                println!("Finished turn because we're leaving this state");
                break;
            }
            
            // Calculate current tick
            let tick = (elapsed_time.num_milliseconds() as u32)/(1000/TICKS_PER_SECOND);
            
            // Simulate any new ticks
            if next_tick < 100 {
                for t in next_tick .. cmp::min(next_tick+tick-next_tick+1, 100) {
                    sim_events.apply_tick(&self.context, t);
                }
                next_tick = tick+1;
            }
        
            // Forward events to GUI
            gui.event(&self.context, &e, &self.player_ship);
            
            // Render GUI
            e.render(|args: &RenderArgs| {
                gl.draw([0, 0, args.width as i32, args.height as i32], |c, gl| {
                    gui.draw_simulating(&self.context, &c, gl, glyph_cache, asset_store, &mut sim_effects, self.player_ship.borrow_mut().deref_mut(), elapsed_seconds, (1.0/60.0) + args.ext_dt);
                });
            });
        }
        
        // Simulate any remaining ticks
        for t in next_tick .. 100 {
            sim_events.apply_tick(&self.context, t);
        }
        
        // After simulation
        self.context.after_simulation();
        
        // Apply module stats
        self.context.apply_module_stats();
        
        // Deactivate modules that can no longer be powered
        self.context.deactivate_unpowerable_modules();
        
        // Set all the dead ships to exploding
        for ship in self.context.ships_iter() {
            let mut ship = ship.borrow_mut();
            
            if ship.state.get_hp() == 0 {
                ship.exploding = true;
            }
        }
    }
    
    fn build_plans_packet(&mut self) -> OutPacket {
        let mut packet = OutPacket::new();
        match packet.write(&ServerPacketId::Plan) {
            Ok(()) => {},
            Err(_) => panic!("Failed to write plan packet ID"),
        }

        packet.write(&self.player_ship.borrow().target_sector).ok().expect("Failed to write player's target sector");
        packet.write(&self.player_ship.borrow().get_module_plans()).ok().expect("Failed to write player's plans");

        packet
    }
    
    fn receive_simulation_results(&mut self) {
        let mut packet = self.client.receive();
        self.handle_simulation_results(&mut packet);
    }
    
    fn handle_simulation_results(&mut self, packet: &mut InPacket) {
        match packet.read::<ClientPacketId>() {
            Ok(ref id) if *id != ClientPacketId::SimResults => panic!("Expected SimResults, got something else"),
            Err(e) => panic!("Failed to read simulation results packet ID: {}", e),
            _ => {}, // All good!
        };
        
        // Results packet has both plans and results
        self.context.read_results(packet);
    }
    
    fn try_receive_new_ships(&mut self, gui: &mut SpaceGui) -> io::Result<()> {
        let mut packet = try!(self.client.try_receive());
        
        self.handle_new_ships_packet(gui, &mut packet);
        
        Ok(())
    }
    
    fn receive_new_ships(&mut self, gui: &mut SpaceGui) {
        let mut packet = self.client.receive();
        
        self.handle_new_ships_packet(gui, &mut packet);
    }
    
    fn handle_new_ships_packet(&mut self, gui: &mut SpaceGui, packet: &mut InPacket) {
        let ships_to_add: Vec<ShipRef> = packet.read().ok().expect("Failed to read ships to add from packet");
        let ships_to_remove: Vec<ShipIndex> = packet.read().ok().expect("Failed to read ships to remove from packet");
        
        for ship in ships_to_remove.into_iter() {
            println!("Removing ship {:?}", ship);
        
            gui.remove_lock(ship);
        
            self.context.remove_ship(ship);
        }
    
        for ship in ships_to_add.into_iter() {
            println!("Got a new ship {:?}", ship.borrow().id);
            let ship_id = ship.borrow().id;
            let player_ship_id = self.player_ship.borrow().id;
            if ship_id == player_ship_id {
                let hp = self.player_ship.borrow().state.get_hp();
                if hp == 0 {
                    println!("Replacing player's ship");
                    self.player_ship = ship.clone();
                    self.context.add_ship(ship);
                }
            } else {
                println!("Trying to lock");
                self.context.add_ship(ship.clone());
                gui.try_lock(&ship);
            }
        }
    }
    
    fn handle_tick_packet(&mut self, packet: &mut InPacket) {
        let packet_id =
            match packet.read::<ClientPacketId>() {
                Ok(id) => id,
                Err(e) => panic!("Failed to read tick packet ID: {}", e),
            };
        
        match packet_id {
            ClientPacketId::Tick => { },
            ClientPacketId::LastTick => { self.last_tick = true; },
            _ => { panic!("Expected tick packet, got something else"); },
        }
    }
}
