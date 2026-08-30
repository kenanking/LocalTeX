use gpui::{Pixels, Point};

pub(crate) const ERASER_RADIUS: f32 = 4.0;

#[derive(Clone, PartialEq)]
pub(crate) struct StrokePt {
    pub(crate) pos: Point<Pixels>,
    pub(crate) t_ms: f32,
}

pub(crate) fn dist_point_seg(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let vx = bx - ax;
    let vy = by - ay;
    let len2 = vx * vx + vy * vy;
    if len2 < 1e-8 {
        let dx = px - ax;
        let dy = py - ay;
        return (dx * dx + dy * dy).sqrt();
    }
    let t = ((px - ax) * vx + (py - ay) * vy) / len2;
    let t = t.clamp(0.0, 1.0);
    let dx = px - (ax + t * vx);
    let dy = py - (ay + t * vy);
    (dx * dx + dy * dy).sqrt()
}

fn pt_xy(p: &StrokePt) -> (f32, f32) {
    (f32::from(p.pos.x), f32::from(p.pos.y))
}

fn point_hit(p: &StrokePt, x: f32, y: f32, radius: f32) -> bool {
    let (px, py) = pt_xy(p);
    let dx = px - x;
    let dy = py - y;
    dx * dx + dy * dy <= radius * radius
}

fn seg_hit(a: &StrokePt, b: &StrokePt, x: f32, y: f32, radius: f32) -> bool {
    let (ax, ay) = pt_xy(a);
    let (bx, by) = pt_xy(b);
    dist_point_seg(x, y, ax, ay, bx, by) <= radius
}

fn line_hits(line: &[StrokePt], x: f32, y: f32, radius: f32) -> bool {
    let n = line.len();
    if n < 2 {
        return false;
    }
    (0..n).any(|i| {
        point_hit(&line[i], x, y, radius)
            || (i > 0 && seg_hit(&line[i - 1], &line[i], x, y, radius))
    })
}

fn flush_fragment(cur: &mut Vec<StrokePt>, out: &mut Vec<Vec<StrokePt>>) {
    if cur.len() >= 2 {
        out.push(std::mem::take(cur));
    } else {
        cur.clear();
    }
}

pub(crate) fn erase_and_split(lines: &mut Vec<Vec<StrokePt>>, x: f32, y: f32, radius: f32) -> bool {
    if !lines.iter().any(|line| line_hits(line, x, y, radius)) {
        return false;
    }
    let mut out: Vec<Vec<StrokePt>> = Vec::new();
    for line in lines.drain(..) {
        let n = line.len();
        if n < 2 {
            continue;
        }
        let drop: Vec<bool> = (0..n).map(|i| point_hit(&line[i], x, y, radius)).collect();
        let mut cur: Vec<StrokePt> = Vec::new();
        for i in 0..n {
            if drop[i] {
                flush_fragment(&mut cur, &mut out);
                continue;
            }
            if i > 0 && !drop[i - 1] && seg_hit(&line[i - 1], &line[i], x, y, radius) {
                flush_fragment(&mut cur, &mut out);
            }
            cur.push(line[i].clone());
        }
        flush_fragment(&mut cur, &mut out);
    }
    *lines = out;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    fn ink(points: &[[f32; 2]]) -> Vec<StrokePt> {
        points
            .iter()
            .enumerate()
            .map(|(i, [x, y])| StrokePt {
                pos: point(px(*x), px(*y)),
                t_ms: i as f32,
            })
            .collect()
    }

    #[test]
    fn dist_to_segment_hits_the_middle() {
        let d = dist_point_seg(5.0, 10.0, 0.0, 0.0, 10.0, 0.0);
        assert!((d - 10.0).abs() < 1e-4);
        assert!(dist_point_seg(5.0, 0.0, 0.0, 0.0, 10.0, 0.0) < 1e-4);
    }

    #[test]
    fn erase_splits_a_stroke_and_drops_short_fragments() {
        let mut lines = vec![ink(&[[0.0, 0.0], [10.0, 0.0], [20.0, 0.0], [30.0, 0.0]])];
        assert!(erase_and_split(&mut lines, 20.0, 0.0, 7.0));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 2);
        assert!((f32::from(lines[0][0].pos.x) - 0.0).abs() < 1e-3);
        assert!((f32::from(lines[0][1].pos.x) - 10.0).abs() < 1e-3);
    }

    #[test]
    fn erase_misses_a_distant_stroke() {
        let mut lines = vec![ink(&[[0.0, 0.0], [10.0, 0.0]])];
        assert!(!erase_and_split(&mut lines, 80.0, 80.0, 7.0));
        assert_eq!(lines[0].len(), 2);
    }

    #[test]
    fn erase_uses_segment_distance_not_only_vertices() {
        let mut lines = vec![ink(&[[0.0, 0.0], [40.0, 0.0]])];
        assert!(erase_and_split(&mut lines, 20.0, 0.0, 7.0));
        assert!(!lines.iter().any(|l| l.len() >= 2));
    }
}
