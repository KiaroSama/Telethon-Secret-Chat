//! Telegram Desktop album geometry and Qt QRect::united. GPL-3.0/LGPL-3.0.
use super::error::{ExportError, Result};
pub type Rect = (i64, i64, i64, i64);
pub type Size = (i64, i64);
fn round(v: f64) -> i64 {
    if v.is_nan() { 0 } else { v.round() as i64 }
}
pub fn layout_media_group_geometry(sizes: &[Size], w: i64, mw: i64, sp: i64) -> Result<Vec<Rect>> {
    if sizes.is_empty() {
        return Ok(vec![]);
    }
    if sizes.iter().any(|(w, h)| *w <= 0 || *h <= 0) {
        return Err(ExportError::InvalidRecord);
    }
    let r = sizes
        .iter()
        .map(|(w, h)| *w as f64 / *h as f64)
        .collect::<Vec<_>>();
    let p = r
        .iter()
        .map(|r| {
            if *r > 1.2 {
                'w'
            } else if *r < 0.8 {
                'n'
            } else {
                'q'
            }
        })
        .collect::<String>();
    let n = r.len();
    let avg = (1.0 + r.iter().sum::<f64>()) / n as f64;
    let h = w;
    let (wf, hf, sf) = (w as f64, h as f64, sp as f64);
    if n == 1 {
        return Ok(vec![(0, 0, w, sizes[0].1 * w / sizes[0].0)]);
    }
    if n >= 5 || r.iter().any(|r| *r > 2.0) {
        return complex(&r, avg, w, mw, sp);
    }
    Ok(if n == 2 {
        if p == "ww" && avg > 1.4 && r[1] - r[0] < 0.2 {
            let height = round((wf / r[0]).min((wf / r[1]).min((hf - sf) / 2.0)));
            vec![(0, 0, w, height), (0, height + sp, w, height)]
        } else if p == "ww" || p == "qq" {
            let width = (w - sp) / 2;
            let height = round((width as f64 / r[0]).min((width as f64 / r[1]).min(hf)));
            vec![(0, 0, width, height), (width + sp, 0, width, height)]
        } else {
            let minimal = round(mw as f64 * 1.5);
            let second = round((0.4 * (wf - sf)).max((wf - sf) / r[0] / (1.0 / r[0] + 1.0 / r[1])))
                .min(w - sp - minimal);
            let first = w - second - sp;
            let height = h.min(round((first as f64 / r[0]).min(second as f64 / r[1])));
            vec![(0, 0, first, height), (first + sp, 0, second, height)]
        }
    } else if n == 3 {
        if p.starts_with('n') {
            let third_h = round(((hf - sf) / 2.0).min(r[1] * (wf - sf) / (r[2] + r[1])));
            let second_h = h - third_h - sp;
            let right = mw.max(round(
                ((wf - sf) / 2.0).min((third_h as f64 * r[2]).min(second_h as f64 * r[1])),
            ));
            let left = round(hf * r[0]).min(w - sp - right);
            vec![
                (0, 0, left, h),
                (left + sp, 0, right, second_h),
                (left + sp, second_h + sp, right, third_h),
            ]
        } else {
            let first_h = round((wf / r[0]).min((hf - sf) * 0.66));
            let second_w = (w - sp) / 2;
            let second_h =
                (h - first_h - sp).min(round((second_w as f64 / r[1]).min(second_w as f64 / r[2])));
            vec![
                (0, 0, w, first_h),
                (0, first_h + sp, second_w, second_h),
                (second_w + sp, first_h + sp, w - second_w - sp, second_h),
            ]
        }
    } else if p.starts_with('w') {
        let h0 = round((wf / r[0]).min((hf - sf) * 0.66));
        let height = round((wf - 2.0 * sf) / (r[1] + r[2] + r[3]));
        let w0 = mw.max(round(((wf - 2.0 * sf) * 0.4).min(height as f64 * r[1])));
        let w2 = round(
            (mw as f64)
                .max((wf - 2.0 * sf) * 0.33)
                .max(height as f64 * r[3]),
        );
        let w1 = w - w0 - w2 - 2 * sp;
        let h1 = (h - h0 - sp).min(height);
        vec![
            (0, 0, w, h0),
            (0, h0 + sp, w0, h1),
            (w0 + sp, h0 + sp, w1, h1),
            (w0 + sp + w1 + sp, h0 + sp, w2, h1),
        ]
    } else {
        let w0 = round((hf * r[0]).min((wf - sf) * 0.6));
        let width = round((hf - 2.0 * sf) / (1.0 / r[1] + 1.0 / r[2] + 1.0 / r[3]));
        let h0 = round(width as f64 / r[1]);
        let h1 = round(width as f64 / r[2]);
        let w1 = mw.max((w - w0 - sp).min(width));
        vec![
            (0, 0, w0, h),
            (w0 + sp, 0, w1, h0),
            (w0 + sp, h0 + sp, w1, h1),
            (w0 + sp, h0 + h1 + 2 * sp, w1, h - h0 - h1 - 2 * sp),
        ]
    })
}
fn complex(source: &[f64], average: f64, w: i64, mw: i64, sp: i64) -> Result<Vec<Rect>> {
    let r = source
        .iter()
        .map(|r| {
            if average > 1.1 {
                r.clamp(1.0, 2.75)
            } else {
                r.clamp(0.6667, 1.0)
            }
        })
        .collect::<Vec<_>>();
    let n = r.len();
    let max_h = w * 4 / 3;
    let mut attempts: Vec<(Vec<usize>, Vec<f64>)> = vec![];
    let mut push = |counts: Vec<usize>| {
        let mut offset = 0;
        let heights = counts
            .iter()
            .map(|line| {
                let height = (w - (*line as i64 - 1) * sp) as f64
                    / r[offset..offset + line].iter().sum::<f64>();
                offset += line;
                height
            })
            .collect();
        attempts.push((counts, heights));
    };
    for a in 1..n {
        let b = n - a;
        if a <= 3 && b <= 3 {
            push(vec![a, b]);
        }
    }
    for a in 1..n - 1 {
        for b in 1..n - a {
            let c = n - a - b;
            if a <= 3 && b <= if average < 0.85 { 4 } else { 3 } && c <= 3 {
                push(vec![a, b, c]);
            }
        }
    }
    for a in 1..n - 1 {
        for b in 1..n - a {
            for c in 1..n - a - b {
                let d = n - a - b - c;
                if a.max(b).max(c).max(d) <= 3 {
                    push(vec![a, b, c, d]);
                }
            }
        }
    }
    let mut optimal = None;
    let mut diff = 0.0;
    for (counts, heights) in attempts {
        let total = heights.iter().sum::<f64>() + sp as f64 * (counts.len() as f64 - 1.0);
        let bad1 = if heights.iter().any(|h| *h < mw as f64) {
            1.5
        } else {
            1.0
        };
        let bad2 = if counts.windows(2).any(|c| c[0] > c[1]) {
            1.5
        } else {
            1.0
        };
        let candidate = (total - max_h as f64).abs() * bad1 * bad2;
        if optimal.is_none() || candidate < diff {
            optimal = Some((counts, heights));
            diff = candidate;
        }
    }
    let Some((counts, heights)) = optimal else {
        return Ok(Vec::new());
    };
    let (mut index, mut y) = (0, 0);
    let mut result = vec![];
    for (columns, line_h) in counts.into_iter().zip(heights) {
        let height = round(line_h);
        let mut x = 0;
        for column in 0..columns {
            let width = if column == columns - 1 {
                w - x
            } else {
                round(r[index] * line_h)
            };
            result.push((x, y, width, height));
            x += width + sp;
            index += 1;
        }
        y += height + sp;
    }
    Ok(result)
}
pub fn united(first: Option<Rect>, second: Rect) -> Rect {
    let Some(first) = first else {
        return second;
    };
    if first.2 == 0 && first.3 == 0 {
        return second;
    }
    if second.2 == 0 && second.3 == 0 {
        return first;
    }
    let span = |start: i64, length: i64| {
        let end = start + length - 1;
        if end - start + 1 < 0 {
            (end, start)
        } else {
            (start, end)
        }
    };
    let (l1, r1) = span(first.0, first.2);
    let (l2, r2) = span(second.0, second.2);
    let (t1, b1) = span(first.1, first.3);
    let (t2, b2) = span(second.1, second.3);
    let (l, r, t, b) = (l1.min(l2), r1.max(r2), t1.min(t2), b1.max(b2));
    (l, t, r - l + 1, b - t + 1)
}
