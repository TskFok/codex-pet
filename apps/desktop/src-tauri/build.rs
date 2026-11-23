fn main() {
    let manifest = tauri_build::AppManifest::new().commands(&[
        "apply_integration",
        "choose_pet_directory",
        "clear_history",
        "copy_session_id",
        "get_pet_asset",
        "get_playback",
        "get_settings",
        "get_snapshot",
        "import_pet_package",
        "integration_status",
        "list_pets",
        "mark_read",
        "open_session",
        "pet_snapshot",
        "play_alert",
        "pointer_input",
        "record_diagnostic",
        "runtime_info",
        "update_hit_region",
        "update_settings",
    ]);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .expect("无法构建桌宠权限清单");
}
