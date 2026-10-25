//! Scan collision, ported from PlayCanvas (MIT): splat-transform
//! 45931669547a7c70cbc44d2657749f8a56e5533c, src/lib/gpu/gpu-voxelization.ts and
//! src/lib/writers/sparse-octree.ts; supersplat-viewer
//! cd0e316e397929ffa87abd4833f73f8a0bd1fd2b, src/collision/voxel-collision.ts.
//! Gaussian density at the nearest voxel point accumulates by Beer-Lambert extinction.
//! Sparse octants end in 4x4x4 bit masks. Capsule queries use segment/voxel-box distance.
//! Unlike the viewer we only report contact, never move an authoritative actor. No scan
//! voxels are serialized: authoring emits cells, steps and ground samples only.

use super::{math::Vec3, splat::Splat};

const MAX_VOXELS: usize = 8_000_000;
const MAX_NODES: usize = 300_000;
const MAX_SPLATS: u64 = 10_000_000;
const MAX_EVALUATIONS: u64 = 500_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColliderError {
    Bounds,
    Capacity,
    Splat,
    Capsule,
}

/// Bounded authoring accumulator: <=32 MB density, <=500 million evaluations,
/// <=10 million splats. It accepts chunks so decoded scans need not coexist in memory.
pub struct VoxelBuilder {
    origin: Vec3,
    cell_m: f32,
    dims: [u32; 3],
    density: Vec<f32>,
    splats: u64,
    evaluations: u64,
}

impl VoxelBuilder {
    pub fn new(origin: Vec3, cell_m: f32, dims: [u32; 3]) -> Result<Self, ColliderError> {
        let voxel_count = dims
            .iter()
            .try_fold(1usize, |n, d| n.checked_mul(*d as usize));
        if !origin.is_finite()
            || !cell_m.is_finite()
            || !(0.025..=0.25).contains(&cell_m)
            || dims.contains(&0)
            || dims.iter().any(|d| *d > 512)
        {
            return Err(ColliderError::Bounds);
        }
        let voxel_count = voxel_count
            .filter(|n| *n <= MAX_VOXELS)
            .ok_or(ColliderError::Capacity)?;
        Ok(Self {
            origin,
            cell_m,
            dims,
            density: vec![0.0; voxel_count],
            splats: 0,
            evaluations: 0,
        })
    }

