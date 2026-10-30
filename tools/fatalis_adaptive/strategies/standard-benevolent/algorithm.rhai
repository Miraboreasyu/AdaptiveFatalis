// Pure learning policy. The host owns persistence, validation and integer rounding.
fn number_or(params, key, fallback) {
    if params.contains(key) { params[key].to_float() } else { fallback }
}

fn update(state, events, params) {
    if !state.contains("effects") { state.effects = #{}; }
    for event in events {
        let player = event.player_loss_fraction.to_float();
        let monster = event.monster_loss_fraction.to_float();
        // Match Learning::observe: reject negative damage, cap each observation at one.
        if player < 0.0 || monster < 0.0 { continue; }
        let key = event.branch_id;
        let effect = if state.effects.contains(key) {
            state.effects[key]
        } else { #{samples: 0.0, player_loss: 0.0, monster_loss: 0.0} };
        effect.samples += 1.0;
        effect.player_loss += player.min(1.0);
        effect.monster_loss += monster.min(1.0);
        state.effects[key] = effect;
    }
    state
}

fn weights(state, table, params) {
    let direction = number_or(params, "direction", 1.0);
    let strength = number_or(params, "strength", 2.0);
    let prior = number_or(params, "prior_samples", 8.0);
    let exploration = number_or(params, "exploration", 0.25);
    if direction.abs() > 1.0 || strength < 0.0 || strength > 10.0
        || prior <= 0.0 || exploration < 0.05 || exploration > 1.0 {
        throw "invalid learning policy";
    }
    let effects = if state.contains("effects") { state.effects } else { #{} };
    let scores = [];
    let total = 0.0;
    let baseline_total = 0.0;
    for branch in table.branches {
        let base = branch.base_weight.to_float();
        if base < 0.0 { throw "invalid baseline distribution"; }
        baseline_total += base;
        let score = 0.0;
        if effects.contains(branch.id) {
            let effect = effects[branch.id];
            score = (effect.player_loss - effect.monster_loss) / (effect.samples + prior);
        }
        let value = base * (direction * strength * score).max(-20.0).min(20.0).exp();
        scores.push(value);
        total += value;
    }
    if baseline_total != 100.0 || total <= 0.0 { throw "invalid baseline distribution"; }
    let result = [];
    for i in 0..scores.len() {
        result.push(exploration * table.branches[i].base_weight.to_float()
            + (1.0 - exploration) * 100.0 * scores[i] / total);
    }
    result
}
