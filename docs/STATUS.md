# Status (what is done)

Updated 5 October 2026. Features in detail: `README.md`; decisions: `sheets/decisions.tsv`; plan coverage: `sheets/02-plan.tsv`, `sheets/03-impl.tsv` (preflight reports 268 plan rows covered, 0 unimplemented).

| Area | State | Since |
|---|---|---|
| Game simulation (`trem_sim`): movement, weapons, structures, classes, stages, votes, admin, bots-as-players, map rotation | done; parity tests against the original | 2026-10-01 |
| Network: server, client, protocol 69 (1.1.0) and 71, downloads, master list, VoIP, ipv6 | done | 2026-10-03 |
| LAN scan on every adapter (dec045); host prints its join address and a firewall hint | done; verified on one PC | 2026-10-05 |
| UPnP port forwarding for hosted games (dec046, `net_upnp`) | done; the owner's router does not answer UPnP | 2026-10-05 |
| Godot client: original menus and HUD, models, animations, effects, sounds, map shaders stage by stage (dec043) | done | 2026-10-05 |
| Enhanced mode (F10, `r_enhanced`): PBR lighting, 4K textures, HD UI, HD sounds, FMOD + Steam Audio, ray-traced AO / GI / reflections, ragdolls, flashlight, world-fixed dynamic shadows (dec044) | done | 2026-10-05 |
| Exported Windows build (`Desktop\Tremulous Release`) with the 4K packs; software ray tracing in the release template | done | 2026-10-05 |
| Weapon looping sounds; loading screen when creating a match | done | 2026-10-05 |
| Model export to OBJ (`tools/md3obj.py`, `Desktop\Tremulous Models`) | done | 2026-10-05 |
| Discord server with moderation bots | done in an earlier session (spec `.agent/discord/spec.json`) | 2026-10-04 |
