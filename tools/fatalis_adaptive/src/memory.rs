//! Read-only, fallible MHW snapshots. Offsets are source-backed, NOT live-validated.
//! Pointer chains use dereference-then-add semantics (radar memory::resolve_pointer).
use std::collections::HashSet;

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub quest_state: Option<i32>,
    pub quest_id: Option<i32>,
    pub stage: Option<i32>,
    /// (maximum, current), never synthesize zero for a failed read.
    pub player_hp: Option<(f64, f64)>,
    pub monster: Option<MonsterSnapshot>,
}

#[derive(Clone, Debug)]
pub struct MonsterSnapshot {
    pub addr: u64,
    pub hp: (f64, f64),
    pub action_id: i32,
    pub state_ptr: u64,
    pub frame: f64,
    pub ai_distance: f64,
    pub ai_angle: f64,
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetCurrentProcess() -> *mut c_void;
        fn ReadProcessMemory(
            process: *mut c_void,
            address: *const c_void,
            buffer: *mut c_void,
            size: usize,
            done: *mut usize,
        ) -> i32;
    }
    pub fn base() -> Option<u64> {
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as u64;
        (base != 0).then_some(base)
    }
    pub fn bytes(address: u64, buffer: &mut [u8]) -> Option<()> {
        if address < 0x10000 || address.checked_add(buffer.len() as u64)? > 0x0000_8000_0000_0000 {
            return None;
        }
        let mut done = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                address as *const c_void,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut done,
            )
        };
        (ok != 0 && done == buffer.len()).then_some(())
    }
}
#[cfg(not(windows))]
mod platform {
    pub fn base() -> Option<u64> {
        None
    }
    pub fn bytes(_: u64, _: &mut [u8]) -> Option<()> {
        None
    }
}

fn bytes<const N: usize>(address: u64) -> Option<[u8; N]> {
    let mut result = [0; N];
    platform::bytes(address, &mut result)?;
    Some(result)
}
/// Pack five decimal digits as table_code * 100 + option.
/// Every digit must be 0..=9; a partial/foreign write is rejected.
pub fn pack_decimal_digits(digits: [u8; 5]) -> Option<i32> {
    if digits.iter().any(|digit| *digit > 9) {
        return None;
    }
    let table_code = u16::from(digits[0]) * 100 + u16::from(digits[1]) * 10 + u16::from(digits[2]);
    let option = u16::from(digits[3]) * 10 + u16::from(digits[4]);
    Some(i32::from(table_code) * 100 + i32::from(option))
}

/// Read five code digits followed by the counter. The counter is read before
/// and after the digits; it must be unchanged. The wrapper writes digits first
/// and the counter last, so a stable counter implies the digits belong to a
/// completed publication. The multi-register write is still not atomic.
pub fn read_register_code(code_addresses: &[u64; 5], counter_address: u64) -> Option<(u32, i32)> {
    if code_addresses.contains(&counter_address) {
        return None;
    }
    let before = u32::from_le_bytes(bytes(counter_address)?);
    let mut digits = [0u8; 5];
    for (digit, address) in digits.iter_mut().zip(code_addresses) {
        let value = int(*address)?;
        *digit = u8::try_from(value).ok().filter(|value| *value <= 9)?;
    }
    let after = u32::from_le_bytes(bytes(counter_address)?);
    (before == after).then_some((after, pack_decimal_digits(digits)?))
}

/// Source-derived context chain, reconstructed from the rebuilt executable
/// (see REGISTER_INSTRUMENTATION.md). These are candidate offsets for the
/// current build, not a live-validated guarantee.
pub const MONSTER_AIDATA_OFFSET: u64 = 0x12278;
pub const AIDATA_THINK_EM_OFFSET: u64 = 0xAA8;
pub const THINK_EM_AIDATA_OFFSET: u64 = 0x630;
pub const THINK_EM_MONSTER_OFFSET: u64 = 0x138;
pub const THINK_EM_REGISTER_BASE: u64 = 0x640;

/// Pure address arithmetic; no memory access.
pub fn think_register_address(think_em: u64, slot: u8) -> Option<u64> {
    if slot > 21 {
        return None;
    }
    think_em.checked_add(THINK_EM_REGISTER_BASE + u64::from(slot) * 4)
}

