use super::length::{Angle, Length, LengthPercentage};

#[derive(Debug, Clone, PartialEq)]
pub enum TransformFunction {
    Translate(LengthPercentage, LengthPercentage),
    TranslateX(LengthPercentage),
    TranslateY(LengthPercentage),
    Scale(f64, f64),
    ScaleX(f64),
    ScaleY(f64),
    Rotate(Angle),
    SkewX(Angle),
    SkewY(Angle),
    Matrix(f64, f64, f64, f64, f64, f64),
    None,
}
