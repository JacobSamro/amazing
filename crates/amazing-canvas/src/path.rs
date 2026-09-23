use crate::{Point, Transform};
use std::f64::consts::{PI, TAU};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path2D {
    pub(crate) contours: Vec<Contour>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Contour {
    pub points: Vec<Point>,
    pub closed: bool,
}

impl Path2D {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn from_svg(data: &str) -> Result<Self, SvgPathError> {
        parse_svg(data)
    }
    pub fn add_path(&mut self, other: &Self, transform: Transform) {
        if !transform.finite() {
            return;
        }
        self.contours
            .extend(other.contours.iter().cloned().map(|mut c| {
                for p in &mut c.points {
                    *p = transform.map(*p);
                }
                c
            }));
    }
    pub fn move_to(&mut self, x: f64, y: f64) {
        if x.is_finite() && y.is_finite() {
            self.contours.push(Contour {
                points: vec![Point::new(x, y)],
                closed: false,
            });
        }
    }
    pub fn line_to(&mut self, x: f64, y: f64) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        if self.contours.is_empty() {
            self.move_to(x, y);
        } else {
            if self.contours.last().is_some_and(|c| c.closed) {
                let start = self.contours.last().unwrap().points[0];
                self.contours.push(Contour {
                    points: vec![start],
                    closed: false,
                });
            }
            self.contours
                .last_mut()
                .unwrap()
                .points
                .push(Point::new(x, y));
        }
    }
    pub fn close_path(&mut self) {
        if let Some(c) = self.contours.last_mut() {
            c.closed = true;
        }
    }
    pub fn current_point(&self) -> Option<Point> {
        self.contours.last().and_then(|c| {
            if c.closed {
                c.points.first().copied()
            } else {
                c.points.last().copied()
            }
        })
    }
    pub fn quadratic_curve_to(&mut self, cx: f64, cy: f64, x: f64, y: f64) {
        let Some(start) = self.current_point() else {
            self.move_to(x, y);
            return;
        };
        if ![cx, cy, x, y].iter().all(|v| v.is_finite()) {
            return;
        }
        let span = ((start.x - cx).hypot(start.y - cy) + (cx - x).hypot(cy - y)).ceil();
        let n = ((span / 3.).ceil() as usize).clamp(8, 256);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1. - t;
            self.line_to(
                u * u * start.x + 2. * u * t * cx + t * t * x,
                u * u * start.y + 2. * u * t * cy + t * t * y,
            );
        }
    }
    pub fn bezier_curve_to(&mut self, c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64) {
        let Some(start) = self.current_point() else {
            self.move_to(x, y);
            return;
        };
        if ![c1x, c1y, c2x, c2y, x, y].iter().all(|v| v.is_finite()) {
            return;
        }
        let span = (start.x - c1x).hypot(start.y - c1y)
            + (c1x - c2x).hypot(c1y - c2y)
            + (c2x - x).hypot(c2y - y);
        let n = ((span / 3.).ceil() as usize).clamp(8, 512);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1. - t;
            self.line_to(
                u * u * u * start.x + 3. * u * u * t * c1x + 3. * u * t * t * c2x + t * t * t * x,
                u * u * u * start.y + 3. * u * u * t * c1y + 3. * u * t * t * c2y + t * t * t * y,
            );
        }
    }
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if ![x, y, w, h].iter().all(|v| v.is_finite()) {
            return;
        }
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close_path();
    }
    pub fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, radii: [f64; 4]) {
        if ![x, y, w, h]
            .iter()
            .chain(radii.iter())
            .all(|v| v.is_finite())
        {
            return;
        }
        let x0 = x.min(x + w);
        let x1 = x.max(x + w);
        let y0 = y.min(y + h);
        let y1 = y.max(y + h);
        let mut r = radii.map(|v| v.max(0.).min((x1 - x0).min(y1 - y0) / 2.));
        if w < 0. {
            r.swap(0, 1);
            r.swap(3, 2);
        }
        if h < 0. {
            r.swap(0, 3);
            r.swap(1, 2);
        }
        self.move_to(x0 + r[0], y0);
        self.line_to(x1 - r[1], y0);
        self.ellipse(x1 - r[1], y0 + r[1], r[1], r[1], 0., -PI / 2., 0., false);
        self.line_to(x1, y1 - r[2]);
        self.ellipse(x1 - r[2], y1 - r[2], r[2], r[2], 0., 0., PI / 2., false);
        self.line_to(x0 + r[3], y1);
        self.ellipse(x0 + r[3], y1 - r[3], r[3], r[3], 0., PI / 2., PI, false);
        self.line_to(x0, y0 + r[0]);
        self.ellipse(
            x0 + r[0],
            y0 + r[0],
            r[0],
            r[0],
            0.,
            PI,
            3. * PI / 2.,
            false,
        );
        self.close_path();
    }
    pub fn arc(&mut self, x: f64, y: f64, r: f64, start: f64, end: f64, ccw: bool) {
        self.ellipse(x, y, r, r, 0., start, end, ccw);
    }
    pub fn ellipse(
        &mut self,
        x: f64,
        y: f64,
        rx: f64,
        ry: f64,
        rotation: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) {
        if ![x, y, rx, ry, rotation, start, end]
            .iter()
            .all(|v| v.is_finite())
            || rx < 0.
            || ry < 0.
        {
            return;
        }
        let mut sweep = end - start;
        if !ccw {
            if sweep >= TAU {
                sweep = TAU;
            } else {
                while sweep < 0. {
                    sweep += TAU;
                }
            }
        } else if sweep <= -TAU {
            sweep = -TAU;
        } else {
            while sweep > 0. {
                sweep -= TAU;
            }
        }
        let n = ((sweep.abs() * rx.max(ry) / 3.).ceil() as usize).clamp(1, 512);
        let (sr, cr) = rotation.sin_cos();
        for i in 0..=n {
            let t = start + sweep * i as f64 / n as f64;
            let p = Point::new(
                x + rx * t.cos() * cr - ry * t.sin() * sr,
                y + rx * t.cos() * sr + ry * t.sin() * cr,
            );
            if i == 0 && self.current_point().is_none() {
                self.move_to(p.x, p.y);
            } else {
                self.line_to(p.x, p.y);
            }
        }
    }
    pub fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, r: f64) {
        if ![x1, y1, x2, y2, r].iter().all(|v| v.is_finite()) || r < 0. {
            return;
        }
        let Some(p) = self.current_point() else {
            self.move_to(x1, y1);
            return;
        };
        let a = Point::new(x1, y1);
        let b = Point::new(x2, y2);
        let u = Point::new(p.x - a.x, p.y - a.y);
        let v = Point::new(b.x - a.x, b.y - a.y);
        let lu = u.x.hypot(u.y);
        let lv = v.x.hypot(v.y);
        if lu < 1e-9 || lv < 1e-9 || r == 0. {
            self.line_to(x1, y1);
            return;
        }
        let u = Point::new(u.x / lu, u.y / lu);
        let v = Point::new(v.x / lv, v.y / lv);
        let cross = u.x * v.y - u.y * v.x;
        let dot = (u.x * v.x + u.y * v.y).clamp(-1., 1.);
        if cross.abs() < 1e-9 {
            self.line_to(x1, y1);
            return;
        }
        let d = r / (dot.acos() / 2.).tan();
        let t1 = Point::new(a.x + u.x * d, a.y + u.y * d);
        let t2 = Point::new(a.x + v.x * d, a.y + v.y * d);
        let normal = if cross < 0. {
            Point::new(u.y, -u.x)
        } else {
            Point::new(-u.y, u.x)
        };
        let center = Point::new(t1.x + normal.x * r, t1.y + normal.y * r);
        self.line_to(t1.x, t1.y);
        self.arc(
            center.x,
            center.y,
            r,
            (t1.y - center.y).atan2(t1.x - center.x),
            (t2.y - center.y).atan2(t2.x - center.x),
            cross > 0.,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgPathError {
    InvalidCommand,
    InvalidNumber,
}

fn tokens(data: &str) -> Result<Vec<String>, SvgPathError> {
    let b = data.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        if b[i].is_ascii_whitespace() || b[i] == b',' {
            i += 1;
            continue;
        }
        if b[i].is_ascii_alphabetic() {
            out.push((b[i] as char).to_string());
            i += 1;
            continue;
        }
        let start = i;
        if b[i] == b'+' || b[i] == b'-' {
            i += 1;
        }
        let mut digits = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(SvgPathError::InvalidNumber);
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            let e = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == e {
                return Err(SvgPathError::InvalidNumber);
            }
        }
        out.push(data[start..i].to_string());
        if out.len() > 100_000 {
            return Err(SvgPathError::InvalidCommand);
        }
    }
    Ok(out)
}

