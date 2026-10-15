//! CPU f64 inverse of the sampled physical scanner response.
//!
//! Regular density cells share a Freudenthal triangulation. Their signed
//! receiver coordinates are interpolated without pixel-level nonlinear solves.
//! Exterior samples use the nearest physical boundary and its raw-capture tangent.

#[derive(Clone, Copy, Debug)]
pub(super) struct CoupledBounds {
    pub domain_min: [f64; 3],
    pub domain_max: [f64; 3],
}

type V3 = [f64; 3];

#[derive(Clone, Copy, Debug)]
struct Box3 {
    min: V3,
    max: V3,
}
impl Box3 {
    fn points(points: &[V3]) -> Self {
        let mut result = Self {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        };
        for p in points {
            for (c, &value) in p.iter().enumerate() {
                result.min[c] = result.min[c].min(value);
                result.max[c] = result.max[c].max(value);
            }
        }
        result
    }
    fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|c| self.min[c].min(other.min[c])),
            max: std::array::from_fn(|c| self.max[c].max(other.max[c])),
        }
    }
    fn contains(&self, point: V3) -> bool {
        (0..3).all(|c| point[c] >= self.min[c] - 1e-12 && point[c] <= self.max[c] + 1e-12)
    }
    fn distance2(&self, point: V3) -> f64 {
        (0..3)
            .map(|c| (point[c] - point[c].clamp(self.min[c], self.max[c])).powi(2))
            .sum()
    }
}

#[derive(Clone, Debug)]
struct Tree {
    bounds: Box3,
    children: Option<(usize, usize)>,
    start: usize,
    end: usize,
}

#[derive(Clone, Debug)]
struct Index {
    nodes: Vec<Tree>,
    items: Vec<usize>,
}
impl Index {
    fn prepare(boxes: &[Box3]) -> Self {
        let mut index = Self {
            nodes: Vec::new(),
            items: (0..boxes.len()).collect(),
        };
        index.build(boxes, 0, boxes.len());
        index
    }
    fn build(&mut self, boxes: &[Box3], start: usize, end: usize) -> usize {
        let bounds = self.items[start + 1..end]
            .iter()
            .fold(boxes[self.items[start]], |b, &i| b.union(boxes[i]));
        let node = self.nodes.len();
        self.nodes.push(Tree {
            bounds,
            children: None,
            start,
            end,
        });
        if end - start > 8 {
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (bounds.max[a] - bounds.min[a]).total_cmp(&(bounds.max[b] - bounds.min[b]))
                })
                .unwrap();
            self.items[start..end].sort_unstable_by(|&a, &b| {
                (boxes[a].min[axis] + boxes[a].max[axis])
                    .total_cmp(&(boxes[b].min[axis] + boxes[b].max[axis]))
                    .then(a.cmp(&b))
            });
            let mid = start + (end - start) / 2;
            let left = self.build(boxes, start, mid);
            let right = self.build(boxes, mid, end);
            self.nodes[node].children = Some((left, right));
        }
        node
    }
}

#[derive(Clone, Debug)]
struct Tetra {
    origin: V3,
    transform: [V3; 3],
    density: V3,
    edges: [V3; 3],
}
impl Tetra {
    fn weights(&self, point: V3) -> V3 {
        let delta = sub(point, self.origin);
        self.transform.map(|row| dot(row, delta))
    }
    fn density(&self, point: V3) -> V3 {
        let w = self.weights(point);
        std::array::from_fn(|c| {
            self.density[c] + (0..3).map(|j| self.edges[j][c] * w[j]).sum::<f64>()
        })
    }
    fn contains(&self, point: V3) -> bool {
        let w = self.weights(point);
        w.iter().all(|&v| v >= -1e-9) && w.iter().sum::<f64>() <= 1.0 + 1e-9
    }
}

#[derive(Clone, Debug)]
struct Face {
    points: [V3; 3],
    vertices: [usize; 3],
    density: [V3; 3],
    jacobian: [[V3; 3]; 3],
}

#[derive(Clone, Debug)]
pub(super) struct CoupledInverse {
    scale: V3,
    tetras: Vec<Tetra>,
    faces: Vec<Face>,
    interior: Index,
    boundary: Index,
}