/// Re-resolve `monster+0x12278 -> AIData`, `AIData+0xAA8 -> cThinkEm` and
/// validate the constructor back-reference `cThinkEm+0x630 == AIData`.
/// Returns `None` on any unreadable pointer or failed identity check.
pub fn resolve_think_em(monster: u64) -> Option<u64> {
    derive_think_em(monster, ptr)
}
fn derive_think_em(monster: u64, mut read_pointer: impl FnMut(u64) -> Option<u64>) -> Option<u64> {
    let aidata = read_pointer(monster.checked_add(MONSTER_AIDATA_OFFSET)?)?;
    let think_em = read_pointer(aidata.checked_add(AIDATA_THINK_EM_OFFSET)?)?;
    if read_pointer(think_em.checked_add(THINK_EM_AIDATA_OFFSET)?)? != aidata {
        return None;
    }
    Some(think_em)
}

/// Resolve six physical register addresses for the current monster: five code
/// digits then the counter. The parent check is strict; a failure means the
/// derived context is not trusted for this poll.
pub fn resolve_register_addresses(monster: u64, slots: &[u8]) -> Option<Vec<u64>> {
    if slots.len() != 6
        || slots.iter().any(|slot| *slot > 21)
        || slots
            .iter()
            .enumerate()
            .any(|(index, slot)| slots[..index].contains(slot))
    {
        return None;
    }
    let think_em = resolve_think_em(monster)?;
    slots
        .iter()
        .map(|slot| think_register_address(think_em, *slot))
        .collect()
}

