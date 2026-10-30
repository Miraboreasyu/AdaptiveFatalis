//! Opt-in compact THK markers. The signature/ABI are offline evidence, not a
//! live-game compatibility guarantee. The caller must pin this DLL for the
//! process lifetime before installing; hooks/trampolines are never unloaded here.
//!
//! Our mutex serializes only this marker writer and this sampler. Native game
//! register writes, other injectors, and object destruction do not obey it.

const OPCODE: u32 = 0x7f41_4446;
const COOKIE: u32 = 0x314d_4641;
const REGISTER_OFFSET: u64 = 0x640 + 4 * 14;
// Complete entry prefix, followed immediately at +0x18 by the known signature.
const ENTRY: &[u8] = &[
    0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x56, 0x48, 0x8d, 0x6c, 0x24,
    0xc9, 0x48, 0x81, 0xec, 0xd0, 0x00, 0x00, 0x00, 0x45, 0x33, 0xf6, 0x40, 0x32, 0xff, 0x44, 0x89,
    0x75, 0x67, 0x4d, 0x8b, 0xe0,
];

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Exactly one runtime-function record must identify this entry, cover the
/// complete fingerprint, and have an in-image unwind RVA. Reject overlapping
/// entries as well. This checks pdata boundaries, not the ABI or unwind program.
fn pdata_confirms(pdata: &[u8], entry: u32, image_size: u32) -> bool {
    if pdata.is_empty() || !pdata.len().is_multiple_of(12) {
        return false;
    }
    let Some(fingerprint_end) = entry.checked_add(ENTRY.len() as u32) else {
        return false;
    };
    let mut count = 0;
    for record in pdata.as_chunks::<12>().0 {
        let begin = u32_at(record, 0).unwrap();
        let end = u32_at(record, 4).unwrap();
        let unwind = u32_at(record, 8).unwrap();
        if begin <= entry && entry < end {
            if begin != entry
                || end < fingerprint_end
                || end > image_size
                || unwind == 0
                || unwind >= image_size
            {
                return false;
            }
            count += 1;
        }
    }
    count == 1
}

fn marker_registers(previous: &[u8; 24], id: u32) -> Option<[u8; 24]> {
    if id > 99_999 {
        return None;
    }
    let mut output = [0u8; 24];
    let mut divisor = 10_000;
    for index in 0..5 {
        let digit = (id / divisor) % 10;
        output[index * 4..index * 4 + 4].copy_from_slice(&digit.to_le_bytes());
        divisor /= 10;
    }
    let next = u32_at(previous, 20)?.wrapping_add(1);
    output[20..24].copy_from_slice(&next.to_le_bytes());
    Some(output)
}

fn decode_registers(bytes: &[u8; 24]) -> Option<(u32, i32)> {
    let mut id = 0;
    for index in 0..5 {
        let digit = u32_at(bytes, index * 4)?;
        if digit > 9 {
            return None;
        }
        id = id * 10 + digit;
    }
    Some((u32_at(bytes, 20)?, id as i32))
}

#[cfg(windows)]
mod platform {
    use super::*;
    use minhook::MinHook;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
    use std::sync::{Mutex, OnceLock};

    type Interpreter = unsafe extern "system" fn(*mut c_void, *const u8, *mut c_void) -> bool;
    static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static INSTALL: OnceLock<Result<(), String>> = OnceLock::new();
    static REGISTERS: Mutex<()> = Mutex::new(());
    // A short/failed WPM might leave partial digits. Never publish another sample
    // in this process after such a failure (there is no safe rollback guarantee).
    static WRITE_FAILED: AtomicBool = AtomicBool::new(false);

