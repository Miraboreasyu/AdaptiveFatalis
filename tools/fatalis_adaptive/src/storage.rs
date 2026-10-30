use std::{fs, path::Path};
#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 1 | 8) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> Result<(), String> {
    fs::rename(source, target).map_err(|e| e.to_string())
}
pub fn save(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("missing parent directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    if let Err(e) = f.write_all(bytes).and_then(|_| f.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(e.to_string());
    }
    drop(f);
    if let Err(e) = replace(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}
pub fn json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    save(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