fn parse_svg(data: &str) -> Result<Path2D, SvgPathError> {
    let t = tokens(data)?;
    let mut i = 0;
    let mut command = ' ';
    let mut path = Path2D::new();
    let mut last_cubic: Option<Point> = None;
    let mut last_quad: Option<Point> = None;
    while i < t.len() {
        if t[i].len() == 1 && t[i].as_bytes()[0].is_ascii_alphabetic() {
            command = t[i].chars().next().unwrap();
            i += 1;
            if command == 'Z' || command == 'z' {
                path.close_path();
                last_cubic = None;
                last_quad = None;
                command = ' ';
                continue;
            }
        }
        if command == ' ' {
            return Err(SvgPathError::InvalidCommand);
        }
        let relative = command.is_ascii_lowercase();
        let upper = command.to_ascii_uppercase();
        let n = match upper {
            'M' | 'L' | 'T' => 2,
            'H' | 'V' => 1,
            'Q' | 'S' => 4,
            'C' => 6,
            'A' => 7,
            _ => return Err(SvgPathError::InvalidCommand),
        };
        if i + n > t.len() {
            return Err(SvgPathError::InvalidCommand);
        }
        let mut v = Vec::with_capacity(n);
        for s in &t[i..i + n] {
            v.push(s.parse::<f64>().map_err(|_| SvgPathError::InvalidNumber)?);
        }
        i += n;
        let p = path.current_point().unwrap_or_default();
        let dx = if relative { p.x } else { 0. };
        let dy = if relative { p.y } else { 0. };
        match upper {
            'M' => {
                path.move_to(v[0] + dx, v[1] + dy);
                command = if relative { 'l' } else { 'L' };
            }
            'L' => path.line_to(v[0] + dx, v[1] + dy),
            'H' => path.line_to(v[0] + dx, p.y),
            'V' => path.line_to(p.x, v[0] + dy),
            'Q' => {
                path.quadratic_curve_to(v[0] + dx, v[1] + dy, v[2] + dx, v[3] + dy);
                last_quad = Some(Point::new(v[0] + dx, v[1] + dy));
                last_cubic = None;
            }
            'T' => {
                let c = last_quad.map_or(p, |q| Point::new(2. * p.x - q.x, 2. * p.y - q.y));
                path.quadratic_curve_to(c.x, c.y, v[0] + dx, v[1] + dy);
                last_quad = Some(c);
                last_cubic = None;
            }
            'C' => {
                path.bezier_curve_to(
                    v[0] + dx,
                    v[1] + dy,
                    v[2] + dx,
                    v[3] + dy,
                    v[4] + dx,
                    v[5] + dy,
                );
                last_cubic = Some(Point::new(v[2] + dx, v[3] + dy));
                last_quad = None;
            }
            'S' => {
                let c = last_cubic.map_or(p, |q| Point::new(2. * p.x - q.x, 2. * p.y - q.y));
                path.bezier_curve_to(c.x, c.y, v[0] + dx, v[1] + dy, v[2] + dx, v[3] + dy);
                last_cubic = Some(Point::new(v[0] + dx, v[1] + dy));
                last_quad = None;
            }
            'A' => {
                svg_arc(
                    &mut path,
                    p,
                    v[0],
                    v[1],
                    v[2].to_radians(),
                    v[3] != 0.,
                    v[4] != 0.,
                    Point::new(v[5] + dx, v[6] + dy),
                );
                last_cubic = None;
                last_quad = None;
            }
            _ => unreachable!(),
        }
        if !matches!(upper, 'Q' | 'T' | 'C' | 'S') {
            last_quad = None;
            last_cubic = None;
        }
    }
    Ok(path)
}

