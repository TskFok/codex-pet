use pet_core::{
    AppPaths, EventEnvelope, JournalStore,
    pet_package::{FrameRect, PetError, import_pet, load_pet},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Barrier,
    thread,
};

const EXPLICIT_MANIFEST: &str =
    include_str!("../../../tests/fixtures/pet-packages/explicit-v1.json");
const SMALL_PNG: &[u8] = include_bytes!("../../../tests/fixtures/pet-packages/atlas-5x2.png");
const LEGACY_PNG: &[u8] =
    include_bytes!("../../../tests/fixtures/pet-packages/legacy-1536x1872.png");
const CORRUPT_PNG: &[u8] =
    include_bytes!("../../../tests/fixtures/pet-packages/corrupt-legacy.png");
const OVERSIZE_PNG: &[u8] =
    include_bytes!("../../../tests/fixtures/pet-packages/oversize-header.png");

const ROWS: [(&str, &[u32], Option<usize>); 9] = [
    ("idle", &[280, 110, 110, 140, 140, 320], Some(0)),
    (
        "running-right",
        &[120, 120, 120, 120, 120, 120, 120, 220],
        Some(0),
    ),
    (
        "running-left",
        &[120, 120, 120, 120, 120, 120, 120, 220],
        Some(0),
    ),
    ("waving", &[140, 140, 140, 280], None),
    ("jumping", &[140, 140, 140, 140, 280], None),
    ("failed", &[140, 140, 140, 140, 140, 140, 140, 240], None),
    ("waiting", &[150, 150, 150, 150, 150, 260], Some(0)),
    ("running", &[120, 120, 120, 120, 120, 220], Some(0)),
    ("review", &[150, 150, 150, 150, 150, 280], Some(0)),
];

fn sandbox() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    (temp, root)
}

fn legacy_manifest(sprite_path: &str) -> Value {
    json!({
        "id": "test-pet", "displayName": "导入测试", "spritesheetPath": sprite_path,
        "description": "旧包描述"
    })
}

fn write_manifest(dir: &Path, manifest: &Value) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("pet.json"), serde_json::to_vec(manifest).unwrap()).unwrap();
}

fn legacy_png_package(dir: &Path, bytes: &[u8]) {
    write_manifest(dir, &legacy_manifest("source.PNG"));
    fs::write(dir.join("source.PNG"), bytes).unwrap();
}

fn explicit_package(dir: &Path, manifest: &Value) {
    write_manifest(dir, manifest);
    fs::create_dir(dir.join("images")).unwrap();
    fs::write(dir.join("images/atlas.png"), SMALL_PNG).unwrap();
}

