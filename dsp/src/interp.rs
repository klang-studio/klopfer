//! Interpolation helpers.

/// 4-point, 3rd-order Hermite interpolation between `x0` and `x1` at fraction `t` in `[0, 1)`.
#[inline]
pub fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c = (x1 - xm1) * 0.5;
    let v = x0 - x1;
    let w = c + v;
    let a = w + v + (x2 - x0) * 0.5;
    let b = w + a;
    (((a * t) - b) * t + c) * t + x0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_sample_points() {
        assert_eq!(hermite(0.0, 1.0, 2.0, 3.0, 0.0), 1.0);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 0.999_999) - 2.0).abs() < 1e-4);
    }

    #[test]
    fn linear_data_stays_linear() {
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 0.5) - 1.5).abs() < 1e-6);
    }
}