fn svg_arc(
    path: &mut Path2D,
    start: Point,
    rx: f64,
    ry: f64,
    rotation: f64,
    large: bool,
    sweep: bool,
    end: Point,
) {
    let (rx, ry) = (rx.abs(), ry.abs());
    if rx == 0. || ry == 0. {
        path.line_to(end.x, end.y);
        return;
    }
    let (s, c) = rotation.sin_cos();
    let dx = (start.x - end.x) / 2.;
    let dy = (start.y - end.y) / 2.;
    let x = c * dx + s * dy;
    let y = -s * dx + c * dy;
    let scale = (x * x / (rx * rx) + y * y / (ry * ry)).sqrt().max(1.);
    let rx = rx * scale;
    let ry = ry * scale;
    let numerator = (rx * rx * ry * ry - rx * rx * y * y - ry * ry * x * x).max(0.);
    let denominator = rx * rx * y * y + ry * ry * x * x;
    let q = if denominator == 0. {
        0.
    } else {
        (numerator / denominator).sqrt() * if large == sweep { -1. } else { 1. }
    };
    let cx = q * rx * y / ry;
    let cy = -q * ry * x / rx;
    let center = Point::new(
        c * cx - s * cy + (start.x + end.x) / 2.,
        s * cx + c * cy + (start.y + end.y) / 2.,
    );
    let a = ((y - cy) / ry).atan2((x - cx) / rx);
    let b = ((-y - cy) / ry).atan2((-x - cx) / rx);
    path.ellipse(center.x, center.y, rx, ry, rotation, a, b, !sweep);
}
