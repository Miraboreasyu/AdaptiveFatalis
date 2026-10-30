use fatalis_adaptive::{
    deploy::Deployer, learning::Learning, model::Model, runtime, storage, thk::Project,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
};
fn main() {
    if let Err(e) = execute() {
        eprintln!("fatalis-adaptive: {e}");
        std::process::exit(1);
    }
}
fn arg(args: &[String], i: usize) -> Result<&str, String> {
    args.get(i)
        .map(String::as_str)
        .ok_or_else(|| "missing argument; run help".into())
}
fn execute() -> Result<(), String> {
    let a = env::args().collect::<Vec<_>>();
    match a.get(1).map(String::as_str).unwrap_or("help"){
        "inspect"=>{let p=Project::from_paths(arg(&a,2)?,arg(&a,3)?)?;println!("{}",serde_json::to_string_pretty(&Model::from_project(&p).coverage).map_err(|e|e.to_string())?);}
        "init"=>{
            let dir=PathBuf::from(arg(&a,2)?);let main=PathBuf::from(arg(&a,3)?);let global=PathBuf::from(arg(&a,4)?);
            let p=Project::from_paths(&main,&global)?;fs::create_dir_all(dir.join("baseline")).map_err(|e|e.to_string())?;
            for (path,bytes) in [(dir.join("baseline/em013_00.thk"),p.main.bytes),(dir.join("baseline/em013_55.thk"),p.global.bytes)]{
                if path.exists(){if fs::read(&path).map_err(|e|e.to_string())?!=bytes{return Err("existing baseline conflicts; refusing overwrite".into());}}else{storage::save(&path,&bytes)?;}
            }
            let config=dir.join("config.json");if !config.exists(){storage::json(&config,&fatalis_adaptive::strategy_config::Selection::default())?;}println!("initialized no-learning selection; no game files deployed");
        }
        "restore"|"recover"=>{
            let d=Deployer::new(arg(&a,2)?,arg(&a,3)?).map_err(|e|e.to_string())?;
            if a[1]=="restore"{d.restore()}else{d.recover()}.map_err(|e|e.to_string())?;
        }
        "generate"=>{
            let dir=Path::new(arg(&a,2)?);let p=runtime::project(dir)?;
            let c=fatalis_adaptive::strategy_config::load(dir)?;
            let l:Learning=if dir.join("learning.json").exists(){serde_json::from_slice(&fs::read(dir.join("learning.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?}else{Learning::default()};
            let out=Path::new(arg(&a,3)?);if out.exists(){return Err("output already exists; choose a new path".into());}
            storage::save(out,&p.patch_main_weights(&runtime::targets(&p,&l,&c)?)?)?;
        }
        "watch"=>watch(arg(&a,2)?.parse().map_err(|_|"invalid PID")?,Path::new(arg(&a,3)?),Path::new(arg(&a,4)?),Path::new(arg(&a,5)?),arg(&a,6)?)?,
        _=>println!("fatalis-adaptive-tool\n  inspect MAIN_THK GLOBAL_THK\n  init PRIVATE_DIR MAIN_THK GLOBAL_THK\n  generate PRIVATE_DIR NEW_OUTPUT\n  restore GAME_ROOT DEPLOYMENT_STATE_DIR\n  recover GAME_ROOT DEPLOYMENT_STATE_DIR\n  watch (internal watchdog; do not start manually)"),
    }
    Ok(())
}
#[cfg(windows)]
fn watch(pid: u32, root: &Path, state: &Path, ready: &Path, nonce: &str) -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let handle = unsafe { OpenProcess(0x00100000, 0, pid) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let result = (|| {
        storage::save(ready, nonce.as_bytes())?;
        let wait = unsafe { WaitForSingleObject(handle, u32::MAX) };
        if wait != 0 {
            return Err(format!("process wait failed {wait}"));
        }
        let d = Deployer::new(root, state).map_err(|e| e.to_string())?;
        d.restore().map_err(|e| e.to_string())?;
        Ok(())
    })();
    unsafe {
        CloseHandle(handle);
    }
    if let Err(ref e) = result {
        runtime::log(state, &format!("watchdog restore failed: {e}"));
    }
    result
}
#[cfg(not(windows))]
fn watch(_: u32, _: &Path, _: &Path, _: &Path, _: &str) -> Result<(), String> {
    Err("watchdog requires Windows".into())
}
