/// 测试角色的实体命中区；输入为窗口客户区逻辑坐标。
pub fn hits_pet(x: f64, y: f64) -> bool {
    ((x - 80.0) / 52.0).powi(2) + ((y - 84.0) / 60.0).powi(2) <= 1.0
}

#[cfg(test)]
mod tests {
    use super::hits_pet;

    #[test]
    fn solid_body_accepts_drag_but_transparent_edges_pass_through() {
        assert!(hits_pet(80.0, 84.0));
        assert!(hits_pet(80.0, 25.0));
        assert!(!hits_pet(10.0, 10.0));
        assert!(!hits_pet(135.0, 84.0));
        assert!(!hits_pet(f64::NAN, 84.0));
    }
}

#[derive(Default)]
pub struct DragTracker {
    down: bool,
    anchor: Option<((f64, f64), (i32, i32))>,
}

impl DragTracker {
    pub fn update(
        &mut self,
        cursor: (f64, f64),
        down: bool,
        hit: bool,
        origin: (i32, i32),
    ) -> Option<(i32, i32)> {
        if down && !self.down && hit {
            self.anchor = Some((cursor, origin));
        }
        self.down = down;
        if !down {
            self.anchor = None;
        }
        self.anchor.map(|(start, origin)| {
            (
                origin.0 + (cursor.0 - start.0).round() as i32,
                origin.1 + (cursor.1 - start.1).round() as i32,
            )
        })
    }
}

#[cfg(test)]
mod drag_tests {
    use super::DragTracker;

    #[test]
    fn dragging_keeps_cursor_anchor_and_stops_on_release() {
        let mut drag = DragTracker::default();
        assert_eq!(
            drag.update((100.0, 100.0), true, true, (20, 30)),
            Some((20, 30))
        );
        assert_eq!(
            drag.update((200.0, 150.0), true, false, (20, 30)),
            Some((120, 80))
        );
        assert_eq!(drag.update((210.0, 150.0), false, false, (120, 80)), None);
        assert_eq!(
            drag.update((210.0, 150.0), true, true, (120, 80)),
            Some((120, 80))
        );
    }

    #[test]
    fn pressing_transparent_edge_does_not_capture_when_moving_into_body() {
        let mut drag = DragTracker::default();
        assert_eq!(drag.update((0.0, 0.0), true, false, (20, 30)), None);
        assert_eq!(drag.update((50.0, 50.0), true, true, (20, 30)), None);
    }
}
