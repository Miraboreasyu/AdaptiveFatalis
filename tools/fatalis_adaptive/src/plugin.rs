//! Stracker-compatible DLL: nativePC/plugins is LoadLibrary-only, no Initialize ABI.
//! No game writes, hooks, remote threads or loader services are used here.
//! Process-lifetime module: intentionally pinned; hot unloading is unsupported.
use std::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, path::PathBuf};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleExW(
            flags: u32,
            name_or_address: *const u16,
            module: *mut *mut c_void,
        ) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, path: *mut u16, size: u32) -> u32;
        fn DisableThreadLibraryCalls(module: *mut c_void) -> i32;
        fn CreateThread(
            attributes: *const c_void,
            stack_size: usize,
            start: Option<unsafe extern "system" fn(*mut c_void) -> u32>,
            parameter: *mut c_void,
            flags: u32,
            id: *mut u32,
        ) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn OutputDebugStringW(message: *const u16);
    }
    fn diagnostic(text: &str) {
        let message: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        unsafe {
            OutputDebugStringW(message.as_ptr());
        }
    }
    fn plugin_dir(module: *mut c_void) -> Option<PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        let mut text = vec![0u16; 32768];
        let count =
            unsafe { GetModuleFileNameW(module, text.as_mut_ptr(), text.len() as u32) } as usize;
        if count == 0 || count >= text.len() {
            return None;
        }
        let path = PathBuf::from(std::ffi::OsString::from_wide(&text[..count]));
        Some(path.parent()?.to_path_buf())
    }
    unsafe extern "system" fn worker(parameter: *mut c_void) -> u32 {
        // DllMain pins before scheduling us, so DLL code cannot unmap in this worker.
        // Heavy Rust initialization and filesystem work occur only outside loader lock.
        let result = std::panic::catch_unwind(|| {
            let Some(dir) = plugin_dir(parameter) else {
                diagnostic("[fatalis_adaptive] Cannot resolve plugin directory; disabled.\n");
                return;
            };
            crate::runtime::run(dir, &STOP);
        });
        if result.is_err() {
            diagnostic("[fatalis_adaptive] Worker panicked; disabled until restart.\n");
        }
        0
    }

    /// Windows calls this under loader lock. Never join, flush, restore files,
    /// initialize the runtime, or call game functions from here.
    #[no_mangle]
    pub unsafe extern "system" fn DllMain(
        module: *mut c_void,
        reason: u32,
        _reserved: *mut c_void,
    ) -> i32 {
        const DLL_PROCESS_ATTACH: u32 = 1;
        const DLL_PROCESS_DETACH: u32 = 0;
        if reason == DLL_PROCESS_ATTACH {
            DisableThreadLibraryCalls(module);
            // FROM_ADDRESS|PIN: retain this module until process termination.
            // Pinning is deliberate: Stracker has no safe shutdown/unload callback.
            let mut pinned = std::ptr::null_mut();
            if GetModuleHandleExW(
                0x0000_0004 | 0x0000_0001,
                DllMain as *const () as *const u16,
                &mut pinned,
            ) == 0
            {
                return 1; // fail closed without preventing the game from loading
            }
            let thread = CreateThread(
                std::ptr::null(),
                0,
                Some(worker),
                module,
                0,
                std::ptr::null_mut(),
            );
            if !thread.is_null() {
                CloseHandle(thread);
            }
        } else if reason == DLL_PROCESS_DETACH {
            STOP.store(true, Ordering::Release);
            // Other threads may already be gone. The companion watchdog waits for
            // the process handle; startup journal recovery remains a fallback.
        }
        1
    }
}