/// Read one derived register slot after independently resolving the context.
pub fn read_think_register(monster: u64, slot: u8) -> Option<i32> {
    int(think_register_address(resolve_think_em(monster)?, slot)?)
}
fn ptr(address: u64) -> Option<u64> {
    let p = u64::from_le_bytes(bytes(address)?);
    (0x10000..0x0000_8000_0000_0000).contains(&p).then_some(p)
}
fn int(address: u64) -> Option<i32> {
    Some(i32::from_le_bytes(bytes(address)?))
}
fn float(address: u64) -> Option<f64> {
    let n = f32::from_le_bytes(bytes(address)?) as f64;
    n.is_finite().then_some(n)
}
fn chain(base: u64, offsets: &[u64]) -> Option<u64> {
    offsets
        .iter()
        .try_fold(base, |a, offset| ptr(a)?.checked_add(*offset))
}
/// Resolve an explicitly configured module-relative pointer chain.
/// offsets[0] is the first global address relative to the module base; every
/// subsequent offset is added after dereferencing. No offsets are invented here.
pub fn resolve_module_chain(offsets: &[u64]) -> Option<u64> {
    if offsets.is_empty() {
        return None;
    }
    chain(platform::base()?, offsets)
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct PartyPlayer {
    pub slot: usize,
    pub token: u64,
    pub is_host: bool,
    pub is_local: bool,
    pub hp: Option<(f64, f64)>,
    pub entity_token: Option<u64>,
}
#[derive(Clone, Debug, Default)]
pub struct PartyObservation {
    pub players: Vec<PartyPlayer>,
    pub host: Option<crate::host_observation::HostSample>,
    pub diagnostics: String,
}
fn position(at: u64) -> Option<[f64; 3]> {
    Some([float(at)?, float(at + 4)?, float(at + 8)?])
}
fn name_bytes(at: u64) -> Option<Vec<u8>> {
    let raw = bytes::<32>(at)?;
    let end = raw.iter().position(|b| *b == 0)?;
    if end == 0 {
        return None;
    }
    let text = std::str::from_utf8(&raw[..end]).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    Some(raw[..end].to_vec())
}
/// Exact HunterPie save-slot name lookup; used only for unique roster matching.
fn local_character_name(base: u64) -> Option<Vec<u8>> {
    let first = chain(base.checked_add(0x05013950)?, &[0xa8])?;
    let slot = u32::from_le_bytes(bytes(first.checked_add(0x44)?)?);
    if slot > 2 {
        return None;
    }
    let header = ptr(first)?.checked_add(u64::from(slot) * 0x26cc00)?;
    name_bytes(header.checked_add(0x50)?)
}
/// HunterPie roster Address is member information, NOT a proven battle entity.
/// Remote HP is unavailable until a source-backed entity mapping is established.
pub fn read_party_observation(monster: Option<u64>) -> PartyObservation {
    let mut out = PartyObservation::default();
    let Some(base) = platform::base() else {
        return out;
    };
    let local = chain(base + 0x0501_39a0, &[0x50, 0]);
    let local_hp = chain(base + 0x050139a0, &[0x50, 0x7630, 0]).and_then(hp);
    let party = read_multiplayer();
    let roster = chain(base + 0x0501_3530, &[0x1ab0]);
    let damage = chain(base + 0x051c_46b8, &[0x258, 0x38, 0x450, 0x8, 0x48]);
    let name = local_character_name(base);
    let raw = roster.and_then(bytes::<352>);
    let entries = raw.as_ref().and_then(|b| crate::party_roster::addresses(b));
    let mut names: [Option<Vec<u8>>; 4] = Default::default();
    if let Some(entries) = entries {
        for (i, entry) in entries.iter().enumerate() {
            if *entry >= 0x10000 {
                names[i] = name_bytes(entry + 0x49);
            }
        }
    }
    let local_slot = if party == Some(false) {
        local.map(|_| 0)
    } else {
        crate::party_roster::local_slot(&names, name.as_deref())
    };
    out.diagnostics=format!("party={party:?} roster={roster:?} entries={entries:?} local_entity={local:?} local_name_available={} occupied={:?} local_slot={local_slot:?} local_hp_available={} damage_ptr={damage:?}; remote HP unavailable (no verified roster-to-entity mapping)",name.is_some(),names.iter().map(Option::is_some).collect::<Vec<_>>(),local_hp.is_some());
    if party == Some(false) {
        if let Some(token) = local {
            out.players.push(PartyPlayer {
                slot: 0,
                token,
                is_host: true,
                is_local: true,
                hp: local_hp,
                entity_token: local,
            });
        }
    } else if party == Some(true) {
        if let Some(entries) = entries {
            for slot in 0..4 {
                if names[slot].is_none() {
                    continue;
                }
                let is_local = local_slot == Some(slot);
                out.players.push(PartyPlayer {
                    slot,
                    token: entries[slot],
                    is_host: slot == 0,
                    is_local,
                    hp: if is_local { local_hp } else { None },
                    entity_token: if is_local { local } else { None },
                });
            }
        }
    }
    if let Some(host) = out.players.iter().find(|p| p.is_host) {
        // HunterPie Player::Position: module global -> +50 -> +C0 -> +670.
        let pos = if host.is_local {
            chain(base + 0x050139a0, &[0x50, 0xc0, 0x670]).and_then(position)
        } else {
            None
        };
        let geometry = monster.and_then(|m| {
            let p = pos?;
            let q = position(m + 0x160)?;
            let quat = [
                float(m + 0x170)?,
                float(m + 0x174)?,
                float(m + 0x178)?,
                float(m + 0x17c)?,
            ];
            if (quat.iter().map(|x| x * x).sum::<f64>() - 1.0).abs() > 0.1 {
                return None;
            }
            let dx = p[0] - q[0];
            let dz = p[2] - q[2];
            let yaw = (2.0 * (quat[3] * quat[1] + quat[0] * quat[2]))
                .atan2(1.0 - 2.0 * (quat[0] * quat[0] + quat[1] * quat[1]));
            Some((
                (dx * dx + dz * dz).sqrt(),
                (dx.atan2(dz) - yaw).to_degrees().rem_euclid(360.0),
            ))
        });
        out.host = Some(crate::host_observation::HostSample {
            player_token: host.token,
            damage_token: damage.unwrap_or(0),
            local_is_host: local_slot.map(|slot| slot == 0),
            slot: Some(0),
            damage_total: damage.and_then(int).filter(|d| *d >= 0).map(f64::from),
            position: pos,
            distance: geometry.map(|g| g.0),
            angle: geometry.map(|g| g.1),
        });
    }
    out
}

/// Source-derived task party flag from radar/HunterPie. Zero means solo;
/// positive values indicate multiplayer. Unknown is never interpreted as solo.
pub fn read_auxiliary(monster: Option<u64>) -> Option<crate::observations::AuxiliarySnapshot> {
    let base = platform::base()?;
    let quest = ptr(base + 0x0500_ed30)?;
    let deaths = int(quest.checked_add(0x17374)?).filter(|n| (0..=100).contains(n));
    let max_deaths = int(quest.checked_add(0x17370)?).filter(|n| (1..=100).contains(n));
    let part_table_token = monster.and_then(|m| ptr(m.checked_add(0x1d058)?));
    let part_counts = part_table_token.and_then(|parts| {
        let mut counts = [0; 11];
        for (i, count) in counts.iter_mut().enumerate() {
            let at = parts.checked_add(0x40 + i as u64 * 0x1f8)?;
            let max = float(at + 0xc)?;
            let health = float(at + 0x10)?;
            *count = int(at + 0x18)?;
            if !(0.0..=1e9).contains(&max)
                || !(-1e6..=1e9).contains(&health)
                || !(0..=10000).contains(count)
            {
                return None;
            }
        }
        Some(counts)
    });
    Some(crate::observations::AuxiliarySnapshot {
        quest_token: quest,
        deaths,
        max_deaths,
        monster_token: monster,
        part_table_token,
        part_counts,
    })
}

pub fn read_multiplayer() -> Option<bool> {
    let base = platform::base()?;
    let value = chain(base + 0x051c_46b8, &[0x258, 0x10, 0x6574]).and_then(int)?;
    match value {
        0 => Some(false),
        1..=4 => Some(true),
        _ => None,
    }
}

fn hp(object: u64) -> Option<(f64, f64)> {
    let max = float(object.checked_add(0x60)?)?;
    let current = float(object.checked_add(0x64)?)?;
    (max > 0.0 && current >= 0.0 && current <= max).then_some((max, current))
}

/// Re-resolves every chain on each call; a read failure is unknown, not zero.
/// Reads are not atomic: consumers must retain read-window timestamps and invalidate
/// intervals on quest/instance changes. No direct game pointer dereferences occur.
pub fn read_snapshot() -> Snapshot {
    let Some(base) = platform::base() else {
        return Snapshot::default();
    };
    let quest = chain(base + 0x0500_ED30, &[0]);
    let quest_state = quest.and_then(|q| int(q + 0x54));
    let quest_id = quest.and_then(|q| int(q + 0x4C));
    let stage = chain(base + 0x0500_ECA0, &[0xAED0]).and_then(int);
    let mut snapshot = Snapshot {
        quest_state,
        quest_id,
        stage,
        player_hp: None,
        monster: None,
    };
    // Do not consume title/loading leftovers or global AI data outside Schrade.
    if quest_state != Some(2) || !quest_id.is_some_and(|id| id > 0) || stage != Some(417) {
        return snapshot;
    }
    snapshot.player_hp = chain(base + 0x0501_39A0, &[0x50, 0x7630, 0]).and_then(hp);
    snapshot.monster = read_monster(base);
    // A transition during the read invalidates combat fields rather than blending tasks.
    if quest.and_then(|q| int(q + 0x54)) != quest_state
        || quest.and_then(|q| int(q + 0x4C)) != quest_id
        || chain(base + 0x0500_ECA0, &[0xAED0]).and_then(int) != stage
    {
        snapshot.player_hp = None;
        snapshot.monster = None;
    }
    snapshot
}

fn read_monster(base: u64) -> Option<MonsterSnapshot> {
    let array = chain(base + 0x0500_CF40, &[0x38])?;
    let mut components = [0u8; 128 * 8];
    platform::bytes(array, &mut components)?;
    let mut found = HashSet::new();
    for chunk in components.as_chunks::<8>().0 {
        let component = u64::from_le_bytes(*chunk);
        if component < 0x10000 {
            continue;
        }
        let Some(monster) = component.checked_add(0x138).and_then(ptr) else {
            continue;
        };
        let Some(text) = ptr(monster + 0x2A0) else {
            continue;
        };
        let Some(path) = bytes::<64>(text + 0x0C) else {
            continue;
        };
        if path.starts_with(b"em\\em013\\") && int(monster + 0x12280) == Some(101) {
            found.insert(monster);
        }
    }
    // Global AI pointer cannot be assigned unambiguously to multiple Fatalis.
    if found.len() != 1 {
        return None;
    }
    let monster = *found.iter().next()?;
    let health = hp(ptr(monster + 0x7670)?)?;
    // Actual radar code reads the action ID inline, NOT through another pointer.
    let action_id = int(monster + 0x6278)?;
    if !(0..=10000).contains(&action_id) {
        return None;
    }
    let state_ptr = ptr(monster + 0x468)?;
    let frame = float(state_ptr + 0x10C)?;
    if frame < 0.0 {
        return None;
    }
    let ai = chain(base + 0x0501_3C50, &[0xE58, 0x1008, 0])?;
    let ai_distance = float(ai + 0x5FC)?;
    let ai_angle = float(ai + 0x604)?;
    if ai_distance < 0.0 {
        return None;
    }
    // Detect a state-pointer swap across the multi-read sampling window.
    if ptr(monster + 0x468) != Some(state_ptr) {
        return None;
    }
    Some(MonsterSnapshot {
        addr: monster,
        hp: health,
        action_id,
        state_ptr,
        frame,
        ai_distance,
        ai_angle,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_addresses_fail_closed() {
        assert!(ptr(0).is_none());
        assert!(int(0).is_none());
        assert!(float(0).is_none());
        assert!(chain(0, &[0]).is_none());
    }
    #[test]
    fn derived_register_offsets_are_fixed_and_bounded() {
        let em = 0x1_0000_0000u64;
        assert_eq!(think_register_address(em, 0), Some(em + 0x640));
        assert_eq!(think_register_address(em, 14), Some(em + 0x678));
        assert_eq!(think_register_address(em, 15), Some(em + 0x67C));
        assert_eq!(think_register_address(em, 21), Some(em + 0x694));
        assert!(think_register_address(em, 22).is_none());
        let slots = [14, 15, 16, 17, 18, 19];
        assert!(resolve_register_addresses(0, &slots).is_none());
        let duplicate = [14, 14, 16, 17, 18, 19];
        assert!(resolve_register_addresses(em, &duplicate).is_none());
        assert!(resolve_register_addresses(em, &slots[..5]).is_none());
    }
    #[test]
    fn decimal_code_packing_and_validation() {
        assert_eq!(pack_decimal_digits([0, 0, 0, 0, 0]), Some(0));
        assert_eq!(pack_decimal_digits([5, 6, 8, 1, 2]), Some(56_812));
        assert_eq!(pack_decimal_digits([9, 9, 9, 9, 9]), Some(99_999));
        assert_eq!(pack_decimal_digits([0, 0, 1, 0, 10]), None);
        assert_eq!(pack_decimal_digits([10, 0, 0, 0, 0]), None);
    }
    #[test]
    fn derived_chain_requires_the_constructor_back_reference() {
        let monster = 0x1_0000u64;
        let aidata = 0x2_0000u64;
        let think_em = 0x3_0000u64;
        let mut pointers = std::collections::BTreeMap::new();
        pointers.insert(monster + MONSTER_AIDATA_OFFSET, aidata);
        pointers.insert(aidata + AIDATA_THINK_EM_OFFSET, think_em);
        pointers.insert(think_em + THINK_EM_AIDATA_OFFSET, aidata);
        let read = |a: u64| pointers.get(&a).copied();
        assert_eq!(derive_think_em(monster, read), Some(think_em));
        let mut wrong = pointers.clone();
        wrong.insert(think_em + THINK_EM_AIDATA_OFFSET, monster);
        assert_eq!(
            derive_think_em(monster, |a| wrong.get(&a).copied()),
            None,
            "a mismatched AIData back-reference must reject the context"
        );
    }
    #[cfg(not(windows))]
    #[test]
    fn unsupported_platform_is_unknown() {
        let s = read_snapshot();
        assert!(s.quest_state.is_none() && s.monster.is_none());
    }
}
