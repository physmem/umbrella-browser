/// A rectangle in GPUI logical pixels, relative to the window's client area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Device pixels, which is what `ICoreWebView2Controller::SetBounds` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl LogicalRect {
    /// Rounds each edge, not the size, so two panes that share a logical edge
    /// also share a device-pixel edge at 125% or 150% scale: no gap, no overlap.
    pub fn to_physical(self, scale: f32) -> PhysicalRect {
        let px = |v: f32| (v * scale).round() as i32;
        PhysicalRect {
            left: px(self.x),
            top: px(self.y),
            right: px(self.x + self.width).max(px(self.x)),
            bottom: px(self.y + self.height).max(px(self.y)),
        }
    }
}

#[cfg(windows)]
impl From<PhysicalRect> for windows::Win32::Foundation::RECT {
    fn from(r: PhysicalRect) -> Self {
        Self {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_panes_share_an_edge_at_fractional_scale() {
        let left = LogicalRect {
            x: 0.0,
            y: 40.0,
            width: 333.3,
            height: 600.0,
        };
        let right = LogicalRect {
            x: 333.3,
            y: 40.0,
            width: 333.4,
            height: 600.0,
        };
        for scale in [1.0, 1.25, 1.5, 1.75] {
            assert_eq!(left.to_physical(scale).right, right.to_physical(scale).left);
        }
    }

    #[test]
    fn scales_and_never_inverts() {
        let r = LogicalRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: -5.0,
        };
        let p = r.to_physical(1.5);
        assert_eq!((p.left, p.top, p.right), (15, 30, 165));
        assert_eq!(p.bottom, p.top);
    }
}
