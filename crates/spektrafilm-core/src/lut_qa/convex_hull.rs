use super::{diff, distance};
use std::collections::BTreeMap;

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
pub(super) fn volume(points: &[[f64; 3]]) -> Result<f64, String> {
    if points.len() < 4 {
        return Err("gamut hull needs four noncoplanar points".into());
    }
    let a = 0;
    let b = (1..points.len())
        .max_by(|&i, &j| distance(points[i], points[a]).total_cmp(&distance(points[j], points[a])))
        .unwrap();
    let c = (0..points.len())
        .max_by(|&i, &j| {
            dot(
                cross(diff(points[b], points[a]), diff(points[i], points[a])),
                cross(diff(points[b], points[a]), diff(points[i], points[a])),
            )
            .total_cmp(&dot(
                cross(diff(points[b], points[a]), diff(points[j], points[a])),
                cross(diff(points[b], points[a]), diff(points[j], points[a])),
            ))
        })
        .unwrap();
    let normal = cross(diff(points[b], points[a]), diff(points[c], points[a]));
    let d = (0..points.len())
        .max_by(|&i, &j| {
            dot(normal, diff(points[i], points[a]))
                .abs()
                .total_cmp(&dot(normal, diff(points[j], points[a])).abs())
        })
        .unwrap();
    if dot(normal, diff(points[d], points[a])).abs() < 1e-14 {
        return Err("degenerate gamut hull (collapsed output)".into());
    }
    let center =
        std::array::from_fn(|i| (points[a][i] + points[b][i] + points[c][i] + points[d][i]) / 4.);
    let orient = |mut f: [usize; 3]| {
        if dot(
            cross(
                diff(points[f[1]], points[f[0]]),
                diff(points[f[2]], points[f[0]]),
            ),
            diff(center, points[f[0]]),
        ) > 0.
        {
            f.swap(1, 2);
        }
        f
    };
    let mut faces = vec![
        orient([a, b, c]),
        orient([a, d, b]),
        orient([a, c, d]),
        orient([b, d, c]),
    ];
    for (i, p) in points.iter().enumerate() {
        if [a, b, c, d].contains(&i) {
            continue;
        }
        let mut edges: BTreeMap<(usize, usize), (usize, usize)> = BTreeMap::new();
        faces.retain(|f| {
            let n = cross(
                diff(points[f[1]], points[f[0]]),
                diff(points[f[2]], points[f[0]]),
            );
            let visible = dot(n, diff(*p, points[f[0]])) > 1e-12 * dot(n, n).sqrt();
            if visible {
                for (u, v) in [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])] {
                    let k = (u.min(v), u.max(v));
                    if edges.remove(&k).is_none() {
                        edges.insert(k, (u, v));
                    }
                }
            }
            !visible
        });
        for (_, (u, v)) in edges {
            faces.push(orient([u, v, i]));
        }
    }
    Ok(faces
        .iter()
        .map(|f| {
            dot(
                diff(points[f[0]], center),
                cross(diff(points[f[1]], center), diff(points[f[2]], center)),
            ) / 6.
        })
        .sum::<f64>()
        .abs())
}

#[cfg(test)]
mod tests {
    use super::volume;
    #[test]
    fn hull_rejects_collapsed_gamut() {
        assert!(volume(&[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]]).is_err());
        assert!(
            (volume(&[
                [0., 0., 0.],
                [1., 0., 0.],
                [0., 1., 0.],
                [0., 0., 1.],
                [1., 1., 1.]
            ])
            .unwrap()
                - 0.5)
                .abs()
                < 1e-12
        );
    }
}
