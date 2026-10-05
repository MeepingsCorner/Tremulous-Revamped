# Roadmap (what remains)

Updated 5 October 2026. Ordered by the owner's priority as far as known; items marked *offered* were proposed and not yet requested.

1. **LAN / online play with the owner's friend.** It fails for him; it works on one PC. Diagnose on both machines (handover §3): network, firewall prompt for `Tremulous.exe`, direct `connect <IP>`; for internet play, UPnP or manual forwarding of UDP 30720.
2. **Hardware ray tracing in the exported game.** The Godot release template rejects the BLAS; the game uses the software tracer. Try a self-built release template or the debug template.
3. **Release size.** The 4K pack makes the release folder 10.2 GB; offer a 2K variant or per-map packs.
4. **Installer / Steam.** An installer that adds the firewall rule; the copyright audit for Steam was answered in chat on 5 October (CC BY-SA 2.5 media, GPL code, proprietary FMOD / Steam Audio need licences); *offered:* a per-map texture provenance list.
5. **Shadows.** World-fixed shadow directions cover 70–98 % of the light; the owner originally asked for ray-traced shadows (he chose the hybrid). Possible next step: ray-traced shadows of models in the ray-tracing modes.
6. *Offered:* GPP (protocol 70) server support; patch stitching (cracks between curved surfaces).
7. Bots (`sheets/work.tsv` t0008, still open).
8. Housekeeping: backfill `sheets/work.tsv` and `sheets/tokens.tsv`; the unused-variable warning in `game.rs` (`let w` near the ragdoll code).
