use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Deserialize)]
pub struct AlphaMask {
    pub width: u32,
    pub height: u32,
    /// 行和半开像素区间 [y, left, right]，包含等比绘制时的透明边缘。
    pub runs: Vec<[u32; 3]>,
}

impl AlphaMask {
    pub fn validate(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= 8192
            && self.height <= 8192
            && self.runs.len() <= 1_000_000
            && self
                .runs
                .iter()
                .all(|&[y, l, r]| y < self.height && l < r && r <= self.width)
            && self.runs.windows(2).all(|pair| {
                pair[0][0] < pair[1][0] || (pair[0][0] == pair[1][0] && pair[0][2] <= pair[1][1])
            })
    }
}

#[cfg(any(test, target_os = "windows"))]
impl AlphaMask {
    pub(crate) fn region_data(&self, w: i32, h: i32, (dx, dy): (i32, i32)) -> Vec<i32> {
        // RGNDATAHEADER 为 8 个 32 位字段，随后是按行排序、互不重叠的 RECT。
        let mut data = vec![32, 1, 0, 0, i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        let x = |v| dx + (f64::from(v) * f64::from(w) / f64::from(self.width)).round() as i32;
        let y = |v| dy + (f64::from(v) * f64::from(h) / f64::from(self.height)).round() as i32;
        for &[row, left, right] in &self.runs {
            let (left, right, top, bottom) = (x(left), x(right), y(row), y(row + 1));
            if left < right && top < bottom {
                data[4] = data[4].min(left);
                data[5] = data[5].min(top);
                data[6] = data[6].max(right);
                data[7] = data[7].max(bottom);
                data.extend([left, top, right, bottom]);
            }
        }
        data[2] = (data.len() as i32 - 8) / 4;
        data[3] = data[2] * 16;
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_region_keeps_gaps_and_accounts_for_dpi_and_nonclient_offset() {
        let mask = AlphaMask {
            width: 4,
            height: 2,
            runs: vec![[0, 0, 1], [0, 2, 4], [1, 2, 3]],
        };
        assert!(mask.validate());
        let data = mask.region_data(6, 4, (8, 30));
        assert_eq!(
            data,
            [
                32, 1, 3, 48, 8, 30, 14, 34, 8, 30, 10, 32, 11, 30, 14, 32, 11, 32, 13, 34
            ]
        );
    }
    #[test]
    fn unordered_rows_and_outside_extents_are_rejected() {
        for runs in [
            vec![[1, 0, 1], [0, 1, 2]],
            vec![[0, 1, 3], [0, 2, 4]],
            vec![[0, 2, 5]],
            vec![[2, 0, 2]],
        ] {
            assert!(
                !AlphaMask {
                    width: 4,
                    height: 2,
                    runs
                }
                .validate()
            );
        }
    }
}

const MAX_BUBBLE_BUTTONS: usize = 32;
const MAX_BUBBLE_BACKDROPS: usize = 8;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BubbleAction {
    Session,
    Application,
    Copy,
    Close,
    PetFeed,
    PetPlay,
    PetNotifications,
}

#[cfg(any(test, target_os = "macos"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BubbleHit {
    pub(crate) stamp: u32,
    pub(crate) bubble: u32,
    pub(crate) action: BubbleAction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BubbleInput {
    #[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
    stamp: u32,
    buttons: Vec<BubbleButton>,
    #[serde(default)]
    backdrop: Option<BubbleBackdrop>,
    #[serde(default)]
    backdrops: Vec<BubbleBackdrop>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct BubbleBackdrop {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    #[serde(default)]
    pub(crate) radius: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BubbleButton {
    #[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
    #[serde(default)]
    bubble: u32,
    #[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
    action: BubbleAction,
    rect: ButtonRect,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ButtonRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl BubbleInput {
    pub(crate) fn validate(&self, mask: &AlphaMask) -> bool {
        let valid_rect = |x: f64, y: f64, width: f64, height: f64| {
            x.is_finite()
                && y.is_finite()
                && width.is_finite()
                && height.is_finite()
                && x >= 0.0
                && y >= 0.0
                && width > 0.0
                && height > 0.0
                && x + width <= f64::from(mask.width)
                && y + height <= f64::from(mask.height)
        };
        let valid_backdrop = |b: &BubbleBackdrop| {
            valid_rect(b.x, b.y, b.width, b.height) && b.radius.is_finite() && b.radius >= 0.0
        };
        (1..=MAX_BUBBLE_BUTTONS).contains(&self.buttons.len())
            && self.backdrops.len() <= MAX_BUBBLE_BACKDROPS
            && self
                .buttons
                .iter()
                .all(|button| valid_rect(button.rect.x, button.rect.y, button.rect.width, button.rect.height))
            && self.backdrop.as_ref().is_none_or(valid_backdrop)
            && self.backdrops.iter().all(valid_backdrop)
    }

    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn backdrops(&self) -> Vec<BubbleBackdrop> {
        if !self.backdrops.is_empty() {
            self.backdrops.clone()
        } else if let Some(backdrop) = self.backdrop {
            vec![backdrop]
        } else {
            Vec::new()
        }
    }

    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn contains_backdrop(&self, x: f64, y: f64) -> bool {
        let contains = |r: &BubbleBackdrop| {
            x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
        };
        if !self.backdrops.is_empty() {
            self.backdrops.iter().any(contains)
        } else {
            self.backdrop.as_ref().is_some_and(contains)
        }
    }

    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn action_at(&self, x: f64, y: f64) -> Option<BubbleHit> {
        self.buttons.iter().find_map(|button| {
            let r = &button.rect;
            (x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height).then_some(BubbleHit {
                stamp: self.stamp,
                bubble: button.bubble,
                action: button.action,
            })
        })
    }
}

#[cfg(test)]
mod bubble_tests {
    use super::*;

    #[test]
    fn bubble_uses_local_scaled_button_bounds_and_excludes_right_bottom() {
        let input: BubbleInput = serde_json::from_value(serde_json::json!({
            "stamp": 7,
            "buttons": [{"action":"copy","rect":{"x":10.5,"y":22.0,"width":48.0,"height":16.0}}]
        }))
        .unwrap();
        let mask = AlphaMask {
            width: 320,
            height: 360,
            runs: vec![],
        };
        assert!(input.validate(&mask));
        assert_eq!(
            input.action_at(11.0, 22.0),
            Some(BubbleHit {
                stamp: 7,
                bubble: 0,
                action: BubbleAction::Copy,
            })
        );
        assert_eq!(input.action_at(58.5, 22.0), None);
        assert_eq!(input.action_at(11.0, 38.0), None);
        let outside = AlphaMask { width: 40, ..mask };
        assert!(!input.validate(&outside));

        let with_close: BubbleInput = serde_json::from_value(serde_json::json!({
            "stamp": 8,
            "buttons": [
                {"action":"close","rect":{"x":280.0,"y":6.0,"width":20.0,"height":20.0}},
                {"action":"session","rect":{"x":10.0,"y":50.0,"width":60.0,"height":20.0}},
                {"action":"application","rect":{"x":74.0,"y":50.0,"width":60.0,"height":20.0}},
                {"action":"copy","rect":{"x":138.0,"y":50.0,"width":56.0,"height":20.0}}
            ]
        }))
        .unwrap();
        assert!(with_close.validate(&AlphaMask {
            width: 320,
            height: 360,
            runs: vec![],
        }));
        assert_eq!(
            with_close.action_at(290.0, 12.0),
            Some(BubbleHit {
                stamp: 8,
                bubble: 0,
                action: BubbleAction::Close,
            })
        );
    }

    #[test]
    fn bubble_backdrop_validates_bounds_and_radius() {
        let mask = AlphaMask {
            width: 480,
            height: 640,
            runs: vec![],
        };
        let valid: BubbleInput = serde_json::from_value(serde_json::json!({
            "stamp": 9,
            "buttons": [{"action":"close","rect":{"x":424.0,"y":120.0,"width":36.0,"height":36.0}}],
            "backdrop": {"x":12.0,"y":108.0,"width":456.0,"height":160.0,"radius":28.0}
        }))
        .unwrap();
        assert!(valid.validate(&mask));
        assert_eq!(
            valid.backdrops(),
            vec![BubbleBackdrop {
                x: 12.0,
                y: 108.0,
                width: 456.0,
                height: 160.0,
                radius: 28.0,
            }]
        );

        let negative_radius: BubbleInput = serde_json::from_value(serde_json::json!({
            "stamp": 10,
            "buttons": [{"action":"close","rect":{"x":10.0,"y":10.0,"width":20.0,"height":20.0}}],
            "backdrop": {"x":10.0,"y":10.0,"width":100.0,"height":60.0,"radius":-1.0}
        }))
        .unwrap();
        assert!(!negative_radius.validate(&mask));

        let out_of_bounds: BubbleInput = serde_json::from_value(serde_json::json!({
            "stamp": 11,
            "buttons": [{"action":"close","rect":{"x":10.0,"y":10.0,"width":20.0,"height":20.0}}],
            "backdrop": {"x":400.0,"y":10.0,"width":100.0,"height":60.0,"radius":14.0}
        }))
        .unwrap();
        assert!(!out_of_bounds.validate(&mask));
    }

    #[test]
    fn interaction_bubble_actions_and_backdrop_hit_test_work() {
        let mask = AlphaMask {
            width: 480,
            height: 640,
            runs: vec![],
        };
        let input: BubbleInput = serde_json::from_value(serde_json::json!({
            "stamp": 12,
            "buttons": [
                {"action":"close","rect":{"x":424.0,"y":120.0,"width":36.0,"height":36.0}},
                {"action":"pet_feed","rect":{"x":24.0,"y":204.0,"width":80.0,"height":40.0}},
                {"action":"pet_play","rect":{"x":112.0,"y":204.0,"width":80.0,"height":40.0}},
                {"action":"pet_notifications","rect":{"x":200.0,"y":204.0,"width":80.0,"height":40.0}}
            ],
            "backdrop": {"x":12.0,"y":108.0,"width":456.0,"height":160.0,"radius":28.0}
        }))
        .unwrap();
        assert!(input.validate(&mask));
        assert_eq!(
            input.action_at(40.0, 210.0),
            Some(BubbleHit {
                stamp: 12,
                bubble: 0,
                action: BubbleAction::PetFeed,
            })
        );
        assert_eq!(
            input.action_at(130.0, 210.0),
            Some(BubbleHit {
                stamp: 12,
                bubble: 0,
                action: BubbleAction::PetPlay,
            })
        );
        assert_eq!(
            input.action_at(220.0, 210.0),
            Some(BubbleHit {
                stamp: 12,
                bubble: 0,
                action: BubbleAction::PetNotifications,
            })
        );
        assert!(input.contains_backdrop(12.0, 108.0));
        assert!(input.contains_backdrop(400.0, 250.0));
        assert!(!input.contains_backdrop(468.0, 108.0));
        assert!(!input.contains_backdrop(240.0, 420.0));
    }

    #[test]
    fn stacked_bubbles_route_actions_by_bubble_index_and_hit_test_each_backdrop() {
        let mask = AlphaMask {
            width: 480,
            height: 960,
            runs: vec![],
        };
        let input: BubbleInput = serde_json::from_value(serde_json::json!({
            "stamp": 15,
            "buttons": [
                {"bubble":0,"action":"close","rect":{"x":424.0,"y":430.0,"width":36.0,"height":36.0}},
                {"bubble":0,"action":"session","rect":{"x":24.0,"y":510.0,"width":100.0,"height":36.0}},
                {"bubble":1,"action":"close","rect":{"x":424.0,"y":260.0,"width":36.0,"height":36.0}},
                {"bubble":1,"action":"pet_feed","rect":{"x":24.0,"y":340.0,"width":80.0,"height":36.0}}
            ],
            "backdrops": [
                {"x":12.0,"y":420.0,"width":456.0,"height":150.0,"radius":28.0},
                {"x":12.0,"y":250.0,"width":456.0,"height":150.0,"radius":28.0}
            ]
        }))
        .unwrap();
        assert!(input.validate(&mask));
        assert_eq!(input.backdrops().len(), 2);
        assert_eq!(
            input.action_at(430.0, 440.0),
            Some(BubbleHit {
                stamp: 15,
                bubble: 0,
                action: BubbleAction::Close,
            })
        );
        assert_eq!(
            input.action_at(430.0, 270.0),
            Some(BubbleHit {
                stamp: 15,
                bubble: 1,
                action: BubbleAction::Close,
            })
        );
        assert_eq!(
            input.action_at(40.0, 350.0),
            Some(BubbleHit {
                stamp: 15,
                bubble: 1,
                action: BubbleAction::PetFeed,
            })
        );
        assert!(input.contains_backdrop(100.0, 460.0));
        assert!(input.contains_backdrop(100.0, 300.0));
        // Gap between the two stacked bubbles is outside both backdrops
        assert!(!input.contains_backdrop(100.0, 410.0));
    }
}