    #[repr(C)]
    #[derive(Default)]
    struct MemoryBasicInformation {
        base_address: usize,
        allocation_base: usize,
        allocation_protect: u32,
        partition_id: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        kind: u32,
    }

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
        fn WriteProcessMemory(
            process: *mut c_void,
            address: *mut c_void,
            buffer: *const c_void,
            size: usize,
            done: *mut usize,
        ) -> i32;
        fn VirtualQuery(
            address: *const c_void,
            information: *mut MemoryBasicInformation,
            length: usize,
        ) -> usize;
    }

    fn valid_range(address: u64, size: usize) -> bool {
        address >= 0x10000
            && address
                .checked_add(size as u64)
                .is_some_and(|end| end <= 0x0000_8000_0000_0000)
    }
    fn read(address: u64, buffer: &mut [u8]) -> Option<()> {
        if !valid_range(address, buffer.len()) {
            return None;
        }
        let mut done = 0;
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
    fn read_array<const N: usize>(address: u64) -> Option<[u8; N]> {
        let mut result = [0; N];
        read(address, &mut result)?;
        Some(result)
    }
    fn write(address: u64, buffer: &[u8]) -> bool {
        if !valid_range(address, buffer.len()) {
            return false;
        }
        let mut done = 0;
        let ok = unsafe {
            WriteProcessMemory(
                GetCurrentProcess(),
                address as *mut c_void,
                buffer.as_ptr().cast(),
                buffer.len(),
                &mut done,
            )
        };
        ok != 0 && done == buffer.len()
    }

    struct Image {
        base: u64,
        size: u32,
        text: (u32, u32),
        pdata: (u32, u32),
    }
    fn parse_image(base: u64) -> Result<Image, String> {
        let dos = read_array::<64>(base).ok_or("unreadable DOS header")?;
        if &dos[..2] != b"MZ" {
            return Err("not an MZ image".into());
        }
        let pe = u32_at(&dos, 60).ok_or("missing PE offset")? as u64;
        if !(64..=0x10_0000).contains(&pe) {
            return Err("unreasonable PE offset".into());
        }
        let nt = read_array::<264>(base.checked_add(pe).ok_or("PE address overflow")?)
            .ok_or("unreadable PE headers")?;
        if &nt[..4] != b"PE\0\0" || u16_at(&nt, 4) != Some(0x8664) || u16_at(&nt, 24) != Some(0x20b)
        {
            return Err("expected AMD64 PE32+ image".into());
        }
        let sections = u16_at(&nt, 6).unwrap() as usize;
        let optional_size = u16_at(&nt, 20).unwrap() as usize;
        let size = u32_at(&nt, 24 + 56).unwrap();
        let header_size = u32_at(&nt, 24 + 60).unwrap() as u64;
        if !(1..=96).contains(&sections)
            || !(240..=4096).contains(&optional_size)
            || size == 0
            || size > 0x8000_0000
            || header_size > u64::from(size)
            || u32_at(&nt, 24 + 108).unwrap() < 4
        {
            return Err("unsupported PE header bounds".into());
        }
        let exception = (
            u32_at(&nt, 24 + 112 + 24).unwrap(),
            u32_at(&nt, 24 + 112 + 28).unwrap(),
        );
        let section_offset = pe + 24 + optional_size as u64;
        if section_offset + (sections * 40) as u64 > header_size {
            return Err("section table outside PE headers".into());
        }
        let mut table = vec![0; sections * 40];
        read(
            base.checked_add(section_offset)
                .ok_or("section address overflow")?,
            &mut table,
        )
        .ok_or("unreadable section table")?;
        let mut text = None;
        let mut pdata = None;
        for section in table.as_chunks::<40>().0 {
            let rva = u32_at(section, 12).unwrap();
            let len = u32_at(section, 8).unwrap();
            let in_image = rva.checked_add(len).is_some_and(|end| end <= size);
            if &section[..8] == b".text\0\0\0" {
                if text.is_some()
                    || !in_image
                    || len < ENTRY.len() as u32
                    || u32_at(section, 36).unwrap() & 0x6000_0000 != 0x6000_0000
                {
                    return Err("invalid/duplicate/non-executable .text".into());
                }
                text = Some((rva, len));
            }
            if &section[..8] == b".pdata\0\0" {
                if pdata.is_some() || !in_image || len == 0 {
                    return Err("invalid/duplicate .pdata".into());
                }
                pdata = Some((rva, len));
            }
        }
        let text = text.ok_or("missing .text")?;
        let (pdata_rva, pdata_len) = pdata.ok_or("missing .pdata")?;
        if exception.1 == 0
            || !exception.1.is_multiple_of(12)
            || exception.1 > 32 * 1024 * 1024
            || exception.0 < pdata_rva
            || exception
                .0
                .checked_add(exception.1)
                .is_none_or(|end| end > pdata_rva + pdata_len)
        {
            return Err("exception directory outside .pdata or malformed".into());
        }
        Ok(Image {
            base,
            size,
            text,
            pdata: exception,
        })
    }

    /// Read only committed readable/executable regions. Refuse an incomplete
    /// scan rather than claiming uniqueness across unreadable .text gaps.
    fn locate(image: &Image) -> Result<u64, String> {
        let start = image
            .base
            .checked_add(u64::from(image.text.0))
            .ok_or("text overflow")?;
        let end = start
            .checked_add(u64::from(image.text.1))
            .ok_or("text end overflow")?;
        let mut at = start;
        let mut carry = Vec::new();
        let mut found = None;
        while at < end {
            let mut mbi = MemoryBasicInformation::default();
            let queried = unsafe {
                VirtualQuery(
                    at as *const c_void,
                    &mut mbi,
                    std::mem::size_of::<MemoryBasicInformation>(),
                )
            };
            let region_end = (mbi.base_address as u64)
                .checked_add(mbi.region_size as u64)
                .ok_or("VirtualQuery region overflow")?;
            // PAGE_EXECUTE alone is not readable; GUARD/NOACCESS are rejected.
            if queried != std::mem::size_of::<MemoryBasicInformation>()
                || mbi.state != 0x1000
                || mbi.protect & 0x100 != 0
                || !matches!(mbi.protect & 0xff, 0x20 | 0x40 | 0x80)
                || mbi.base_address as u64 > at
                || region_end <= at
            {
                return Err(".text contains inaccessible/non-readable-executable pages".into());
            }
            let until = end.min(region_end);
            while at < until {
                let len = (until - at).min(64 * 1024) as usize;
                let carry_len = carry.len();
                let mut buffer = carry;
                buffer.resize(carry_len + len, 0);
                read(at, &mut buffer[carry_len..]).ok_or(".text ReadProcessMemory failed")?;
                for (offset, window) in buffer.windows(ENTRY.len()).enumerate() {
                    if window == ENTRY {
                        let address = at - carry_len as u64 + offset as u64;
                        if found.replace(address).is_some() {
                            return Err("compact interpreter fingerprint is not unique".into());
                        }
                    }
                }
                carry = buffer[buffer.len().saturating_sub(ENTRY.len() - 1)..].to_vec();
                at += len as u64;
            }
        }
        let address = found.ok_or("compact interpreter fingerprint not found")?;
        let mut pdata = vec![0; image.pdata.1 as usize];
        read(image.base + u64::from(image.pdata.0), &mut pdata).ok_or("unreadable .pdata")?;
        let rva = u32::try_from(address - image.base).map_err(|_| "entry RVA overflow")?;
        if !pdata_confirms(&pdata, rva, image.size) {
            return Err(".pdata does not confirm unique fingerprint function entry".into());
        }
        Ok(address)
    }

    fn reject_conflicts() -> Result<(), String> {
        // Known-name detection only: renamed, later-loaded or unknown hooks are
        // not ruled out. The entry fingerprint also rejects an existing detour.
        for name in [
            "mhw-thk-extender.dll",
            "mhw_thk_extender.dll",
            "thk-extender.dll",
            "thk_extender.dll",
            "THKExtender.dll",
        ] {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            if !unsafe { GetModuleHandleW(wide.as_ptr()) }.is_null() {
                return Err(format!("conflicting loaded THK extension: {name}"));
            }
        }
        Ok(())
    }

    fn install() -> Result<(), String> {
        if !cfg!(target_pointer_width = "64") {
            return Err("compact hook requires Windows x64".into());
        }
        reject_conflicts()?;
        let base = unsafe { GetModuleHandleW(std::ptr::null()) } as u64;
        if base == 0 {
            return Err("GetModuleHandleW(NULL) failed".into());
        }
        let address = locate(&parse_image(base)?)?;
        reject_conflicts()?;
        // Narrow the scan-to-create window; this is not a guarantee against
        // another injector racing us after this check.
        let mut current = vec![0; ENTRY.len()];
        read(address, &mut current).ok_or("entry reread failed")?;
        if current != ENTRY {
            return Err("interpreter changed during hook preparation".into());
        }
        let target = address as *mut c_void;
        let original = unsafe { MinHook::create_hook(target, hook as *const () as *mut c_void) }
            .map_err(|error| format!("MinHook create_hook: {error:?}"))?;
        if original.is_null() {
            return Err("MinHook returned a null trampoline".into());
        }
        ORIGINAL.store(original, Ordering::Release);
        // Never enable all hooks, uninitialize the shared MinHook runtime, or
        // remove the trampoline. On failure retain state for process lifetime.
        unsafe { MinHook::enable_hook(target) }
            .map_err(|error| format!("MinHook enable_hook: {error:?}"))?;
        INSTALLED.store(true, Ordering::Release);
        Ok(())
    }

    pub fn try_install() -> Result<(), String> {
        INSTALL
            .get_or_init(|| {
                std::panic::catch_unwind(install)
                    .unwrap_or_else(|_| Err("compact hook installation panicked".into()))
            })
            .clone()
    }
    pub fn installed() -> bool {
        INSTALLED.load(Ordering::Acquire)
    }
    pub fn read_code(context: u64) -> Option<(u32, i32)> {
        if !installed() || WRITE_FAILED.load(Ordering::Acquire) {
            return None;
        }
        let _guard = REGISTERS.lock().ok()?;
        if WRITE_FAILED.load(Ordering::Acquire) {
            return None;
        }
        let bytes = read_array::<24>(context.checked_add(REGISTER_OFFSET)?)?;
        decode_registers(&bytes)
    }

    unsafe extern "system" fn hook(em: *mut c_void, segment: *const u8, arg: *mut c_void) -> bool {
        let address = segment as u64;
        let opcode = address
            .checked_add(8)
            .and_then(read_array::<4>)
            .map(u32::from_le_bytes);
        if opcode == Some(OPCODE) {
            // Reserved magic never reaches the native interpreter, even with
            // malformed payloads, unreadable context or a poisoned lock.
            let Some(payload) = address.checked_add(12).and_then(read_array::<24>) else {
                return false;
            };
            let id = u32_at(&payload, 0).unwrap();
            if u32_at(&payload, 20) != Some(COOKIE) || id > 99_999 {
                return false;
            }
            let Ok(_guard) = REGISTERS.lock() else {
                return false;
            };
            if WRITE_FAILED.load(Ordering::Acquire) {
                return false;
            }
            let Some(registers) = (em as u64).checked_add(REGISTER_OFFSET) else {
                return false;
            };
            let Some(previous) = read_array::<24>(registers) else {
                return false;
            };
            let Some(next) = marker_registers(&previous, id) else {
                return false;
            };
            if !write(registers, &next) {
                WRITE_FAILED.store(true, Ordering::Release);
                return false;
            }
            return true;
        }
        // Crucially no register mutex is held while the interpreter executes;
        // it can recursively enter this detour.
        let original = ORIGINAL.load(Ordering::Acquire);
        if original.is_null() {
            return false;
        }
        let original: Interpreter = unsafe { std::mem::transmute(original) };
        unsafe { original(em, segment, arg) }
    }
}

