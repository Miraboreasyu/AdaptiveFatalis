use crate::learning::Policy;
use serde::{Deserialize, Serialize};

/// Explicit, physically addressed register instrumentation. Supplying this
/// section selects the exact register-event pipeline instead of approximate
/// action attribution.
///
/// Six int32 registers carry a decimal event code: three table-code digits, two
/// option/exit digits, and a counter. With `addresses` omitted, each poll derives
/// the register context from the already-identified monster (monster+0x12278 ->
/// AIData, AIData+0xAA8 -> cThinkEm, cThinkEm+0x640+4*slot). An explicit array
/// must contain all six addresses in the same order; partial overrides are never
/// guessed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterMode {
    /// Decimal table-code digits: hundreds, tens, units.
    pub table_code_slots: [u8; 3],
    /// Decimal option/exit digits: tens, units.
    pub option_slots: [u8; 2],
    pub counter_slot: u8,
    /// Explicit innermost leaf tables. Empty means every discovered instrumentable table.
    #[serde(default)]
    pub target_tables: Vec<String>,
    /// Five code-digit addresses followed by the counter address.
    #[serde(default)]
    pub addresses: Option<[RegisterAddress; 6]>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RegisterAddress {
    /// Fixed address (survives only while the allocation stays there).
    Absolute { address: u64 },
    /// Module-relative pointer chain with the same semantics as memory::chain:
    /// offsets[0] is added to the module base and dereferenced, then each later
    /// offset is added and dereferenced. Resolved again on every poll.
    ModuleChain { offsets: Vec<u64> },
}
impl RegisterAddress {
    pub fn resolve(&self) -> Option<u64> {
        match self {
            Self::Absolute { address } => (*address >= 0x10000).then_some(*address),
            Self::ModuleChain { offsets } => crate::memory::resolve_module_chain(offsets),
        }
    }
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Absolute { address } if *address < 0x10000 => {
                Err("absolute register address is not in the user address range".into())
            }
            Self::ModuleChain { offsets } if offsets.is_empty() => {
                Err("module-chain address needs at least one offset".into())
            }
            _ => Ok(()),
        }
    }
}
impl RegisterMode {
    pub fn slots(&self) -> [u8; 6] {
        [
            self.table_code_slots[0],
            self.table_code_slots[1],
            self.table_code_slots[2],
            self.option_slots[0],
            self.option_slots[1],
            self.counter_slot,
        ]
    }
    pub fn validate(&self) -> Result<(), String> {
        let slots = self.slots();
        if slots.iter().any(|slot| *slot > 21) {
            return Err("register slots must be physical slots in 0..21".into());
        }
        if slots
            .iter()
            .enumerate()
            .any(|(index, slot)| slots[..index].contains(slot))
        {
            return Err("register slots must be six distinct physical slots".into());
        }
        if self
            .target_tables
            .iter()
            .any(|table| table.trim().is_empty())
        {
            return Err("register target_tables entries must be nonempty".into());
        }
        if let Some(addresses) = &self.addresses {
            for address in addresses {
                address.validate()?;
            }
        }
        Ok(())
    }
    /// `None` means derive all six addresses from the current monster each poll.
    pub fn explicit_addresses(&self) -> Option<&[RegisterAddress; 6]> {
        self.addresses.as_ref()
    }
    pub fn address_mode(&self) -> &'static str {
        if self.addresses.is_some() {
            "explicit"
        } else {
            "derived from monster"
        }
    }
    pub fn registers(&self) -> crate::instrument::Registers {
        crate::instrument::Registers {
            table_code: self.table_code_slots,
            option: self.option_slots,
            counter: self.counter_slot,
        }
    }
    /// Sorted and deduplicated. Empty means all; the caller discovers them.
    pub fn targets(&self) -> Vec<String> {
        let mut tables = self.target_tables.clone();
        tables.sort();
        tables.dedup();
        tables
    }
    pub fn targets_all(&self) -> bool {
        self.target_tables.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptLearning {
    pub script: String,
    pub profile: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub import_legacy: bool,
    #[serde(default)]
    pub debug: bool,
}
impl ScriptLearning {
    pub fn validate(&self) -> Result<(), String> {
        if self.script.trim().is_empty()
            || self.profile.is_empty()
            || !self
                .profile
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err(
                "scripted_learning requires script and a simple alphanumeric profile name".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub policy: Policy,
    pub poll_ms: u64,
    pub stable_ms: u64,
    /// Explicit opt-in to approximate candidate sharing when hidden AI state is unavailable.
    pub share_ambiguous: bool,
    /// Safety gate for source-derived offsets; confirm after inspecting diagnostic logs.
    pub allow_unverified_offsets: bool,
    /// Present = exact register events; absent = legacy approximate action windows.
    pub register: Option<RegisterMode>,
    /// Optional external strategy, evaluated only at startup and hunt settlement.
    pub scripted_learning: Option<ScriptLearning>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            policy: Policy::default(),
            poll_ms: 16,
            stable_ms: 1500,
            share_ambiguous: true,
            allow_unverified_offsets: false,
            register: None,
            scripted_learning: None,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        self.policy.validate()?;
        if let Some(scripted) = &self.scripted_learning {
            scripted.validate()?;
            if self.register.is_none() {
                return Err("scripted_learning requires register mode".into());
            }
        }
        if !(4..=1000).contains(&self.poll_ms) || !(500..=10000).contains(&self.stable_ms) {
            return Err("poll_ms must be 4..=1000; stable_ms must be 500..=10000".into());
        }
        if let Some(register) = &self.register {
            register.validate()?;
            if !self.allow_unverified_offsets {
                return Err(
                    "register mode requires allow_unverified_offsets=true after address verification"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(value: u64) -> RegisterAddress {
        RegisterAddress::Absolute { address: value }
    }
    fn mode(tables: &[&str]) -> RegisterMode {
        RegisterMode {
            table_code_slots: [14, 15, 16],
            option_slots: [17, 18],
            counter_slot: 19,
            target_tables: tables.iter().map(|s| s.to_string()).collect(),
            addresses: Some([
                address(0x1234_0000),
                address(0x1234_0004),
                address(0x1234_0008),
                address(0x1234_000c),
                address(0x1234_0010),
                address(0x1234_0014),
            ]),
        }
    }

    #[test]
    fn defaults_keep_register_mode_off() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(config.register.is_none());
        config.validate().unwrap();
    }

    #[test]
    fn register_mode_is_opt_in_and_addresses_are_validated() {
        let mut config = Config {
            register: Some(mode(&["main:n105:s1"])),
            ..Config::default()
        };
        assert!(config.validate().is_err());
        config.allow_unverified_offsets = true;
        config.validate().unwrap();
        let mut bad = config.clone();
        bad.register.as_mut().unwrap().addresses.as_mut().unwrap()[0] =
            RegisterAddress::ModuleChain { offsets: vec![] };
        assert!(bad.validate().is_err());
        let mut same = config.clone();
        same.register.as_mut().unwrap().counter_slot = 14;
        assert!(same.validate().is_err());
    }

    #[test]
    fn derived_mode_needs_no_addresses_and_allows_empty_targets() {
        let parsed: Config = serde_json::from_str(
            r#"{
            "enabled": true,
            "allow_unverified_offsets": true,
            "register": {
                "table_code_slots": [14, 15, 16],
                "option_slots": [17, 18],
                "counter_slot": 19
            }
        }"#,
        )
        .unwrap();
        parsed.validate().unwrap();
        let register = parsed.register.as_ref().unwrap();
        assert_eq!(register.address_mode(), "derived from monster");
        assert!(register.targets_all());
        assert_eq!(register.slots(), [14, 15, 16, 17, 18, 19]);
        assert!(register.registers().validate().is_ok());
        let mut derived = mode(&[]);
        derived.addresses = None;
        derived.validate().unwrap();
        assert!(derived.explicit_addresses().is_none());
        assert!(derived.targets_all());
        assert!(serde_json::from_str::<Config>(
            r#"{"register":{"table_code_slots":[14,15,16],"option_slots":[17,18],"counter_slot":19,"addresses":[{"kind":"absolute","address":305397760}]}}"#
        )
        .is_err());
    }

    #[test]
    fn targets_are_sorted_deduplicated_and_unknown_fields_rejected() {
        let config: Config = serde_json::from_str(
            r#"{
            "enabled": true,
            "allow_unverified_offsets": true,
            "register": {
                "table_code_slots": [14, 15, 16],
                "option_slots": [17, 18],
                "counter_slot": 19,
                "target_tables": ["main:n105:s11", "main:n105:s1", "main:n105:s11"],
                "addresses": [
                    {"kind":"absolute","address":305397760},
                    {"kind":"absolute","address":305397764},
                    {"kind":"absolute","address":305397768},
                    {"kind":"absolute","address":305397772},
                    {"kind":"absolute","address":305397776},
                    {"kind":"absolute","address":305397780}
                ]
            }
        }"#,
        )
        .unwrap();
        config.validate().unwrap();
        let register = config.register.unwrap();
        assert_eq!(register.targets(), vec!["main:n105:s1", "main:n105:s11"]);
        assert_eq!(register.address_mode(), "explicit");
        assert!(serde_json::from_str::<Config>(
            r#"{"register":{"table_code_slots":[14,15,16],"option_slots":[17,18],"counter_slot":19,"extra":1}}"#
        )
        .is_err());
        assert!(serde_json::from_str::<Config>(
            r#"{"register":{"table_code_slots":[14,15,16],"option_slots":[17,18],"counter_slot":19,"target_tables":["a"],"addresses":[{"kind":"absolute","address":1}]}}"#
        )
        .is_err());
    }
}
