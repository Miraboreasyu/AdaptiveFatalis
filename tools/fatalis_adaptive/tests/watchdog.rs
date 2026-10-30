//! No game is launched. A disposable PowerShell process stands in for its lifetime.
#[cfg(windows)]
#[test]
fn watchdog_restores_after_watched_process_exits() {
    use fatalis_adaptive::deploy::Deployer;
    use std::{
        fs,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!(
        "fatalis-watchdog-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let game = root.join("game");
    let state = root.join("private");
    let target = game.join("nativePC/em/em013/00/data/em013_00.thk");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, b"original").unwrap();
    let source = root.join("generated.thk");
    fs::write(&source, b"adaptive").unwrap();
    let deploy = Deployer::new(&game, &state).unwrap();
    deploy.install(&source).unwrap();
    let mut watched = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let ready = state.join("ready");
    let nonce = "test-nonce";
    let mut watchdog = Command::new(env!("CARGO_BIN_EXE_fatalis-adaptive-tool"))
        .arg("watch")
        .arg(watched.id().to_string())
        .arg(&game)
        .arg(&state)
        .arg(&ready)
        .arg(nonce)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !ready.exists() && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let acknowledged = fs::read_to_string(&ready).ok().as_deref() == Some(nonce);
    let _ = watched.kill();
    let _ = watched.wait();
    let start = Instant::now();
    let status = loop {
        if let Some(s) = watchdog.try_wait().unwrap() {
            break Some(s);
        }
        if start.elapsed() > Duration::from_secs(5) {
            let _ = watchdog.kill();
            let _ = watchdog.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(acknowledged);
    assert!(status.unwrap().success());
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert!(deploy.installed().unwrap().is_none());
    // Only this unique test-created absolute root is deleted.
    assert!(root.starts_with(std::env::temp_dir()));
    fs::remove_dir_all(root).unwrap();
}
