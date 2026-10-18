//! 3x4 row-major affine transforms.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine(pub [f32; 12]);

impl Affine {
    pub const IDENTITY: Affine = Affine([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0,
    ]);

    pub fn translation(t: [f32; 3]) -> Affine {
        Affine([
            1.0, 0.0, 0.0, t[0], //
            0.0, 1.0, 0.0, t[1], //
            0.0, 0.0, 1.0, t[2],
        ])
    }

    pub fn scale(s: [f32; 3]) -> Affine {
        Affine([
            s[0], 0.0, 0.0, 0.0, //
            0.0, s[1], 0.0, 0.0, //
            0.0, 0.0, s[2], 0.0,
        ])
    }

    pub fn rot_x(rad: f32) -> Affine {
        let (s, c) = rad.sin_cos();
        Affine([
            1.0, 0.0, 0.0, 0.0, //
            0.0, c, -s, 0.0, //
            0.0, s, c, 0.0,
        ])
    }

    pub fn rot_y(rad: f32) -> Affine {
        let (s, c) = rad.sin_cos();
        Affine([
            c, 0.0, s, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            -s, 0.0, c, 0.0,
        ])
    }

    pub fn rot_z(rad: f32) -> Affine {
        let (s, c) = rad.sin_cos();
        Affine([
            c, -s, 0.0, 0.0, //
            s, c, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ])
    }

    /// `self * rhs`, so `a.mul(b).point(p) == a.point(b.point(p))`.
    pub fn mul(&self, rhs: &Affine) -> Affine {
        let (a, b) = (&self.0, &rhs.0);
        let mut m = [0.0f32; 12];

        for r in 0..3 {
            let (a0, a1, a2, a3) = (a[r * 4], a[r * 4 + 1], a[r * 4 + 2], a[r * 4 + 3]);
            for c in 0..3 {
                m[r * 4 + c] = a0 * b[c] + a1 * b[4 + c] + a2 * b[8 + c];
            }
            m[r * 4 + 3] = a0 * b[3] + a1 * b[7] + a2 * b[11] + a3;
        }

        Affine(m)
    }

    pub fn point(&self, p: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        [
            m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
            m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
            m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
        ]
    }

    /// Linear part only, valid for normals under uniform scale.
    pub fn vector(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.0;
        [
            m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
            m[4] * v[0] + m[5] * v[1] + m[6] * v[2],
            m[8] * v[0] + m[9] * v[1] + m[10] * v[2],
        ]
    }

    pub fn about(pivot: [f32; 3], m: &Affine) -> Affine {
        let neg = [-pivot[0], -pivot[1], -pivot[2]];
        Affine::translation(pivot)
            .mul(m)
            .mul(&Affine::translation(neg))
    }
}

/// Bedrock Euler degrees (file space) to an IR rotation: `Rz * Ry * Rx` with Y and Z negated, as in game.
/// The one place the rotation convention lives: x = -90 points an arm forward (+Z).
pub fn bedrock_rotation(deg: [f32; 3]) -> Affine {
    let x = Affine::rot_x(deg[0].to_radians());
    let y = Affine::rot_y(-deg[1].to_radians());
    let z = Affine::rot_z(-deg[2].to_radians());
    z.mul(&y).mul(&x)
}

pub fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len == 0.0 {
        return v;
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
