//! The predicted player collides with the structures and players of the
//! snapshot (CG_ClipMoveToEntities skips nothing but skipNumber): a
//! structure whose otherEntityNum is 0, like every one the server sends,
//! must not be taken for an entity owned by the mover. Walking through
//! them predicted while the server blocked was the online rubber band.

use trem_sim::gen::constants as k;
use trem_sim::gen::maps::MapId;
use trem_sim::kernel::cgview::apply_entity;
use trem_sim::kernel::msg::EntityState;
use trem_sim::kernel::svgame;
use trem_sim::kernel::types::PlayerState;
use trem_sim::kernel::world::{EType, World};

#[test]
fn predicted_player_does_not_pass_through_structures() {
    // a structure where the arena has one (open space around it)
    let game = World::new(MapId::Arena);
    let b = (0..game.num_entities).find(|&i| game.ents[i].inuse && game.ents[i].etype == EType::Buildable).expect("a structure on the arena");
    let be = &game.ents[b];
    let origin = be.origin;

    let mut view = World::client_view(MapId::Arena);
    for me in [0usize, 1] {
        // the local player's entity as cgview writes it
        let mut ps = PlayerState::default();
        ps.client_num = me as i32;
        ps.origin = origin;
        let mut s = svgame::player_state_to_entity_state(&ps, false);
        s.number = me as i32;
        apply_entity(&mut view, me, &s, &ps.origin, &ps.viewangles);

        // the structure as the server sends it: otherEntityNum left 0
        let s = EntityState {
            number: b as i32,
            e_type: svgame::ET_BUILDABLE,
            modelindex: be.buildable.unwrap().num() as i32,
            solid: svgame::solid_for(be),
            other_entity_num: 0,
            ..Default::default()
        };
        apply_entity(&mut view, b, &s, &origin, &[0.0; 3]);
        // as CG add_packet_entities raises it
        view.num_entities = view.num_entities.max(b + 1);

        // a player box dropping onto the structure from above
        let (mins, maxs) = ([-15.0f32, -15.0, -24.0], [15.0f32, 15.0, 32.0]);
        let start = [origin[0], origin[1], origin[2] + be.maxs[2] + 40.0];
        let end = origin;
        let tr = view.trace(&start, Some(&mins), Some(&maxs), &end, me as i32, k::MASK_PLAYERSOLID);
        assert_eq!(tr.entity_num, b as i32, "client {me}: the trace hits the structure, fraction {}", tr.fraction);
        assert!(tr.fraction < 1.0);
    }
}
