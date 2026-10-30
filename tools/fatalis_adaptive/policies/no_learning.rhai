// Reference no-learning policy; this is also the host's built-in fallback.
// Keep collecting raw events, but retain no learned state and return baseline weights.
fn update(state, events, params) { #{} }
fn weights(state, table, params) {
    let result = [];
    for branch in table.branches { result.push(branch.base_weight.to_float()); }
    result
}
