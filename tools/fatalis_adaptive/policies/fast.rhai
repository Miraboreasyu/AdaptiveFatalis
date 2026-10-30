// Faster cumulative learner: lower prior, higher response, retains exploration.
// direction +1 malevolent, -1 benevolent, 0 baseline. State compatible with default.rhai.
fn setting(params, key, fallback) {
    if params.contains(key) { params[key].to_float() } else { fallback }
}
fn update(state, events, params) {
    if !state.contains("effects") { state.effects = #{}; }
    for event in events {
        let p = event.player_loss_fraction.to_float();
        let m = event.monster_loss_fraction.to_float();
        if p < 0.0 || m < 0.0 { continue; }
        let key = event.branch_id;
        let e = if state.effects.contains(key) { state.effects[key] }
            else { #{samples:0.0, player_loss:0.0, monster_loss:0.0} };
        e.samples += 1.0;
        e.player_loss += p.min(1.0);
        e.monster_loss += m.min(1.0);
        state.effects[key] = e;
    }
    state
}
fn weights(state, table, params) {
    let direction = setting(params,"direction",1.0);
    let strength = setting(params,"strength",4.0);
    let prior = setting(params,"prior_samples",1.0);
    let exploration = setting(params,"exploration",0.15);
    if direction.abs() > 1.0 || strength < 0.0 || strength > 10.0
        || prior <= 0.0 || exploration < 0.05 || exploration > 1.0 {
        throw "invalid fast learning parameters";
    }
    let effects = if state.contains("effects") { state.effects } else { #{} };
    let values = []; let total = 0.0;
    for b in table.branches {
        let score = 0.0;
        if effects.contains(b.id) {
            let e = effects[b.id];
            score = (e.player_loss-e.monster_loss)/(e.samples+prior);
        }
        let v = b.base_weight.to_float() * (direction*strength*score).max(-20.0).min(20.0).exp();
        values.push(v); total += v;
    }
    if total <= 0.0 { throw "invalid baseline"; }
    let result = [];
    for i in 0..values.len() {
        result.push(exploration*table.branches[i].base_weight.to_float()
            +(1.0-exploration)*100.0*values[i]/total);
    }
    result
}
