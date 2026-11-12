use serde::Deserialize;

/// 测试角色实体区；输入坐标必须来自本次原生/PointerEvent，而不是光标采样。
pub fn hits_pet(x: f64, y: f64) -> bool {
    ((x - 80.0) / 52.0).powi(2) + ((y - 84.0) / 60.0).powi(2) <= 1.0
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerPhase {
    Down,
    Move,
    Up,
}

#[derive(Clone, Copy, Deserialize)]
pub struct PointerInput {
    pub phase: PointerPhase,
    pub x: f64,
    pub y: f64,
    pub button: i16,
}

pub fn dispatch_pointer_input<E>(
    input: PointerInput,
    start_drag: impl FnOnce() -> Result<(), E>,
) -> Result<bool, E> {
    if !matches!(input.phase, PointerPhase::Down)
        || input.button != 0
        || !hits_pet(input.x, input.y)
    {
        return Ok(false);
    }
    start_drag()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn input(phase: PointerPhase, x: f64, y: f64) -> PointerInput {
        PointerInput {
            phase,
            x,
            y,
            button: 0,
        }
    }
    fn accept(event: PointerInput) -> bool {
        dispatch_pointer_input(event, || Ok::<_, ()>(())).unwrap()
    }

    #[test]
    fn body_move_then_corner_down_routes_by_actual_down_point() {
        assert!(!accept(input(PointerPhase::Move, 80.0, 84.0)));
        assert!(!accept(input(PointerPhase::Down, 5.0, 5.0)));
        assert!(accept(input(PointerPhase::Down, 80.0, 84.0)));
    }

    #[test]
    fn corner_down_then_body_move_never_starts_drag() {
        let attempts = Cell::new(0);
        for event in [
            input(PointerPhase::Down, 5.0, 5.0),
            input(PointerPhase::Move, 80.0, 84.0),
            input(PointerPhase::Up, 80.0, 84.0),
        ] {
            let accepted = dispatch_pointer_input(event, || {
                attempts.set(attempts.get() + 1);
                Ok::<_, ()>(())
            })
            .unwrap();
            assert!(!accepted);
        }
        assert_eq!(attempts.get(), 0);
    }

    #[test]
    fn temporary_platform_error_does_not_disable_the_next_real_down() {
        let body = input(PointerPhase::Down, 80.0, 84.0);
        assert_eq!(
            dispatch_pointer_input(body, || Err("临时平台错误")),
            Err("临时平台错误")
        );
        assert_eq!(dispatch_pointer_input(body, || Ok::<_, &str>(())), Ok(true));
    }

    #[test]
    fn invalid_coordinates_and_other_buttons_never_call_native_drag() {
        for event in [
            input(PointerPhase::Down, f64::NAN, 84.0),
            PointerInput {
                button: 1,
                ..input(PointerPhase::Down, 80.0, 84.0)
            },
        ] {
            assert!(!dispatch_pointer_input(event, || Err("不得调用原生拖动")).unwrap());
        }
        assert!(hits_pet(80.0, 25.0));
        assert!(!hits_pet(135.0, 84.0));
    }
}

#[cfg(any(target_os = "windows", test))]
pub mod region {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct BodyRegion {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    pub fn body_region(width: i32, height: i32, offset: (i32, i32)) -> Option<BodyRegion> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let x = |value: f64| offset.0 + (value * f64::from(width) / 160.0).round() as i32;
        let y = |value: f64| offset.1 + (value * f64::from(height) / 180.0).round() as i32;
        Some(BodyRegion {
            left: x(28.0),
            top: y(24.0),
            right: x(132.0),
            bottom: y(144.0),
        })
    }

    #[derive(Default)]
    pub struct RegionUpdate {
        applied: Option<BodyRegion>,
    }
    impl RegionUpdate {
        pub fn apply<E>(
            &mut self,
            requested: BodyRegion,
            operation: impl FnOnce(BodyRegion) -> Result<(), E>,
        ) -> Result<bool, E> {
            if self.applied == Some(requested) {
                return Ok(false);
            }
            operation(requested)?;
            self.applied = Some(requested);
            Ok(true)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn region_accounts_for_physical_dpi_and_client_offset() {
            assert_eq!(
                body_region(320, 360, (8, 30)),
                Some(BodyRegion {
                    left: 64,
                    top: 78,
                    right: 272,
                    bottom: 318
                })
            );
            assert_eq!(body_region(0, 360, (0, 0)), None);
        }
        #[test]
        fn failed_native_region_update_is_retried_not_cached_as_applied() {
            let mut update = RegionUpdate::default();
            let shape = BodyRegion {
                left: 28,
                top: 24,
                right: 132,
                bottom: 144,
            };
            assert_eq!(
                update.apply(shape, |_| Err("临时原生失败")),
                Err("临时原生失败")
            );
            assert_eq!(update.apply(shape, |_| Ok::<_, &str>(())), Ok(true));
            assert_eq!(
                update.apply(shape, |_| Err("重复配置不应调用平台")),
                Ok(false)
            );
        }
    }
}
