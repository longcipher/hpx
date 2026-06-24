const SHIFT: i32 = 6;
const ONE_PX: i32 = 1 << SHIFT;
const SCALE_F64: f64 = ONE_PX as f64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct LayoutUnit(i32);

impl LayoutUnit {
    pub const ZERO: LayoutUnit = LayoutUnit(0);

    pub fn from_f64_px(v: f64) -> Self {
        if !v.is_finite() {
            return LayoutUnit::ZERO;
        }
        let scaled = (v * SCALE_F64).round();
        let clamped = scaled.clamp(i32::MIN as f64, i32::MAX as f64) as i32;
        LayoutUnit(clamped)
    }

    pub fn from_taffy_f32(v: f32) -> Self {
        Self::from_f64_px(v as f64)
    }

    pub fn to_f64_px(self) -> f64 {
        (self.0 as f64) / SCALE_F64
    }

    pub fn raw(self) -> i32 {
        self.0
    }
}

impl std::ops::Add for LayoutUnit {
    type Output = LayoutUnit;
    fn add(self, rhs: Self) -> Self::Output {
        LayoutUnit(self.0.saturating_add(rhs.0))
    }
}

impl std::ops::Sub for LayoutUnit {
    type Output = LayoutUnit;
    fn sub(self, rhs: Self) -> Self::Output {
        LayoutUnit(self.0.saturating_sub(rhs.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantizes_to_64ths() {
        let v = LayoutUnit::from_f64_px(1.3);
        assert_eq!(v.to_f64_px(), 1.296875);
        assert_eq!(v.raw(), 83);
    }

    #[test]
    fn integer_pixels_round_trip() {
        let v = LayoutUnit::from_f64_px(100.0);
        assert_eq!(v.to_f64_px(), 100.0);
    }

    #[test]
    fn nan_becomes_zero() {
        let v = LayoutUnit::from_f64_px(f64::NAN);
        assert_eq!(v.raw(), 0);
    }

    #[test]
    fn from_taffy_f32_quantizes() {
        let v = LayoutUnit::from_taffy_f32(1.3);
        assert_eq!(v.to_f64_px(), 1.296875);
    }
}
