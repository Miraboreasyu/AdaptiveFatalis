# Observation kernel v1

This document defines the finalized first observation contract. Facts are independent of reward rules. No built-in kill bonus, faint inference, or strategy tuning is added.

## Strategy input

The existing `update(state, events, params)` API is unchanged. New records have `observation_schema: 1` and a `kind`:

| kind | meaning | fields |
|---|---|---|
| `option_window` | A reliably tracked option window | existing branch/table/exit and damage fields; `started_at`, `ended_at`, `duration_ms`, `end_reason`, `window_truncated`, `part_threshold`, `player_hp_reached_zero` |
| `player_hp_zero` | A valid local HP snapshot crossed from positive to zero | `timestamp_ms`, `quest_id`, `player_hp_reached_zero: true`; unassigned |
| `quest_deaths` | Quest charged-death counter increased | `timestamp_ms`, `quest_id`, `quest_deaths_delta`, raw `snapshot`; unassigned |
| `part_threshold` | A normal part counter increased | `timestamp_ms`, `quest_id`, `part_slot`, `delta`; unassigned |

Raw zero-HP and attributed window flags may describe the SAME observation: they must not be added as separate kills. Task charged deaths may arrive later and describe that same cart again. The host intentionally does not merge them or claim a killing move. `player_fainted` remains null; new scripts should use explicit observations instead.

`player_hp_reached_zero` on a window means a coherent HP edge was observed inside that tracked window. False means no edge was observed, not proof the player survived. A first zero sample, repeated zero, missing read, changed max HP or session discontinuity does not invent an edge. The independent HP observer still supplies an unassigned raw event if no option is trackable.

`part_threshold` on an option window means this counter observation CAUSED that window to close. A raw threshold event is emitted even when there is no active window, including another threshold reached during a knockdown. It is not a claim that every threshold interrupts an animation.

## Window time and boundaries

All timestamps are monotonic milliseconds since the current worker started; they are not comparable across process runs. Within each settlement batch the strategy receives window completions and raw observations stably sorted by completion/observation time. Equal timestamps keep window records before raw facts. Batches remain in journal order; records are not globally resorted across batches or sessions.

Normal duration runs from observed marker to next observed marker. A positive part threshold closes at the coherent HP sample in that poll. Damage AND duration stop there; neither extends through a knockdown or phase transition. Until the next marker, no window reopens for the old counter. If a marker changes in the same poll, normal marker ordering wins: do not close the newly opened window.

`duration_ms = ended_at - started_at`; this is observed decision-window time, not exact animation duration. `window_truncated` is true for any reason other than `next_marker`. Reasons include `part_threshold`, `maximum_hp_changed`, `monster_token_changed`, `counter_reset`, and `observation_ended`. Reliable-prefix boundaries end at the LAST coherent snapshot, excluding changed/respawn HP. Incoherent reads and missed-marker gaps still discard ambiguous windows.

## Persistence and compatibility

`register/events.jsonl` is authoritative. Each batch retains its existing `events` array and adds a default-empty `observations` array. Both are presented through the same strategy event list. The multiplayer/unknown-party invalidation flag is propagated to BOTH types before script execution. New profiles automatically replay this history; existing profiles read only their unprocessed suffix.

Old batches without `observations` work unchanged. Old option windows lacking new fields must be treated as having UNKNOWN duration/boundary/zero-HP observations, not synthesized values. The bundled learners skip explicitly non-window kinds and continue accepting legacy window records without `kind`; their damage/reward mathematics and state versions are unchanged. Third-party scripts must filter event kinds before accessing branch/damage fields.

`register/observations.jsonl` remains a human-readable diagnostic mirror, not a second strategy input or a second replay source. Do not load both journals into the same learning pass.

## Live evidence and limits

The prototype hunt observed four head counter increases and three active-window closures. The user confirmed the fourth threshold was reached while already down, and the long gap after the third was a phase transition. Truncated durations were 1435ms, 620ms and 407ms. These observations agree with the chosen boundary rule; they do not prove animation-level interruption recognition universally.

Two user-confirmed carts produced one charged-death increase. The first was covered by Insurance, explaining the lack of task deduction. The counter is therefore NOT all carts. Independent HP-to-zero facts complement it; the final independent-zero event delivery was tested offline and still needs a live confirmation run.

Normal slots 0..10 are head, neck, chest, body, arms, legs, wings and tail. The separate severable Knockdown pseudo-part is not read. Pointer replacement, missing reads and counter decreases establish new baselines. Source addresses: quest global module+0x0500ED30; max/deaths quest+0x17370/+0x17374; parts pointer monster+0x1D058; array+0x40, stride0x1F8, maxHP/currentHP/counter+0xC/+0x10/+0x18. Evidence originated from HunterPie build421810 and was checked in the prototype hunt; other game builds are not guaranteed.

LuaEngine/auto-cart research did not yield a verified cart/stagger callback implementation. A failed read must never be promoted to zero HP.

## Use

Close the game, merge the package, and retain register/deployment. The package selects `none`. Strategies and parameters remain external and optional. No GitHub release, remote push or game installation is implied by finalizing this observation contract.
