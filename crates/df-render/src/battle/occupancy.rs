//! A coarse occupancy of a scan's geometry around the arena: which cubes of space hold solid
//! splats (trunks, branches, foliage, walls). The resting camera uses it to keep a clear line from
//! the lens to every figure and to the field, so nothing large and out of focus stands between
//! them. It is authored with the `splat_occupancy` example from the scan's own splats at the level
//! in use, and stored in the scene file as a bit set (hexadecimal text), so the browser needs no
//! splat data on the CPU.

use super::math::Vec3;

/// The most voxels a scene file may carry (a bound on the file, not a feature).
pub const MAX_VOXELS: u64 = 600_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Occupancy {
    origin: Vec3,
    cell_m: f32,
    dims: [u32; 3],
    bits: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OccupancyError {
    Dims,
    Hex,
    Length,
}

impl Occupancy {
    /// An empty occupancy over `dims` cells of `cell_m` metres starting at `origin`.
    pub fn empty(origin: Vec3, cell_m: f32, dims: [u32; 3]) -> Result<Self, OccupancyError> {
        let count = u64::from(dims[0]) * u64::from(dims[1]) * u64::from(dims[2]);
        if count == 0 || count > MAX_VOXELS || !cell_m.is_finite() || cell_m <= 0.0 {
            return Err(OccupancyError::Dims);
        }
        Ok(Self {
            origin,
            cell_m,
            dims,
            bits: vec![0; count.div_ceil(8) as usize],
        })
    }

    /// From the scene file's hexadecimal text (two digits per byte, voxel 0 in the lowest bit).
    pub fn from_hex(
        origin: Vec3,
        cell_m: f32,
        dims: [u32; 3],
        hex: &str,
    ) -> Result<Self, OccupancyError> {
        let mut occupancy = Self::empty(origin, cell_m, dims)?;
        if hex.len() != occupancy.bits.len() * 2 {
            return Err(OccupancyError::Length);
        }
        for (byte, pair) in occupancy.bits.iter_mut().zip(hex.as_bytes().chunks(2)) {
            let digit = |c: u8| {
                (c as char)
                    .to_digit(16)
                    .map(|value| value as u8)
                    .ok_or(OccupancyError::Hex)
            };
            *byte = digit(pair[0])? << 4 | digit(pair[1])?;
        }
        Ok(occupancy)
    }

    pub fn to_hex(&self) -> String {
        self.bits.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    /// The same map moved by `by`: for a revision of a scene that places the same scan a
    /// little differently (the grid moved toward a landmark).
    pub fn moved(mut self, by: Vec3) -> Self {
        self.origin = self.origin + by;
        self
    }

    pub fn cell_m(&self) -> f32 {
        self.cell_m
    }

    pub fn dims(&self) -> [u32; 3] {
        self.dims
    }

    fn index(&self, x: u32, y: u32, z: u32) -> usize {
        ((y * self.dims[2] + z) * self.dims[0] + x) as usize
    }

    pub fn set(&mut self, x: u32, y: u32, z: u32) {
        if x < self.dims[0] && y < self.dims[1] && z < self.dims[2] {
            let index = self.index(x, y, z);
            self.bits[index / 8] |= 1 << (index % 8);
        }
    }

    pub fn is_set(&self, x: u32, y: u32, z: u32) -> bool {
        if x >= self.dims[0] || y >= self.dims[1] || z >= self.dims[2] {
            return false;
        }
        let index = self.index(x, y, z);
        self.bits[index / 8] & (1 << (index % 8)) != 0
    }

    /// The cell holding `point`, or `None` outside the covered volume.
    pub fn cell_of(&self, point: Vec3) -> Option<(u32, u32, u32)> {
        let local = point - self.origin;
        if local.x < 0.0 || local.y < 0.0 || local.z < 0.0 {
            return None;
        }
        let cell = |value: f32, size: u32| {
            let index = (value / self.cell_m) as u32;
            (index < size).then_some(index)
        };
        Some((
            cell(local.x, self.dims[0])?,
            cell(local.y, self.dims[1])?,
            cell(local.z, self.dims[2])?,
        ))
    }

    /// Whether the straight line from `from` to `to` passes through no solid cube above
    /// `min_height_m` (the ground itself is not an obstruction), ignoring the last `ignore_end_m`
    /// of it (the figure's own surroundings).
    pub fn segment_clear(
        &self,
        from: Vec3,
        to: Vec3,
        ignore_end_m: f32,
        min_height_m: f32,
    ) -> bool {
        let delta = to - from;
        let length = delta.length();
        if length < 1e-4 {
            return true;
        }
        let direction = delta * (1.0 / length);
        let step = self.cell_m * 0.5;
        let mut along = 0.0;
        while along < length - ignore_end_m {
            let point = from + direction * along;
            if point.y >= min_height_m
                && let Some((x, y, z)) = self.cell_of(point)
                && self.is_set(x, y, z)
            {
                return false;
            }
            along += step;
        }
        true
    }

    /// How many cells are solid, for reports.
    pub fn solid_cells(&self) -> usize {
        self.bits
            .iter()
            .map(|byte| byte.count_ones() as usize)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume() -> Occupancy {
        let mut occupancy =
            Occupancy::empty(Vec3::new(-10.0, 0.0, -10.0), 1.0, [20, 10, 20]).unwrap();
        // A trunk 2 m wide at x = 0..1, z = -5..-4, from the ground to 8 m.
        for y in 0..8 {
            occupancy.set(10, y, 5);
        }
        occupancy
    }

    #[test]
    fn hex_round_trips_and_bad_text_is_refused() {
        let occupancy = volume();
        let hex = occupancy.to_hex();
        let back = Occupancy::from_hex(occupancy.origin(), 1.0, occupancy.dims(), &hex).unwrap();
        assert_eq!(back, occupancy);
        assert_eq!(
            Occupancy::from_hex(occupancy.origin(), 1.0, occupancy.dims(), "zz").unwrap_err(),
            OccupancyError::Length
        );
        let bad = "g".repeat(hex.len());
        assert_eq!(
            Occupancy::from_hex(occupancy.origin(), 1.0, occupancy.dims(), &bad).unwrap_err(),
            OccupancyError::Hex
        );
    }

    #[test]
    fn a_line_through_a_trunk_is_blocked_and_one_beside_it_is_clear() {
        let occupancy = volume();
        let eye = Vec3::new(0.5, 3.0, 8.0);
        assert!(!occupancy.segment_clear(eye, Vec3::new(0.5, 1.0, -9.0), 0.5, 0.9));
        assert!(occupancy.segment_clear(eye, Vec3::new(6.5, 1.0, -9.0), 0.5, 0.9));
    }

    #[test]
    fn the_ground_and_the_end_of_the_line_are_not_obstructions() {
        let occupancy = volume();
        // Below the minimum height the trunk's base does not count; and a figure standing against
        // the trunk is not hidden by it because the last metre of the line is ignored.
        assert!(occupancy.segment_clear(
            Vec3::new(0.5, 0.5, 8.0),
            Vec3::new(0.5, 0.5, -9.0),
            0.5,
            0.9
        ));
        assert!(occupancy.segment_clear(
            Vec3::new(0.5, 3.0, 8.0),
            Vec3::new(0.5, 3.0, -4.6),
            1.5,
            0.9
        ));
    }
}
