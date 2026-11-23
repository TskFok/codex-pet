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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BubbleAction {
    Session,
    Application,
    Copy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BubbleInput {
    #[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
    stamp: u32,
    buttons: Vec<BubbleButton>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BubbleButton {
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
        (1..=3).contains(&self.buttons.len())
            && self.buttons.iter().all(|button| {
                let r = &button.rect;
                r.x >= 0.0
                    && r.y >= 0.0
                    && r.width > 0.0
                    && r.height > 0.0
                    && r.x + r.width <= f64::from(mask.width)
                    && r.y + r.height <= f64::from(mask.height)
            })
    }

    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn action_at(&self, x: f64, y: f64) -> Option<(u32, BubbleAction)> {
        self.buttons.iter().find_map(|button| {
            let r = &button.rect;
            (x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
                .then_some((self.stamp, button.action))
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
        assert_eq!(input.action_at(11.0, 22.0), Some((7, BubbleAction::Copy)));
        assert_eq!(input.action_at(58.5, 22.0), None);
        assert_eq!(input.action_at(11.0, 38.0), None);
        let outside = AlphaMask { width: 40, ..mask };
        assert!(!input.validate(&outside));
    }
}