impl CoupledInverse {
    pub(super) fn prepare<F>(bounds: CoupledBounds, n: usize, forward: F) -> Result<Self, String>
    where
        F: Fn(V3) -> V3,
    {
        if !(2..=65).contains(&n) {
            return Err("coupled inverse resolution must be in 2..=65".into());
        }
        if (0..3).any(|c| {
            !bounds.domain_min[c].is_finite()
                || !bounds.domain_max[c].is_finite()
                || bounds.domain_min[c] >= bounds.domain_max[c]
        }) {
            return Err("coupled inverse requires finite increasing density bounds".into());
        }
        let point = |r: usize, g: usize, b: usize| {
            let i = [r, g, b];
            std::array::from_fn(|c| {
                bounds.domain_min[c]
                    + (bounds.domain_max[c] - bounds.domain_min[c]) * i[c] as f64 / (n - 1) as f64
            })
        };
        let index = |r: usize, g: usize, b: usize| (r * n + g) * n + b;
        let mut raw = Vec::with_capacity(n * n * n);
        let mut scale = [0.0f64; 3];
        for r in 0..n {
            for g in 0..n {
                for b in 0..n {
                    let capture = forward(point(r, g, b));
                    if capture.iter().any(|v| !v.is_finite()) {
                        return Err(format!("coupled capture is nonfinite at [{r},{g},{b}]"));
                    }
                    for c in 0..3 {
                        scale[c] = scale[c].max(capture[c].abs());
                    }
                    raw.push(capture);
                }
            }
        }
        if scale.iter().any(|&v| v <= 0.0 || !v.is_finite()) {
            return Err("coupled receiver has an unusable capture scale".into());
        }
        let coordinates: Vec<V3> = raw
            .iter()
            .map(|p| std::array::from_fn(|c| (p[c] / scale[c]).asinh()))
            .collect();
        let patterns = [
            [0, 1, 3, 7],
            [0, 1, 5, 7],
            [0, 2, 3, 7],
            [0, 2, 6, 7],
            [0, 4, 5, 7],
            [0, 4, 6, 7],
        ];
        let mut tetras = Vec::with_capacity(6 * (n - 1).pow(3));
        let mut faces = Vec::new();
        let mut vertex_jacobians = vec![[[0.0; 3]; 3]; coordinates.len()];
        let mut vertex_counts = vec![0usize; coordinates.len()];
        let mut boxes = Vec::with_capacity(tetras.capacity());
        let mut orientation = None;
        for r in 0..n - 1 {
            for g in 0..n - 1 {
                for b in 0..n - 1 {
                    let corners = std::array::from_fn::<_, 8, _>(|bit| {
                        [
                            r + (bit & 1 != 0) as usize,
                            g + (bit & 2 != 0) as usize,
                            b + (bit & 4 != 0) as usize,
                        ]
                    });
                    for pattern in patterns {
                        let grid = pattern.map(|i| corners[i]);
                        let p = grid.map(|[r, g, b]| coordinates[index(r, g, b)]);
                        let d = grid.map(|[r, g, b]| point(r, g, b));
                        let edges = [sub(p[1], p[0]), sub(p[2], p[0]), sub(p[3], p[0])];
                        let (inverse, det) =
                            invert(edges).ok_or("coupled inverse has a singular physical cell")?;
                        let density_edges = [sub(d[1], d[0]), sub(d[2], d[0]), sub(d[3], d[0])];
                        let sign = (det
                            / dot(density_edges[0], cross(density_edges[1], density_edges[2])))
                        .is_sign_positive();
                        if orientation.is_some_and(|previous| previous != sign) {
                            return Err(
                                "coupled receiver response folds within the density domain".into(),
                            );
                        }
                        orientation = Some(sign);
                        let condition = edges.iter().map(|e| dot(*e, *e)).sum::<f64>().sqrt()
                            * inverse.iter().map(|e| dot(*e, *e)).sum::<f64>().sqrt();
                        if !condition.is_finite() || condition > 1e8 {
                            return Err(
                                "coupled receiver response is too poorly conditioned to invert"
                                    .into(),
                            );
                        }
                        let jacobian: [V3; 3] = std::array::from_fn(|c| {
                            std::array::from_fn(|k| {
                                (0..3).map(|j| density_edges[j][c] * inverse[j][k]).sum()
                            })
                        });
                        for grid in grid {
                            let id = index(grid[0], grid[1], grid[2]);
                            vertex_counts[id] += 1;
                            for c in 0..3 {
                                for k in 0..3 {
                                    vertex_jacobians[id][c][k] += jacobian[c][k];
                                }
                            }
                        }
                        tetras.push(Tetra {
                            origin: p[0],
                            transform: inverse,
                            density: d[0],
                            edges: density_edges,
                        });
                        boxes.push(Box3::points(&p));
                        for indices in [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]] {
                            if (0..3).any(|axis| {
                                indices.iter().all(|&i| grid[i][axis] == 0)
                                    || indices.iter().all(|&i| grid[i][axis] == n - 1)
                            }) {
                                faces.push(Face {
                                    points: indices.map(|i| p[i]),
                                    vertices: indices
                                        .map(|i| index(grid[i][0], grid[i][1], grid[i][2])),
                                    density: indices.map(|i| d[i]),
                                    jacobian: [[[0.0; 3]; 3]; 3],
                                });
                            }
                        }
                    }
                }
            }
        }
        for face in &mut faces {
            for v in 0..3 {
                let id = face.vertices[v];
                face.jacobian[v] =
                    vertex_jacobians[id].map(|row| row.map(|x| x / vertex_counts[id] as f64));
            }
            let [a, b, c] = face.points;
            let ab = sub(b, a);
            let ac = sub(c, a);
            let denominator = dot(ab, ab) * dot(ac, ac) - dot(ab, ac).powi(2);
            if denominator <= 0.0
                || !denominator.is_finite()
                || face
                    .jacobian
                    .iter()
                    .flatten()
                    .flatten()
                    .any(|v| !v.is_finite())
            {
                return Err("coupled receiver boundary tangent is not representable".into());
            }
        }
        let face_boxes: Vec<_> = faces.iter().map(|f| Box3::points(&f.points)).collect();
        let interior = Index::prepare(&boxes);
        let boundary = Index::prepare(&face_boxes);
        Ok(Self {
            scale,
            tetras,
            faces,
            interior,
            boundary,
        })
    }

    #[cfg(test)]
    pub(super) fn estimate_density(&self, capture: V3) -> Result<V3, String> {
        self.estimate_density_with_anchor(capture)
            .map(|result| result.0)
    }

    pub(super) fn estimate_density_with_anchor(
        &self,
        capture: V3,
    ) -> Result<(V3, Option<V3>), String> {
        if capture.iter().any(|v| !v.is_finite()) {
            return Err("coupled scan capture must contain finite RGB values".into());
        }
        let query = std::array::from_fn(|c| {
            let ratio = capture[c] / self.scale[c];
            if ratio.is_finite() {
                ratio.asinh()
            } else {
                capture[c].signum()
                    * (capture[c].abs().ln() - self.scale[c].ln() + std::f64::consts::LN_2)
            }
        });
        if query.iter().any(|v| !v.is_finite()) {
            return Err("coupled scan capture coordinate is not representable".into());
        }
        if let Some(density) = self.find(0, query) {
            return Ok((density, None));
        }
        let mut nearest = (f64::INFINITY, usize::MAX, [0.0; 3]);
        self.nearest(0, query, &mut nearest);
        if nearest.1 == usize::MAX {
            return Err("coupled scan capture has no finite boundary reference".into());
        }
        let face = &self.faces[nearest.1];
        let anchor = nearest.2;
        let [a, b, c] = face.points;
        let ab = sub(b, a);
        let ac = sub(c, a);
        let ap = sub(anchor, a);
        let aa = dot(ab, ab);
        let bb = dot(ac, ac);
        let cc = dot(ab, ac);
        let denominator = aa * bb - cc * cc;
        let v = (bb * dot(ap, ab) - cc * dot(ap, ac)) / denominator;
        let w = (aa * dot(ap, ac) - cc * dot(ap, ab)) / denominator;
        let weights = [1.0 - v - w, v, w];
        let anchor_density =
            std::array::from_fn(|c| (0..3).map(|i| weights[i] * face.density[i][c]).sum::<f64>());
        let delta = std::array::from_fn::<_, 3, _>(|c| {
            let raw_anchor = self.scale[c] * anchor[c].sinh();
            (capture[c] - raw_anchor) / self.scale[c].hypot(raw_anchor)
        });
        let density = std::array::from_fn(|c| {
            anchor_density[c]
                + (0..3)
                    .map(|k| {
                        let derivative = (0..3)
                            .map(|i| weights[i] * face.jacobian[i][c][k])
                            .sum::<f64>();
                        derivative * delta[k]
                    })
                    .sum::<f64>()
        });
        if density.iter().any(|v| !v.is_finite()) {
            return Err("coupled scan endpoint tangent exceeds the finite density range".into());
        }
        Ok((density, Some(anchor_density)))
    }

    fn find(&self, node: usize, query: V3) -> Option<V3> {
        let node = &self.interior.nodes[node];
        if !node.bounds.contains(query) {
            return None;
        }
        if let Some((left, right)) = node.children {
            return self.find(left, query).or_else(|| self.find(right, query));
        }
        for &i in &self.interior.items[node.start..node.end] {
            let tetra = &self.tetras[i];
            if tetra.contains(query) {
                return Some(tetra.density(query));
            }
        }
        None
    }

    fn nearest(&self, node: usize, query: V3, best: &mut (f64, usize, V3)) {
        let node = &self.boundary.nodes[node];
        if node.bounds.distance2(query) > best.0 {
            return;
        }
        if let Some((left, right)) = node.children {
            let a = self.boundary.nodes[left].bounds.distance2(query);
            let b = self.boundary.nodes[right].bounds.distance2(query);
            let order = if a <= b { [left, right] } else { [right, left] };
            for child in order {
                self.nearest(child, query, best);
            }
        } else {
            for &i in &self.boundary.items[node.start..node.end] {
                let anchor = closest_triangle(query, self.faces[i].points);
                let delta = sub(query, anchor);
                let distance = dot(delta, delta);
                if (distance, i) < (best.0, best.1) {
                    *best = (distance, i, anchor);
                }
            }
        }
    }
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: V3, b: V3) -> V3 {
    std::array::from_fn(|c| a[c] - b[c])
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn add_scaled(a: V3, b: V3, scale: f64) -> V3 {
    std::array::from_fn(|c| a[c] + b[c] * scale)
}
fn invert(columns: [V3; 3]) -> Option<([V3; 3], f64)> {
    let cofactors = [
        cross(columns[1], columns[2]),
        cross(columns[2], columns[0]),
        cross(columns[0], columns[1]),
    ];
    let det = dot(columns[0], cofactors[0]);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let inverse = cofactors.map(|row| row.map(|v| v / det));
    inverse
        .iter()
        .flatten()
        .all(|v| v.is_finite())
        .then_some((inverse, det))
}

// Closest point on a triangle, including its vertex and edge Voronoi regions.
fn closest_triangle(p: V3, [a, b, c]: [V3; 3]) -> V3 {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return add_scaled(a, ab, d1 / (d1 - d3));
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return add_scaled(a, ac, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return add_scaled(b, sub(c, b), (d4 - d3) / (d4 - d3 + d5 - d6));
    }
    let denominator = 1.0 / (va + vb + vc);
    add_scaled(add_scaled(a, ab, vb * denominator), ac, vc * denominator)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn affine(d: V3) -> V3 {
        [d[0] + 0.2 * d[1], d[1] + 0.1 * d[2], d[2] + 0.1 * d[0]]
    }
    #[test]
    fn signed_coupled_cells_recover_interior_and_continue_exterior_tangent() {
        let inverse = CoupledInverse::prepare(
            CoupledBounds {
                domain_min: [-1.0; 3],
                domain_max: [1.0; 3],
            },
            17,
            affine,
        )
        .unwrap();
        for density in [[0.13, -0.24, 0.35], [-0.8, 0.4, -0.6], [0.0; 3]] {
            let recovered = inverse.estimate_density(affine(density)).unwrap();
            assert!(
                (0..3).all(|c| (recovered[c] - density[c]).abs() < 0.002),
                "{density:?} -> {recovered:?}"
            );
        }
        for axis in 0..3 {
            for side in [-1.0, 1.0] {
                let mut boundary = [0.0; 3];
                boundary[axis] = side;
                let mut outside = boundary;
                outside[axis] += side * 0.01;
                let (recovered, anchor) = inverse
                    .estimate_density_with_anchor(affine(outside))
                    .unwrap();
                assert!(anchor.is_some());
                assert!(recovered[axis] * side > 1.0);
                assert!((0..3).all(|c| (recovered[c] - outside[c]).abs() < 0.002));
                let mut near = boundary;
                near[axis] += side * 1e-7;
                let recovered = inverse.estimate_density(affine(near)).unwrap();
                assert!((0..3).all(|c| (recovered[c] - boundary[c]).abs() < 1e-6));
            }
        }
    }
    #[test]
    fn rejects_singular_folded_and_nonfinite_responses() {
        let bounds = CoupledBounds {
            domain_min: [-1.0; 3],
            domain_max: [1.0; 3],
        };
        assert!(CoupledInverse::prepare(bounds, 5, |d| [d[0], d[0], d[2]]).is_err());
        assert!(CoupledInverse::prepare(bounds, 6, |d| [d[0] * d[0], d[1], d[2]]).is_err());
        assert!(CoupledInverse::prepare(bounds, 3, |_| [f64::NAN; 3]).is_err());
        let inverse = CoupledInverse::prepare(bounds, 5, affine).unwrap();
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(inverse.estimate_density([invalid, 0.0, 0.0]).is_err());
        }
    }
}