/// Install only on a validated, uniquely fingerprinted Windows x64 interpreter.
/// An error leaves the caller responsible for choosing the native fallback.
#[cfg(windows)]
pub use platform::{installed, read_code, try_install};

#[cfg(not(windows))]
pub fn try_install() -> Result<(), String> {
    Err("compact THK hook is only supported on Windows x64".into())
}
#[cfg(not(windows))]
pub fn installed() -> bool {
    false
}
#[cfg(not(windows))]
pub fn read_code(_: u64) -> Option<(u32, i32)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_layout_and_counter_wrap() {
        let mut previous = [0; 24];
        previous[20..].copy_from_slice(&u32::MAX.to_le_bytes());
        let next = marker_registers(&previous, 12_345).unwrap();
        assert_eq!(decode_registers(&next), Some((0, 12_345)));
        for (index, digit) in [1, 2, 3, 4, 5].into_iter().enumerate() {
            assert_eq!(u32_at(&next, index * 4), Some(digit));
        }
        assert!(marker_registers(&previous, 100_000).is_none());
        for id in [0, 1, 99, 100, 99_999] {
            assert_eq!(
                decode_registers(&marker_registers(&[0; 24], id).unwrap()),
                Some((1, id as i32))
            );
        }
    }
    #[test]
    fn rejects_foreign_register_digits() {
        let mut bytes = [0; 24];
        bytes[4..8].copy_from_slice(&10u32.to_le_bytes());
        assert!(decode_registers(&bytes).is_none());
    }
    #[test]
    fn pdata_requires_exact_entry_and_complete_fingerprint() {
        let record = |begin: u32, end: u32| {
            [
                begin.to_le_bytes(),
                end.to_le_bytes(),
                0x800u32.to_le_bytes(),
            ]
            .concat()
        };
        assert!(pdata_confirms(&record(0x100, 0x200), 0x100, 0x1000));
        assert!(!pdata_confirms(&record(0xff, 0x200), 0x100, 0x1000));
        assert!(!pdata_confirms(&record(0x100, 0x110), 0x100, 0x1000));
        assert!(!pdata_confirms(
            &[record(0x100, 0x200), record(0x100, 0x200)].concat(),
            0x100,
            0x1000
        ));
        assert!(!pdata_confirms(&record(0x100, 0x200)[..11], 0x100, 0x1000));
    }
    #[test]
    fn complete_entry_prefix_prevents_interior_signature_match() {
        let mut bytes = vec![0x90; 0x100];
        bytes.extend_from_slice(ENTRY);
        let hits: Vec<usize> = bytes
            .windows(ENTRY.len())
            .enumerate()
            .filter_map(|(i, bytes)| (bytes == ENTRY).then_some(i))
            .collect();
        assert_eq!(hits, [0x100]);
        assert_eq!(
            &ENTRY[0x18..],
            &[0x45, 0x33, 0xf6, 0x40, 0x32, 0xff, 0x44, 0x89, 0x75, 0x67, 0x4d, 0x8b, 0xe0]
        );
        bytes[0x100] = 0xe9;
        assert!(!bytes.windows(ENTRY.len()).any(|bytes| bytes == ENTRY));
    }
}