    pub fn add(&mut self, splats: &[Splat]) -> Result<(), ColliderError> {
        self.splats += splats.len() as u64;
        if self.splats > MAX_SPLATS {
            return Err(ColliderError::Capacity);
        }
        for splat in splats {
            if !splat.centre.is_finite() || splat.covariance.iter().any(|v| !v.is_finite()) {
                return Err(ColliderError::Splat);
            }
            let opacity = f32::from(splat.colour[3]) / 255.0;
            // Discard faint floaters; dense opaque surface samples still accumulate together.
            if opacity < 0.1 {
                continue;
            }
            let mut covariance = splat.covariance.map(f64::from);
            if [covariance[0], covariance[3], covariance[5]]
                .into_iter()
                .any(|v| v <= 0.0)
            {
                return Err(ColliderError::Splat);
            }
            // Scan surface Gaussians can be nearly rank two. f32 covariance rounding can
            // erase the thinnest axis; a 1% voxel-width diagonal restores its inverse
            // without expanding geometry by even one voxel.
            let variance = f64::from(self.cell_m * 0.01).powi(2);
            for axis in [0, 3, 5] {
                covariance[axis] += variance;
            }
            let [xx, xy, xz, yy, yz, zz] = covariance;
            let det =
                xx * (yy * zz - yz * yz) - xy * (xy * zz - xz * yz) + xz * (xy * yz - xz * yy);
            if det <= 0.0 || xx <= 0.0 || yy <= 0.0 || zz <= 0.0 {
                return Err(ColliderError::Splat);
            }
            let inverse = [
                yy * zz - yz * yz,
                xz * yz - xy * zz,
                xy * yz - xz * yy,
                xx * zz - xz * xz,
                xy * xz - xx * yz,
                xx * yy - xy * xy,
            ]
            .map(|v| v / det);
            let centre = [splat.centre.x, splat.centre.y, splat.centre.z];
            let origin = [self.origin.x, self.origin.y, self.origin.z];
            let extent = [xx, yy, zz].map(|v| 3.0 * v.sqrt() as f32);
            let mut low = [0u32; 3];
            let mut high = [0u32; 3];
            let mut outside = false;
            for axis in 0..3 {
                let first = ((centre[axis] - extent[axis] - origin[axis]) / self.cell_m).floor();
                let last = ((centre[axis] + extent[axis] - origin[axis]) / self.cell_m).floor();
                outside |= last < 0.0 || first >= self.dims[axis] as f32;
                low[axis] = first.max(0.0) as u32;
                high[axis] = (last.max(0.0) as u32).min(self.dims[axis] - 1);
            }
            if outside {
                continue;
            }
            let evaluation_count = (0..3)
                .map(|a| u64::from(high[a] - low[a] + 1))
                .product::<u64>();
            self.evaluations += evaluation_count;
            if self.evaluations > MAX_EVALUATIONS {
                return Err(ColliderError::Capacity);
            }
            for z in low[2]..=high[2] {
                for y in low[1]..=high[1] {
                    for x in low[0]..=high[0] {
                        let mut diff = [0.0f64; 3];
                        for (axis, index) in [x, y, z].into_iter().enumerate() {
                            let min = origin[axis] + index as f32 * self.cell_m;
                            diff[axis] = f64::from(
                                centre[axis].clamp(min, min + self.cell_m) - centre[axis],
                            );
                        }
                        let [dx, dy, dz] = diff;
                        let d2 = inverse[0] * dx * dx
                            + inverse[3] * dy * dy
                            + inverse[5] * dz * dz
                            + 2.0
                                * (inverse[1] * dx * dy
                                    + inverse[2] * dx * dz
                                    + inverse[4] * dy * dz);
                        let density_index = ((z * self.dims[1] + y) * self.dims[0] + x) as usize;
                        self.density[density_index] += opacity * (-0.5 * d2.max(0.0)).exp() as f32;
                    }
                }
            }
        }
        Ok(())
    }

    /// Surface shell only: exterior flood fill/carving would seal deliberate outdoor gateways.
    pub fn finish(self) -> Result<VoxelCollider, ColliderError> {
        let side = self
            .dims
            .into_iter()
            .max()
            .ok_or(ColliderError::Bounds)?
            .max(4)
            .next_power_of_two();
        let mut collider = VoxelCollider {
            origin: self.origin,
            cell_m: self.cell_m,
            dims: self.dims,
            side,
            nodes: vec![Node::default()],
        };
        for z in (0..self.dims[2]).step_by(4) {
            for y in (0..self.dims[1]).step_by(4) {
                for x in (0..self.dims[0]).step_by(4) {
                    let mut mask = 0u64;
                    for dz in 0..4 {
                        for dy in 0..4 {
                            for dx in 0..4 {
                                if x + dx >= self.dims[0]
                                    || y + dy >= self.dims[1]
                                    || z + dz >= self.dims[2]
                                {
                                    continue;
                                }
                                let at = (((z + dz) * self.dims[1] + y + dy) * self.dims[0]
                                    + x
                                    + dx) as usize;
                                // 1-exp(-density) >= 0.3, PlayCanvas's extinction rule.
                                if self.density[at] >= -(0.7f32).ln() {
                                    mask |= 1 << (dz * 16 + dy * 4 + dx);
                                }
                            }
                        }
                    }
                    if mask != 0 {
                        collider.insert([x, y, z], mask)?;
                    }
                }
            }
        }
        Ok(collider)
    }
}

#[derive(Clone, Debug, Default)]
struct Node {
    children: [u32; 8],
    mask: u64,
}

