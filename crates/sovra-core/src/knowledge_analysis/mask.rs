//! Binary morphology with a square structuring element and zero-valued exterior.
//! A summed-area table makes each output pixel constant work at any radius.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mask {
    width: usize,
    height: usize,
    pixels: Vec<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Erode,
    Dilate,
}

impl Mask {
    pub fn new(width: usize, height: usize, pixels: Vec<bool>) -> Result<Self, &'static str> {
        if width == 0
            || height == 0
            || width.max(height) > 4096
            || width.checked_mul(height) != Some(pixels.len())
            || pixels.len() > 1_048_576
        {
            return Err("Use a nonempty binary mask with at most one million pixels.");
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }
    pub fn packed(&self) -> Vec<u8> {
        self.pixels
            .chunks(8)
            .map(|chunk| {
                chunk
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (i, on)| bits | (u8::from(*on) << i))
            })
            .collect()
    }
    pub fn unpack(width: usize, height: usize, bytes: &[u8]) -> Option<Self> {
        let count = width.checked_mul(height)?;
        if count > 1_048_576 || bytes.len() != count.div_ceil(8) {
            return None;
        }
        Self::new(
            width,
            height,
            (0..count)
                .map(|i| bytes[i / 8] & (1 << (i % 8)) != 0)
                .collect(),
        )
        .ok()
    }
    pub fn describe(&self, radius: u8) -> Result<String, &'static str> {
        let foreground = self.foreground();
        let eroded = self.apply(Operation::Erode, radius.into())?.foreground();
        let dilated = self.apply(Operation::Dilate, radius.into())?.foreground();
        let side = usize::from(radius) * 2 + 1;
        Ok(format!(
            "{} × {} binary mask · foreground {foreground} pixels · {side} × {side} square, zero border · eroded {eroded} · dilated {dilated} · inner boundary {} pixels",
            self.width,
            self.height,
            foreground - eroded
        ))
    }
    pub fn foreground(&self) -> usize {
        self.pixels.iter().filter(|p| **p).count()
    }
    pub fn pixels(&self) -> &[bool] {
        &self.pixels
    }
    pub fn apply(&self, op: Operation, radius: usize) -> Result<Self, &'static str> {
        if radius > 64 {
            return Err("Choose a square mask radius between 0 and 64 pixels.");
        }
        let stride = self.width + 1;
        let mut sums = vec![0u32; stride * (self.height + 1)];
        for (y, row) in self.pixels.chunks_exact(self.width).enumerate() {
            let mut sum = 0;
            for (x, &pixel) in row.iter().enumerate() {
                sum += u32::from(pixel);
                sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + sum;
            }
        }
        let area = (2 * radius + 1).pow(2);
        let mut output = Vec::with_capacity(self.pixels.len());
        for y in 0..self.height {
            let top = y.saturating_sub(radius) * stride;
            let bottom = (y + radius + 1).min(self.height) * stride;
            for x in 0..self.width {
                let left = x.saturating_sub(radius);
                let right = (x + radius + 1).min(self.width);
                let sum = (sums[bottom + right] - sums[bottom + left])
                    - (sums[top + right] - sums[top + left]);
                output.push(match op {
                    Operation::Erode => sum as usize == area,
                    Operation::Dilate => sum > 0,
                });
            }
        }
        Ok(Self {
            width: self.width,
            height: self.height,
            pixels: output,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn square_kernel_and_zero_border_return_exact_pixels() {
        let solid = Mask::new(3, 3, vec![true; 9]).unwrap();
        let center = solid.apply(Operation::Erode, 1).unwrap();
        assert_eq!(
            center.pixels(),
            [false, false, false, false, true, false, false, false, false]
        );
        assert_eq!(center.apply(Operation::Dilate, 1).unwrap(), solid);
        assert_eq!(center.apply(Operation::Dilate, 0).unwrap(), center);
        assert!(center.apply(Operation::Erode, 65).is_err());
    }
}
