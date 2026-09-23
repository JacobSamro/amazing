/// Unpremultiplied RGBA image data, as exposed by Canvas 2D's pixel API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    InvalidDimensions,
    InvalidLength,
}

impl ImageData {
    pub fn new(width: u32, height: u32) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::InvalidDimensions);
        }
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|v| v.checked_mul(4))
            .ok_or(ImageError::InvalidDimensions)?;
        Ok(Self {
            width,
            height,
            data: vec![0; size],
        })
    }
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Result<Self, ImageError> {
        let size = (width as usize)
            .checked_mul(height as usize)
            .and_then(|v| v.checked_mul(4))
            .ok_or(ImageError::InvalidDimensions)?;
        if width == 0 || height == 0 {
            return Err(ImageError::InvalidDimensions);
        }
        if data.len() != size {
            return Err(ImageError::InvalidLength);
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Bitmap {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        ImageData::from_rgba(width, height, pixels.clone())?;
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let at = ((y * self.width + x) as usize) * 4;
        self.pixels[at..at + 4].try_into().unwrap()
    }
    pub fn sample(&self, x: f64, y: f64, smooth: bool) -> [u8; 4] {
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.
            || y < 0.
            || x >= self.width as f64
            || y >= self.height as f64
        {
            return [0; 4];
        }
        if !smooth {
            return self.pixel(x.floor() as u32, y.floor() as u32);
        }
        let x = x - 0.5;
        let y = y - 0.5;
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let tx = x - x0 as f64;
        let ty = y - y0 as f64;
        let lookup = |ix: i64, iy: i64| {
            self.pixel(
                ix.clamp(0, self.width as i64 - 1) as u32,
                iy.clamp(0, self.height as i64 - 1) as u32,
            )
        };
        let a = lookup(x0, y0);
        let b = lookup(x0 + 1, y0);
        let c = lookup(x0, y0 + 1);
        let d = lookup(x0 + 1, y0 + 1);
        // Interpolate premultiplied channels to avoid dark fringes at transparent edges.
        let weights = [
            (1. - tx) * (1. - ty),
            tx * (1. - ty),
            (1. - tx) * ty,
            tx * ty,
        ];
        let pixels = [a, b, c, d];
        let alpha = pixels
            .iter()
            .zip(weights)
            .map(|(p, w)| p[3] as f64 * w)
            .sum::<f64>();
        if alpha <= 0. {
            return [0; 4];
        }
        let mut out = [0; 4];
        out[3] = alpha.round().clamp(0., 255.) as u8;
        for k in 0..3 {
            out[k] = (pixels
                .iter()
                .zip(weights)
                .map(|(p, w)| p[k] as f64 * p[3] as f64 * w)
                .sum::<f64>()
                / alpha)
                .round()
                .clamp(0., 255.) as u8;
        }
        out
    }
}
