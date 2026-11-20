use pet_desktop::placement::{WindowPlacement, WorkArea, clamp_placement};

fn monitor(id: &str, x: f64, y: f64, w: f64, h: f64, dpi: f64) -> WorkArea {
    WorkArea {
        id: id.into(),
        x,
        y,
        width: w,
        height: h,
        scale_factor: dpi,
    }
}

#[test]
fn negative_secondary_origin_and_mixed_dpi_round_trip_local_offsets() {
    for dpi in [1., 1.5, 2.] {
        let screen = monitor("left", -2560., -300., 1920., 1200., dpi);
        let local = screen.capture(-2410., -150., 1.);
        assert_eq!((local.x, local.y), (150. / dpi, 150. / dpi));
        let fit = clamp_placement(local, std::slice::from_ref(&screen));
        assert_eq!(screen.position(&fit), (-2410., -150.));
        assert_eq!(fit.monitor_id.as_deref(), Some("left"));
    }
}

#[test]
fn removed_screen_uses_primary_work_area_and_dpi_change_clamps_bottom_right() {
    let left = monitor("removed", -2560., -300., 2560., 1600., 1.);
    let saved = left.capture(-110., 1000., 1.);
    let primary = monitor("primary", 30., 64., 1200., 900., 1.5);
    let fit = clamp_placement(saved, std::slice::from_ref(&primary));
    assert_eq!(fit.monitor_id.as_deref(), Some("primary"));
    assert_eq!(primary.position(&fit), (990., 694.));
    let changed = monitor("primary", 30., 64., 1200., 900., 2.);
    let fit = clamp_placement(fit, std::slice::from_ref(&changed));
    assert_eq!(changed.position(&fit), (910., 604.));
}

#[test]
fn oversized_pet_fits_even_a_tiny_work_area_and_remains_recoverable() {
    let area = monitor("tiny", -25., 5., 80., 90., 2.);
    let fit = clamp_placement(
        WindowPlacement {
            x: 400.,
            y: -700.,
            scale: 3.,
            monitor_id: None,
        },
        &[area],
    );
    assert_eq!((fit.x, fit.y, fit.scale), (0., 0., 0.25));
}

#[test]
fn missing_or_transient_zero_sized_work_area_keeps_the_last_valid_scale() {
    let current = WindowPlacement {
        x: 12.0,
        y: 40.0,
        scale: 1.5,
        monitor_id: Some("recovering".into()),
    };
    assert_eq!(
        clamp_placement(
            current.clone(),
            &[monitor("recovering", 0., 0., 0., 0., 2.)]
        ),
        current
    );
    assert_eq!(clamp_placement(current.clone(), &[]), current);
}