/// <=300,000 nodes (12 MB payload). Empty octants allocate nothing; each leaf stores 64 voxels.
#[derive(Clone, Debug)]
pub struct VoxelCollider {
    origin: Vec3,
    cell_m: f32,
    dims: [u32; 3],
    side: u32,
    nodes: Vec<Node>,
}

impl VoxelCollider {
    fn insert(&mut self, cell: [u32; 3], mask: u64) -> Result<(), ColliderError> {
        let mut node = 0usize;
        let mut size = self.side;
        while size > 4 {
            size /= 2;
            let octant = ((cell[0] & size != 0) as usize)
                | (((cell[1] & size != 0) as usize) << 1)
                | (((cell[2] & size != 0) as usize) << 2);
            if self.nodes[node].children[octant] == 0 {
                if self.nodes.len() >= MAX_NODES {
                    return Err(ColliderError::Capacity);
                }
                self.nodes[node].children[octant] = self.nodes.len() as u32;
                self.nodes.push(Node::default());
            }
            node = self.nodes[node].children[octant] as usize;
        }
        self.nodes[node].mask = mask;
        Ok(())
    }

    fn solid(&self, cell: [u32; 3]) -> bool {
        if (0..3).any(|a| cell[a] >= self.dims[a]) {
            return false;
        }
        let mut node = 0usize;
        let mut size = self.side;
        while size > 4 {
            size /= 2;
            let octant = ((cell[0] & size != 0) as usize)
                | (((cell[1] & size != 0) as usize) << 1)
                | (((cell[2] & size != 0) as usize) << 2);
            node = self.nodes[node].children[octant] as usize;
            if node == 0 {
                return false;
            }
        }
        self.nodes[node].mask & (1 << ((cell[2] & 3) * 16 + (cell[1] & 3) * 4 + (cell[0] & 3))) != 0
    }

    pub fn touches_capsule(
        &self,
        from: Vec3,
        to: Vec3,
        radius: f32,
    ) -> Result<bool, ColliderError> {
        if !from.is_finite()
            || !to.is_finite()
            || !(to - from).is_finite()
            || !radius.is_finite()
            || !(0.0..=10.0).contains(&radius)
        {
            return Err(ColliderError::Capsule);
        }
        Ok(self.touches_node(0, [0; 3], self.side, from, to, radius * radius))
    }

    fn touches_node(
        &self,
        node: usize,
        cell: [u32; 3],
        size: u32,
        from: Vec3,
        to: Vec3,
        radius_sq: f32,
    ) -> bool {
        let min =
            self.origin + Vec3::new(cell[0] as f32, cell[1] as f32, cell[2] as f32) * self.cell_m;
        let max = min + Vec3::new(1.0, 1.0, 1.0) * (size as f32 * self.cell_m);
        if segment_box_distance_sq(from, to, min, max) > radius_sq {
            return false;
        }
        let entry = &self.nodes[node];
        if size == 4 {
            let mut mask = entry.mask;
            while mask != 0 {
                let bit = mask.trailing_zeros();
                mask &= mask - 1;
                let min = min
                    + Vec3::new((bit & 3) as f32, ((bit >> 2) & 3) as f32, (bit >> 4) as f32)
                        * self.cell_m;
                let max = min + Vec3::new(1.0, 1.0, 1.0) * self.cell_m;
                if segment_box_distance_sq(from, to, min, max) <= radius_sq {
                    return true;
                }
            }
            return false;
        }
        let half = size / 2;
        entry.children.iter().enumerate().any(|(octant, child)| {
            *child != 0
                && self.touches_node(
                    *child as usize,
                    [
                        cell[0] + (octant as u32 & 1) * half,
                        cell[1] + ((octant as u32 >> 1) & 1) * half,
                        cell[2] + ((octant as u32 >> 2) & 1) * half,
                    ],
                    half,
                    from,
                    to,
                    radius_sq,
                )
        })
    }