#[test]
fn imports_real_v1_and_v2_with_exact_legacy_timings_and_directions() {
    let (_temp, root) = sandbox();
    let paths = AppPaths::under(&root.join("应用数据"));
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (id, name, height, directions) in [
        ("yueying-cape-dancer", "夏乌拉", 1872, 0),
        ("deepseek-big-fish", "deepseek大肥鱼", 2288, 16),
    ] {
        let source = repo.join("pet-output").join(id).join("package");
        let destination = paths.pets.join(id);
        let source_json = fs::read(source.join("pet.json")).unwrap();
        let source_image = fs::read(source.join("spritesheet.webp")).unwrap();
        let manifest = import_pet(&source, &destination).unwrap();
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.id, id);
        assert_eq!(manifest.name, name);
        assert_eq!(manifest.atlas.path, "atlas.webp");
        assert_eq!(
            (manifest.atlas.width, manifest.atlas.height),
            (1536, height)
        );
        assert_eq!(
            (manifest.atlas.cell_width, manifest.atlas.cell_height),
            (192, 208)
        );
        assert_eq!(manifest.animations.len(), ROWS.len());
        for (row, (name, durations, loop_start)) in ROWS.iter().enumerate() {
            let action = &manifest.animations[*name];
            assert_eq!(action.loop_start, *loop_start);
            assert_eq!(action.frames.len(), durations.len());
            for (column, (frame, duration)) in action.frames.iter().zip(*durations).enumerate() {
                assert_eq!(frame.duration_ms, *duration);
                assert_eq!(
                    frame.rect,
                    FrameRect {
                        x: column as u32 * 192,
                        y: row as u32 * 208,
                        width: 192,
                        height: 208
                    },
                );
            }
        }
        if directions == 0 {
            assert!(manifest.look_directions.is_none());
        } else {
            let look = manifest.look_directions.as_ref().unwrap();
            assert_eq!(look.len(), directions);
            for (index, frame) in look.iter().enumerate() {
                assert_eq!(
                    *frame,
                    FrameRect {
                        x: index as u32 % 8 * 192,
                        y: (9 + index as u32 / 8) * 208,
                        width: 192,
                        height: 208
                    },
                );
            }
        }

        let destination_json = fs::read(destination.join("pet.json")).unwrap();
        let destination_image = fs::read(destination.join("atlas.webp")).unwrap();
        assert_eq!(destination_image, source_image);
        assert_ne!(destination_json, source_json);
        assert_eq!(fs::read(source.join("pet.json")).unwrap(), source_json);
        assert_eq!(
            fs::read(source.join("spritesheet.webp")).unwrap(),
            source_image
        );
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);
        assert_eq!(load_pet(&destination).unwrap(), manifest);

        let wire: Value = serde_json::from_slice(&destination_json).unwrap();
        assert_eq!(wire["atlas"]["cellWidth"], 192);
        assert_eq!(wire["animations"]["idle"]["frames"][0]["durationMs"], 280);
        assert_eq!(wire["animations"]["idle"]["loopStart"], 0);
        assert!(wire["animations"]["waving"].get("loopStart").is_none());
        assert!(wire.get("description").is_none());
        assert_eq!(wire.get("lookDirections").is_some(), directions != 0);
        println!(
            "{id} {name} 1536×{height}, {} 字节图集，{} 字节清单",
            destination_image.len(),
            destination_json.len()
        );
        for (row, (name, _, _)) in ROWS.iter().enumerate() {
            let action = &manifest.animations[*name];
            let timings: Vec<_> = action
                .frames
                .iter()
                .map(|frame| frame.duration_ms)
                .collect();
            println!("行 {row} {name}: {timings:?}");
        }
        println!("方向: {:?}", manifest.look_directions);
    }

    let mut journal = JournalStore::open(paths.clone()).unwrap();
    let mut event: EventEnvelope = serde_json::from_str(include_str!(
        "../../../tests/fixtures/protocol/valid-end.json"
    ))
    .unwrap();
    event.occurred_at_ms = pet_core::paths::wall_clock_ms();
    journal.ingest(event).unwrap();
    let before = journal.recover(pet_core::paths::wall_clock_ms()).unwrap();
    assert_eq!(before.notifications.len(), 1);
    let count = journal.clear_history().unwrap().notifications.len();
    assert_eq!(count, 0);
    for id in ["yueying-cape-dancer", "deepseek-big-fish"] {
        let destination = paths.pets.join(id);
        let manifest = load_pet(&destination).unwrap();
        assert_eq!(manifest.id, id);
        assert_eq!(
            fs::read(destination.join(&manifest.atlas.path)).unwrap(),
            fs::read(
                repo.join("pet-output")
                    .join(id)
                    .join("package/spritesheet.webp")
            )
            .unwrap()
        );
    }
    println!(
        "清空前 {} 条通知，清空后 {count} 条，两个宠物包仍可加载",
        before.notifications.len()
    );
}

#[test]
fn explicit_manifest_loads_a_nine_frame_action_across_rows() {
    let (_temp, root) = sandbox();
    let manifest_json: Value = serde_json::from_str(EXPLICIT_MANIFEST).unwrap();
    explicit_package(&root, &manifest_json);
    let manifest = load_pet(&root).unwrap();
    let idle = &manifest.animations["idle"];
    assert_eq!(idle.frames.len(), 9);
    assert_eq!(
        idle.frames[8].rect,
        FrameRect {
            x: 3,
            y: 1,
            width: 1,
            height: 1
        }
    );
    assert_eq!(idle.frames[8].duration_ms, 48);
    assert_eq!(idle.loop_start, Some(2));
    assert_eq!(serde_json::to_value(manifest).unwrap(), manifest_json);
}

#[test]
fn rejects_invalid_frame_bounds_and_timing_in_explicit_manifest() {
    let (_temp, root) = sandbox();
    let original: Value = serde_json::from_str(EXPLICIT_MANIFEST).unwrap();
    explicit_package(&root, &original);
    for (pointer, invalid_value) in [
        ("/animations/idle/frames/0/rect/x", json!(5)),
        ("/animations/idle/frames/0/rect/y", json!(u32::MAX)),
        ("/animations/idle/frames/0/rect/width", json!(0)),
        ("/animations/idle/frames/0/durationMs", json!(0)),
        ("/animations/idle/loopStart", json!(9)),
        ("/atlas/cellWidth", json!(6)),
        ("/atlas/width", json!(6)),
        ("/version", json!(2)),
        ("/id", json!("../other")),
        ("/name", json!(" ")),
        ("/animations/idle/frames", json!([])),
        ("/animations", json!({})),
    ] {
        let mut invalid = original.clone();
        *invalid.pointer_mut(pointer).unwrap() = invalid_value;
        write_manifest(&root, &invalid);
        assert!(
            matches!(load_pet(&root), Err(PetError::InvalidManifest(_))),
            "{pointer}"
        );
    }
    for invalid in [
        json!([]),
        json!(vec![json!({"x": 0, "y": 0, "width": 5, "height": 3}); 16]),
    ] {
        let mut manifest = original.clone();
        manifest["lookDirections"] = invalid;
        write_manifest(&root, &manifest);
        assert!(matches!(load_pet(&root), Err(PetError::InvalidManifest(_))));
    }
}

