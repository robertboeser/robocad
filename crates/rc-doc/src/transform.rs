//! Node transforms: translation, Euler rotation (degrees) and per-axis scale.

use glam::{DMat4, DQuat, DVec3, EulerRot};
use serde::{Deserialize, Serialize};

/// Local transform of a node. Applied as scale, then rotation, then translation.
///
/// Rotation is stored as Euler angles in degrees, applied about the world X, then Y, then Z axis
/// (`R = Rz · Ry · Rx`). Euler angles are stored instead of a quaternion so the numbers the user
/// types are the numbers that are kept.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transform {
    pub translation: DVec3,
    pub rotation_deg: DVec3,
    pub scale: DVec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: DVec3::ZERO,
        rotation_deg: DVec3::ZERO,
        scale: DVec3::ONE,
    };

    pub fn from_translation(t: DVec3) -> Self {
        Self {
            translation: t,
            ..Self::IDENTITY
        }
    }

    pub fn rotation(&self) -> DQuat {
        let r = self.rotation_deg;
        DQuat::from_euler(
            EulerRot::ZYX,
            r.z.to_radians(),
            r.y.to_radians(),
            r.x.to_radians(),
        )
    }

    pub fn set_rotation(&mut self, q: DQuat) {
        let (z, y, x) = q.normalize().to_euler(EulerRot::ZYX);
        self.rotation_deg = clean(DVec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees()));
    }

    pub fn matrix(&self) -> DMat4 {
        DMat4::from_scale_rotation_translation(self.scale, self.rotation(), self.translation)
    }

    /// Decomposes a matrix into a transform (shear, if any, is lost).
    pub fn from_matrix(m: &DMat4) -> Self {
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        let mut t = Self {
            translation: clean(translation),
            rotation_deg: DVec3::ZERO,
            scale: clean(scale),
        };
        t.set_rotation(rotation);
        t
    }

    pub fn is_mirroring(&self) -> bool {
        self.scale.x * self.scale.y * self.scale.z < 0.0
    }
}

/// Rounds values that are within floating-point noise of a "nice" number.
fn clean(v: DVec3) -> DVec3 {
    v.map(|c| {
        let r = (c * 1e6).round() / 1e6;
        if (r - c).abs() < 1e-9 { r + 0.0 } else { c }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_round_trip() {
        for r in [
            DVec3::new(30.0, 0.0, 0.0),
            DVec3::new(0.0, 45.0, 0.0),
            DVec3::new(10.0, 20.0, 30.0),
            DVec3::new(-90.0, 0.0, 180.0),
        ] {
            let mut t = Transform {
                rotation_deg: r,
                ..Transform::IDENTITY
            };
            let q = t.rotation();
            t.set_rotation(q);
            assert!(
                t.rotation().angle_between(q) < 1e-9,
                "{r:?} -> {:?}",
                t.rotation_deg
            );
        }
    }

    #[test]
    fn rotation_order_is_x_then_y_then_z() {
        let t = Transform {
            rotation_deg: DVec3::new(90.0, 0.0, 90.0),
            ..Transform::IDENTITY
        };
        // Rx(90) maps Y to Z; then Rz(90) leaves Z alone.
        let p = t.matrix().transform_vector3(DVec3::Y);
        assert!((p - DVec3::Z).length() < 1e-12, "{p:?}");
        // Rx(90) leaves X; Rz(90) maps X to Y.
        let p = t.matrix().transform_vector3(DVec3::X);
        assert!((p - DVec3::Y).length() < 1e-12, "{p:?}");
    }

    #[test]
    fn matrix_round_trip() {
        let t = Transform {
            translation: DVec3::new(1.0, 2.0, 3.0),
            rotation_deg: DVec3::new(10.0, 20.0, 30.0),
            scale: DVec3::new(2.0, 3.0, 4.0),
        };
        let back = Transform::from_matrix(&t.matrix());
        assert!((back.matrix() - t.matrix()).abs_diff_eq(DMat4::ZERO, 1e-9));
    }
}