    /// Highest solid surface below `ceiling_y`, down to `floor_y`. A ground hint confines
    /// the search to the ground layer rather than mistaking a canopy for the floor.
    pub fn ground_height(&self, x: f32, z: f32, floor_y: f32, ceiling_y: f32) -> Option<f32> {
        if ![x, z, floor_y, ceiling_y].into_iter().all(f32::is_finite) || floor_y > ceiling_y {
            return None;
        }
        let col = ((x - self.origin.x) / self.cell_m).floor();
        let row = ((z - self.origin.z) / self.cell_m).floor();
        if col < 0.0 || row < 0.0 || col >= self.dims[0] as f32 || row >= self.dims[2] as f32 {
            return None;
        }
        (0..self.dims[1]).rev().find_map(|y| {
            let top = self.origin.y + (y + 1) as f32 * self.cell_m;
            (top <= ceiling_y && top >= floor_y && self.solid([col as u32, y, row as u32]))
                .then_some(top)
        })
    }

    /// Highest solid surface above `ground_y` in this voxel column, measured from the local floor.
    pub fn solid_height_above_ground(
        &self,
        x: f32,
        z: f32,
        ground_y: f32,
        search_height_m: f32,
    ) -> Option<f32> {
        if ![x, z, ground_y, search_height_m]
            .into_iter()
            .all(f32::is_finite)
            || search_height_m <= self.cell_m
        {
            return None;
        }
        self.ground_height(x, z, ground_y + self.cell_m, ground_y + search_height_m)
            .map(|top| top - ground_y)
    }
}

