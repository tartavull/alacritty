#[cfg(target_os = "macos")]
#[test]
fn webview_smoke_renders() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_webview_smoke"))
        .status()
        .expect("launch webview_smoke");

    assert!(status.success(), "webview_smoke exited with {status}");
}
