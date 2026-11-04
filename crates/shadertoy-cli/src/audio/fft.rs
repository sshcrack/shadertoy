//! Minimal radix-2 FFT for magnitude spectra.

use std::f32::consts::PI;

pub struct Fft {
    size: usize,
    bit_reverse: Vec<usize>,
    twiddles: Vec<(f32, f32)>,
    window: Vec<f32>,
}

impl Fft {
    /// FFT of `size` (power of two) using a Blackman window, matching Web Audio's AnalyserNode.
    pub fn new(size: usize) -> Self {
        assert!(
            size.is_power_of_two() && size >= 4,
            "FFT size must be a power of two"
        );
        let bits = size.trailing_zeros();
        let bit_reverse = (0..size)
            .map(|index| index.reverse_bits() >> (usize::BITS - bits))
            .collect();
        let twiddles = (0..size / 2)
            .map(|k| {
                let angle = -2.0 * PI * k as f32 / size as f32;
                (angle.cos(), angle.sin())
            })
            .collect();
        let alpha = 0.16f32;
        let window = (0..size)
            .map(|n| {
                let x = n as f32 / size as f32;
                0.5 * (1.0 - alpha) - 0.5 * (2.0 * PI * x).cos()
                    + 0.5 * alpha * (4.0 * PI * x).cos()
            })
            .collect();
        Self {
            size,
            bit_reverse,
            twiddles,
            window,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Windowed magnitude spectrum `|X[k]| / N` for bins `0..N/2`.
    pub fn magnitudes(&self, samples: &[f32], out: &mut Vec<f32>) {
        debug_assert_eq!(samples.len(), self.size);
        let mut re = vec![0.0f32; self.size];
        let mut im = vec![0.0f32; self.size];
        for (index, sample) in samples.iter().enumerate() {
            re[self.bit_reverse[index]] = sample * self.window[index];
        }
        let mut length = 2;
        while length <= self.size {
            let half = length / 2;
            let stride = self.size / length;
            for start in (0..self.size).step_by(length) {
                for k in 0..half {
                    let (wr, wi) = self.twiddles[k * stride];
                    let a = start + k;
                    let b = a + half;
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            length *= 2;
        }
        let scale = 1.0 / self.size as f32;
        out.clear();
        out.extend((0..self.size / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() * scale));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_peaks_at_its_bin() {
        let fft = Fft::new(1024);
        let bin = 37;
        let samples: Vec<f32> = (0..1024)
            .map(|n| (2.0 * PI * bin as f32 * n as f32 / 1024.0).sin())
            .collect();
        let mut magnitudes = Vec::new();
        fft.magnitudes(&samples, &mut magnitudes);
        let peak = magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index);
        assert_eq!(peak, Some(bin));
    }
}