#[test]
fn rejects_corrupt_image_pixels_then_can_retry_the_same_destination() {
    let (_temp, root) = sandbox();
    let source = root.join("old");
    let destination = root.join("pets/new");
    legacy_png_package(&source, CORRUPT_PNG);
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::Image(_))
    ));
    assert!(!destination.exists());
    assert!(!destination.parent().unwrap().exists());

    fs::write(source.join("source.PNG"), LEGACY_PNG).unwrap();
    let manifest = import_pet(&source, &destination).unwrap();
    assert_eq!(manifest.atlas.path, "atlas.png");
    assert_eq!(manifest.animations["idle"].frames.len(), 6);
    assert_eq!(fs::read(destination.join("atlas.png")).unwrap(), LEGACY_PNG);
}

#[test]
fn rejects_file_and_header_limits_before_decoding_corrupt_pixels() {
    let (_temp, root) = sandbox();
    let source = root.join("old");
    let destination = root.join("new");
    legacy_png_package(&source, OVERSIZE_PNG);
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::ImageTooLarge { .. })
    ));
    fs::OpenOptions::new()
        .write(true)
        .open(source.join("source.PNG"))
        .unwrap()
        .set_len(20 * 1024 * 1024 + 1)
        .unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::FileTooLarge(_))
    ));
    assert!(!destination.exists());
}

#[test]
fn rejects_corrupt_webp_pixels_and_oversized_nested_vp8_before_decoding() {
    let (_temp, root) = sandbox();
    let source = root.join("old");
    let destination = root.join("new");
    write_manifest(&source, &legacy_manifest("atlas.webp"));
    let atlas = source.join("atlas.webp");
    fs::write(
        &atlas,
        include_bytes!("../../../tests/fixtures/pet-packages/corrupt-legacy.webp"),
    )
    .unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::Image(_))
    ));

    fs::write(
        &atlas,
        include_bytes!("../../../tests/fixtures/pet-packages/oversize-vp8-chunk.webp"),
    )
    .unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::ImageTooLarge {
            width: 4097,
            height: 1872
        })
    ));
    assert!(!destination.exists());
}

#[test]
fn distinguishes_metadata_version_and_layout_errors_from_image_payload_errors() {
    let (_temp, root) = sandbox();
    let source = root.join("old");
    let destination = root.join("new");
    let mut metadata = legacy_manifest("source.PNG");
    legacy_png_package(&source, SMALL_PNG);
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::InvalidManifest(_))
    ));
    metadata["spriteVersionNumber"] = json!(2);
    write_manifest(&source, &metadata);
    fs::write(source.join("source.PNG"), LEGACY_PNG).unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::InvalidManifest(_))
    ));
    metadata["spriteVersionNumber"] = json!(3);
    write_manifest(&source, &metadata);
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::InvalidManifest(_))
    ));
    fs::write(source.join("pet.json"), b"{").unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::Json(_))
    ));
    assert!(!destination.exists());
}

#[test]
fn rejects_platform_specific_path_escapes_in_both_formats() {
    let (_temp, root) = sandbox();
    let source = root.join("old");
    let destination = root.join("new");
    let original: Value = serde_json::from_str(EXPLICIT_MANIFEST).unwrap();
    explicit_package(&source, &original);
    for escape in [
        "../atlas.png",
        "images/../../atlas.png",
        "images/./atlas.png",
        "/images/atlas.png",
        "C:atlas.png",
        "C:/images/atlas.png",
        "images\\..\\atlas.png",
        "\\\\server\\pet\\atlas.png",
        "//server/pet/atlas.png",
        "images/atlas.png:secret",
        "images/NUL.png",
        "images/COM¹.png",
        "images/atlas.png.",
        "images/atlas.png ",
        "images//atlas.png",
        "images/atlas\0.png",
    ] {
        write_manifest(&source, &legacy_manifest(escape));
        assert!(
            matches!(
                import_pet(&source, &destination),
                Err(PetError::UnsafePath(_))
            ),
            "{escape:?}"
        );
        let mut manifest = original.clone();
        manifest["atlas"]["path"] = json!(escape);
        write_manifest(&source, &manifest);
        assert!(
            matches!(load_pet(&source), Err(PetError::UnsafePath(_))),
            "{escape:?}"
        );
    }
    assert!(!destination.exists());
}

