# Multiplayer batch exclusion (test build)

The current AI is not disabled or replaced when another player joins. Only learning data is filtered. Batches already committed before detection are not rolled back.

The worker checks a source-derived party flag during Schrade combat updates. Any observed multiplayer state latches `multiplayer_detected`; an unreadable/unsupported flag latches `party_state_unknown` conservatively. A pending batch and all later batches inherit `invalidate: true` and an `invalidation_reason`. A teammate leaving does not clear the latch. Confirmed quest-end states reset it after settlement. After 60 seconds without valid combat updates, pending data is settled with the old flag before reset; the next combat sample must check party state again. An unchanged move counter or unchanged HP does not count as idle.

The journal retains invalidated batches. During replay the batch flag/reason are propagated to each event in the existing `update(state, events, params)` API. Bundled learning scripts skip invalidated events before inspecting their damage fields. Third-party scripts must implement the same check if they wish to exclude multiplayer data. Old journal lines with no flag remain valid and cannot retroactively be classified as single-player or multiplayer.

No strategy IDs or state versions are changed, so existing profiles continue processing their unconsumed journal suffix. `hunts` remains the historical count of processed batches, including excluded batches, not a reliable quest counter. Exclusion does not delete existing learned effects or prevent weight changes due to previously unprocessed valid history.

Party flag source: local radar/HunterPie-derived module pointer chain `0x051C46B8 -> +0x258 -> +0x10 -> +0x6574`. Zero is treated as solo, values 1..4 as multiplayer, all other values/read failures as unknown. This is NOT live-validated for the installed build yet; the task scope, join/leave transitions and relationship to gathering-hub membership require actual tests. The mod logs party state changes, latch reasons, invalidated batch sizes, and idle reset.

Offline validation: guard latch/reset/unknown handling, legacy batch defaults, batch metadata roundtrip, and standard/fast/super script skip/backward-compatibility tests pass. Release build and clippy pass. No game files were modified, and this version is not published as a release.
