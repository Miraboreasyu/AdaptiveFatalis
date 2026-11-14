# Host observation prototype (schema 2)

Implementation checkpoint: 1a8e3b3, created before changes. No subagents were used. This prototype is not a published release and has not been installed by the developer.

## Option windows

On the candidate quest-host client, windows retain existing marker/end/part-threshold timing. `host_exchange` records opening world position, horizontal distance and clockwise angle relative to Fatalis, host slot/token, cumulative personal damage delta and opening-monster-max-normalized damage. `player_loss_fraction` remains that local host's HP loss; `monster_loss_fraction` now means host personal damage / opening monster maximum HP, NOT team monster HP loss. `host_received_damage_fraction` is an explicit alias.

Invalid/unavailable host identity, personal statistic, geometry, counter reset or sampling gap marks the window `invalidate=true`. Unknown host output is null, never zero or team damage. Nonhost windows have null host received damage and zero-HP flag rather than mislabeled local values. Existing bundled strategies already skip invalidated windows before accessing damage.

## Four player zero-HP observations

Each occupied roster slot has independent HP continuity. `player_hp_zero` carries player_slot, player_token, is_host, is_local, and timestamp. Failed HP reads, leave/rejoin, changed identity or maximum HP establish a new baseline. Initial/repeated zero is not an edge. Slot numbers are session observations, not stable player accounts. These facts are not assigned to the currently active option automatically.

`quest_deaths` and `part_threshold` remain independent and unchanged. Quest charge counts do not count Insurance-covered carts. Thresholds can occur without closing another window. No rewards or new strategy tuning are introduced.

## Multiplayer validity

Merely being multiplayer no longer invalidates a batch. Unknown party state remains conservative. Host exchange validity is checked per window; raw facts remain independent. This does not prove AI ownership/synchronization or authorize nonhost learning. Run with `none` until source assumptions have been verified live. Old multiplayer records retain their invalidation; missing personal-output observations cannot be reconstructed.

## Source-derived candidates requiring live verification

HunterPie roster global module+0x05013530, offset0x1AB0, read with dereference-then-add semantics; four sequential structures, stride0x58. First field Address is member information (name+0x49), NOT assumed to be a battle entity. The complete 352-byte array is read and decoded with explicit layout tests. HunterPie matches member name to the local save character name: module+0x05013950 -> +0xA8, save index +0x44, header=pointer(first)+index*0x26CC00, name+0x50. We require a unique exact nonempty match; duplicate names or unreadable names produce unknown identity. Raw player names are not logged.

Only the matched local member receives local HP from module+0x050139A0 offsets0x50,0x7630,0. Remote HP is explicitly unavailable: HunterPie's party code does NOT expose a proven remote battle-entity/HP mapping, so the previous guessed member+0x7630 read is removed. The four-slot edge observers remain ready but cannot report remote zeros until that mapping is established. Local position now uses HunterPie's Player::Position offsets0x50,0xC0,0x670 instead of guessed member+0x390.

Historical Party.IsLocalHost uses slot0 as host, still a convention rather than a verified authority API. Party damage global module+0x051C46B8 offsets0x258,0x38,0x450,0x8,0x48; slot stride0x2A0; only slot0 used for host output. All chains mirror HunterPie's dereference-then-add implementation.

The party damage statistic is a candidate: HunterPie has a separate hook fallback when statistics are unavailable. Readable zero does not prove that output is accurately updating. Its victim/quest scope must be checked; this prototype cannot yet promise the statistic is exclusively damage to Fatalis. Do not use for permanent learning before comparing controlled host-only and teammate-only hits. Missing remote HP is reported as unavailable, not interpreted as zero.

## Test

Close the game and preserve register/deployment. Merge the test package and keep strategy none. First verify solo host position/damage; then host a private multiplayer hunt and compare host-only hits, teammate-only hits and individual zero-HP events. Inspect runtime.log for `host observation candidate`, host_exchange in register/events.jsonl, and per-slot raw observations. Join-client behavior should be checked separately: its option windows must not be accepted as host data. Do not force carts for testing.