#[cfg(unix)]
#[test]
fn rejects_atlas_file_and_parent_links_and_a_linked_destination_parent() {
    use std::os::unix::fs::symlink;

    let (_temp, root) = sandbox();
    let source = root.join("old");
    let outside = root.join("outside");
    let destination = root.join("new");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("atlas.png"), LEGACY_PNG).unwrap();

    write_manifest(&source, &legacy_manifest("atlas.png"));
    symlink(outside.join("atlas.png"), source.join("atlas.png")).unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::UnsafePath(_))
    ));

    write_manifest(&source, &legacy_manifest("images/atlas.png"));
    symlink(&outside, source.join("images")).unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::UnsafePath(_))
    ));

    legacy_png_package(&source, LEGACY_PNG);
    symlink(&outside, root.join("linked")).unwrap();
    assert!(matches!(
        import_pet(&source, &root.join("linked/new")),
        Err(PetError::UnsafePath(_))
    ));
    assert!(!outside.join("new").exists());
    assert!(!destination.exists());
}

#[test]
fn refuses_placing_the_import_inside_its_source_directory() {
    let (_temp, root) = sandbox();
    let source = root.join("old");
    legacy_png_package(&source, LEGACY_PNG);
    let destination = source.join("nested/new");
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::UnsafePath(_))
    ));
    assert!(!source.join("nested").exists());
}

#[cfg(unix)]
#[test]
fn rejects_source_directory_link_with_trailing_separator() {
    use std::os::unix::fs::symlink;

    let (_temp, root) = sandbox();
    let source = root.join("old");
    let destination = root.join("pets/new");
    let linked = root.join("linked");
    legacy_png_package(&source, LEGACY_PNG);
    symlink(&source, &linked).unwrap();
    for alias in [linked.join(""), linked.join(".")] {
        assert!(matches!(
            import_pet(&alias, &destination),
            Err(PetError::UnsafePath(_))
        ));
        assert!(!destination.exists());
    }
    import_pet(&source.join(""), &destination).unwrap();
}

#[cfg(unix)]
#[test]
fn accepts_a_parent_directory_alias_and_still_keeps_the_source_separate() {
    use std::os::unix::fs::symlink;

    let (_temp, root) = sandbox();
    let workspace = root.join("workspace");
    let source = workspace.join("old");
    let alias = root.join("alias");
    legacy_png_package(&source, LEGACY_PNG);
    symlink(&workspace, &alias).unwrap();

    assert!(matches!(
        import_pet(&alias.join("old"), &source.join("nested/new")),
        Err(PetError::UnsafePath(_))
    ));
    assert!(!source.join("nested").exists());

    let destination = workspace.join("pets/new");
    let manifest = import_pet(&alias.join("old"), &destination).unwrap();
    assert_eq!(load_pet(&alias.join("pets/new")).unwrap(), manifest);
}

#[test]
fn existing_destination_is_untouched_even_when_empty() {
    let (_temp, root) = sandbox();
    let source = root.join("old");
    let destination = root.join("new");
    legacy_png_package(&source, LEGACY_PNG);
    fs::create_dir(&destination).unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::DestinationExists(_))
    ));
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
    fs::write(destination.join("sentinel"), b"existing data").unwrap();
    assert!(matches!(
        import_pet(&source, &destination),
        Err(PetError::DestinationExists(_))
    ));
    assert_eq!(
        fs::read(destination.join("sentinel")).unwrap(),
        b"existing data"
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
}

#[test]
fn competing_imports_publish_one_complete_package_without_leaving_staging_files() {
    let (_temp, root) = sandbox();
    let source = root.join("old");
    let destination = root.join("pets/new");
    legacy_png_package(&source, LEGACY_PNG);
    let start = Barrier::new(2);
    thread::scope(|scope| {
        let import = || {
            start.wait();
            import_pet(&source, &destination)
        };
        let first = scope.spawn(import);
        let second = scope.spawn(import);
        match (first.join().unwrap(), second.join().unwrap()) {
            (Ok(manifest), Err(PetError::DestinationExists(_)))
            | (Err(PetError::DestinationExists(_)), Ok(manifest)) => {
                assert_eq!(load_pet(&destination).unwrap(), manifest);
            }
            results => panic!("两个导入的结果不符合预期：{results:?}"),
        }
    });
    assert_eq!(
        fs::read_dir(destination.parent().unwrap()).unwrap().count(),
        1
    );
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);
    assert_eq!(fs::read(destination.join("atlas.png")).unwrap(), LEGACY_PNG);
}