// Squared distance of a segment to an AABB. Between face-crossing times the squared
// point-box distance is quadratic; check its minimum and interval endpoints exactly.
fn segment_box_distance_sq(from: Vec3, to: Vec3, min: Vec3, max: Vec3) -> f32 {
    let start = [from.x, from.y, from.z];
    let delta = [to.x - from.x, to.y - from.y, to.z - from.z];
    let low = [min.x, min.y, min.z];
    let high = [max.x, max.y, max.z];
    let mut times = [0.0f32; 8];
    times[1] = 1.0;
    let mut count = 2;
    for axis in 0..3 {
        if delta[axis] != 0.0 {
            for face in [low[axis], high[axis]] {
                let time = (face - start[axis]) / delta[axis];
                if time > 0.0 && time < 1.0 {
                    times[count] = time;
                    count += 1;
                }
            }
        }
    }
    times[..count].sort_by(f32::total_cmp);
    let distance = |time: f32| {
        (0..3)
            .map(|axis| {
                let p = start[axis] + delta[axis] * time;
                let d = p - p.clamp(low[axis], high[axis]);
                d * d
            })
            .sum::<f32>()
    };
    let mut best = f32::INFINITY;
    for pair in times[..count].windows(2) {
        let middle = (pair[0] + pair[1]) * 0.5;
        let mut quadratic = 0.0;
        let mut linear = 0.0;
        for axis in 0..3 {
            let p = start[axis] + delta[axis] * middle;
            if p < low[axis] || p > high[axis] {
                let face = if p < low[axis] { low[axis] } else { high[axis] };
                quadratic += delta[axis] * delta[axis];
                linear += delta[axis] * (start[axis] - face);
            }
        }
        best = best.min(distance(pair[0])).min(distance(pair[1]));
        if quadratic > 0.0 {
            best = best.min(distance((-linear / quadratic).clamp(pair[0], pair[1])));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    fn splat(x: f32, y: f32, z: f32, scale: [f32; 3], opacity: u8) -> Splat {
        Splat {
            centre: Vec3::new(x, y, z),
            colour: [255, 255, 255, opacity],
            covariance: [
                scale[0] * scale[0],
                0.0,
                0.0,
                scale[1] * scale[1],
                0.0,
                scale[2] * scale[2],
            ],
        }
    }
    fn build(splats: &[Splat]) -> VoxelCollider {
        let mut builder =
            VoxelBuilder::new(Vec3::new(-2.0, -0.5, -2.0), 0.1, [40, 50, 40]).unwrap();
        builder.add(splats).unwrap();
        builder.finish().unwrap()
    }
    #[test]
    fn a_gaussian_wall_blocks_a_crossing_capsule_but_open_ground_does_not() {
        let wall: Vec<_> = (0..8)
            .flat_map(|y| {
                (0..12).map(move |z| {
                    splat(
                        0.0,
                        y as f32 * 0.25,
                        z as f32 * 0.25 - 1.5,
                        [0.04, 0.15, 0.15],
                        255,
                    )
                })
            })
            .collect();
        let collider = build(&wall);
        assert!(
            collider
                .touches_capsule(Vec3::new(-1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), 0.35)
                .unwrap()
        );
        assert!(
            !collider
                .touches_capsule(Vec3::new(-1.0, 0.6, 0.0), Vec3::new(-1.0, 1.35, 0.0), 0.35)
                .unwrap()
        );
        assert!(
            !build(&[])
                .touches_capsule(Vec3::new(-1.0, 1.0, 0.0), Vec3::new(1.0, 1.0, 0.0), 0.35)
                .unwrap()
        );
    }
    #[test]
    fn low_grass_high_canopy_and_a_faint_floater_do_not_block_a_standing_figure() {
        let collider = build(&[
            splat(0.0, 0.03, 0.0, [0.3, 0.01, 0.3], 255),
            splat(0.0, 3.0, 0.0, [0.3, 0.1, 0.3], 255),
            splat(0.0, 1.0, 0.0, [0.2; 3], 10),
        ]);
        assert!(
            !collider
                .touches_capsule(Vec3::new(0.0, 0.6, 0.0), Vec3::new(0.0, 1.35, 0.0), 0.35)
                .unwrap()
        );
        assert!(collider.ground_height(0.0, 0.0, -0.2, 0.2).is_some());
        assert!(collider.ground_height(1.5, 1.5, -0.2, 0.2).is_none());
    }

    #[test]
    fn solid_height_query_measures_barriers_above_the_local_ground() {
        let ground = splat(0.0, 0.03, 0.0, [0.5, 0.03, 0.5], 255);
        let floor = build(&[ground]);
        let ground_y = floor.ground_height(0.0, 0.0, -0.2, 0.2).unwrap();
        assert_eq!(
            floor.solid_height_above_ground(0.0, 0.0, ground_y, 2.0),
            None
        );

        let wall: Vec<_> = (0..12)
            .map(|index| splat(0.0, 0.2 + index as f32 * 0.1, 0.0, [0.2; 3], 255))
            .collect();
        let collider = build(&[ground].into_iter().chain(wall).collect::<Vec<_>>());
        let ground_y = collider.ground_height(0.0, 0.0, -0.2, 0.2).unwrap();
        let height = collider
            .solid_height_above_ground(0.0, 0.0, ground_y, 2.0)
            .unwrap();
        assert!(height > 0.8, "measured barrier height {height}");
    }
    #[test]
    fn contact_is_exact_at_box_corners_and_capsule_ends() {
        let min = Vec3::new(0.0, 0.0, 0.0);
        let max = Vec3::new(1.0, 1.0, 1.0);
        assert!(
            (segment_box_distance_sq(
                Vec3::new(-1.0, -1.0, 0.5),
                Vec3::new(-1.0, 2.0, 0.5),
                min,
                max
            ) - 1.0)
                .abs()
                < 1e-5
        );
        assert_eq!(
            segment_box_distance_sq(
                Vec3::new(-1.0, 0.5, 0.5),
                Vec3::new(2.0, 0.5, 0.5),
                min,
                max
            ),
            0.0
        );
    }
    #[test]
    fn excessive_volume_and_invalid_splats_are_refused() {
        assert!(matches!(
            VoxelBuilder::new(Vec3::ZERO, 0.1, [512; 3]),
            Err(ColliderError::Capacity)
        ));
        let mut builder = VoxelBuilder::new(Vec3::ZERO, 0.1, [4; 3]).unwrap();
        assert_eq!(
            builder.add(&[splat(0.0, 0.0, 0.0, [0.0; 3], 255)]),
            Err(ColliderError::Splat)
        );
    }
}
