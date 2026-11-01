// Test policy: last option causing > 0.9 maximum HP loss in ONE window wins.
// No cross-window damage accumulation, no exploration, no actual faint detection.
fn update(state, events, params) {
    if !state.contains("winners") { state.winners = #{}; }
    for event in events {
        if event.contains("kind") && event.kind != "option_window" { continue; }
        if event.contains("invalidate") && event.invalidate { continue; }
        if event.player_loss_fraction.to_float() > 0.9 {
            state.winners[event.table_id] = event.branch_id;
        }
    }
    state
}
fn weights(state, table, params) {
    let winner = if state.contains("winners") && state.winners.contains(table.id) {
        state.winners[table.id]
    } else { "" };
    let found = false;
    for branch in table.branches {
        if branch.id == winner && branch.base_weight > 0 { found = true; }
    }
    let result = [];
    for branch in table.branches {
        if found {
            result.push(if branch.id == winner { 100.0 } else { 0.0 });
        } else {
            result.push(branch.base_weight.to_float());
        }
    }
    result
}
