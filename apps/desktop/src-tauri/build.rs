use std::{env, path::Path};

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let ext = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let sidecar = Path::new(&manifest_dir)
        .join("binaries")
        .join(format!("pet-bridge-{target}{ext}"));
    let staged_manifest = Path::new(&manifest_dir).join("resources/resource-manifest.json");
    println!("cargo:rerun-if-env-changed=TAURI_ENV_TARGET_TRIPLE");
    println!("cargo:rerun-if-changed={}", sidecar.display());
    println!("cargo:rerun-if-changed={}", staged_manifest.display());

    // 仅在非 Tauri 打包的本地开发/测试/交叉检查且尚未准备发行资源时跳过随包资源复制；
    // 任何 `tauri build` 或 `release` 构建都必须严格校验并随包打包真实转发器与资源。
    if env::var_os("TAURI_ENV_TARGET_TRIPLE").is_none()
        && env::var("PROFILE").as_deref() != Ok("release")
        && env::var_os("TAURI_CONFIG").is_none()
        && (!sidecar.is_file() || !staged_manifest.is_file())
    {
        // SAFETY: 构建脚本单线程执行，在进入 tauri_build 前设置临时配置覆盖。
        unsafe {
            env::set_var(
                "TAURI_CONFIG",
                r#"{"bundle":{"externalBin":null,"resources":null}}"#,
            );
        }
    }

    let manifest = tauri_build::AppManifest::new().commands(&[
        "apply_integration",
        "choose_pet_directory",
        "clear_history",
        "copy_session_id",
        "get_deepseek_activity_status",
        "get_pet_asset",
        "get_playback",
        "get_settings",
        "get_snapshot",
        "import_pet_package",
        "integration_status",
        "interrupt_pet_activity",
        "list_pets",
        "mark_read",
        "open_notifications",
        "open_session",
        "pet_snapshot",
        "play_alert",
        "pointer_input",
        "record_diagnostic",
        "runtime_info",
        "show_pet_menu",
        "test_deepseek_connection",
        "trigger_pet_activity",
        "update_hit_region",
        "update_settings",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("无法构建桌宠权限清单");
}
