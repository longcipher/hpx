//! Blink-compatible `PeriodicWave` wavetable.
//!
//! Ports the design of Chromium's `periodic_wave.cc`: band-limited
//! wavetables built via inverse-FFT for each standard wave type.

use rustfft::{FftPlanner, num_complex::Complex};

/// Length of each per-range wavetable.
pub const WAVE_TABLE_SIZE: usize = 2048;

/// Maximum harmonic index we populate.
pub const MAX_NUMBER_OF_PARTIALS: usize = WAVE_TABLE_SIZE / 2;

/// Number of band-limited tables (one per octave).
pub const NUM_RANGES: usize = 12;

/// Standard Web Audio oscillator wave types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardWaveType {
    Sine,
    Triangle,
    Square,
    Sawtooth,
}

impl StandardWaveType {
    /// Fourier coefficients as `(real, imag)` vectors.
    pub fn fourier_coefficients(self) -> (Vec<f32>, Vec<f32>) {
        let len = MAX_NUMBER_OF_PARTIALS + 1;
        let real = vec![0.0f32; len];
        let mut imag = vec![0.0f32; len];
        match self {
            StandardWaveType::Sine => {
                imag[1] = 1.0;
            }
            StandardWaveType::Square => {
                let pi = std::f32::consts::PI;
                for n in (1..=MAX_NUMBER_OF_PARTIALS).step_by(2) {
                    imag[n] = (4.0 / pi) / n as f32;
                }
            }
            StandardWaveType::Sawtooth => {
                let pi = std::f32::consts::PI;
                for n in 1..=MAX_NUMBER_OF_PARTIALS {
                    let sign = if n % 2 == 0 { -1.0 } else { 1.0 };
                    imag[n] = sign * (2.0 / pi) / n as f32;
                }
            }
            StandardWaveType::Triangle => {
                let pi = std::f32::consts::PI;
                let factor = 8.0 / (pi * pi);
                for n in (1..=MAX_NUMBER_OF_PARTIALS).step_by(2) {
                    let sign = if ((n - 1) / 2) % 2 == 0 { 1.0 } else { -1.0 };
                    imag[n] = factor * sign / (n * n) as f32;
                }
            }
        }
        (real, imag)
    }
}

fn build_wavetable(real: &[f32], imag: &[f32]) -> Vec<f32> {
    let n = WAVE_TABLE_SIZE;
    let mut buf: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); n];
    for k in 0..=(n / 2) {
        buf[k] = Complex::new(real[k], -imag[k]);
        if k > 0 && k < n / 2 {
            buf[n - k] = Complex::new(real[k], imag[k]);
        }
    }

    let mut planner = FftPlanner::<f32>::new();
    let ifft = planner.plan_fft_inverse(n);
    ifft.process(&mut buf);

    let mut samples: Vec<f32> = buf.iter().map(|c| c.re).collect();
    let peak = samples.iter().copied().fold(0.0_f32, |a, b| a.max(b.abs()));
    if peak > 0.0 {
        let inv = 1.0 / peak;
        for s in &mut samples {
            *s *= inv;
        }
    }
    samples
}

/// A band-limited wavetable oscillator.
pub struct PeriodicWave {
    tables: Vec<Vec<f32>>,
    sample_rate: f32,
    range_top_hz: Vec<f32>,
}

impl PeriodicWave {
    /// Construct for a standard wave type at the given sample rate.
    pub fn new(wave_type: StandardWaveType, sample_rate: f32) -> Self {
        let nyquist = sample_rate * 0.5;
        let (real, imag) = wave_type.fourier_coefficients();

        let mut tables = Vec::with_capacity(NUM_RANGES);
        let mut range_top_hz = Vec::with_capacity(NUM_RANGES);

        for range in 0..NUM_RANGES {
            let top_hz = nyquist / 2.0_f32.powi(range as i32);
            range_top_hz.push(top_hz);

            let mut real_clamped = real.clone();
            let mut imag_clamped = imag.clone();
            for n in 1..=MAX_NUMBER_OF_PARTIALS {
                if (n as f32) * top_hz > nyquist {
                    real_clamped[n] = 0.0;
                    imag_clamped[n] = 0.0;
                }
            }

            tables.push(build_wavetable(&real_clamped, &imag_clamped));
        }

        Self {
            tables,
            sample_rate,
            range_top_hz,
        }
    }

    /// Sample the wavetable at `phase ∈ [0, 1)`.
    pub fn sample(&self, phase: f32, fundamental_hz: f32) -> f32 {
        let index = self.table_index_for_frequency(fundamental_hz);
        let table = &self.tables[index];
        let position = phase * WAVE_TABLE_SIZE as f32;
        let i0 = (position.floor() as usize) % WAVE_TABLE_SIZE;
        let i1 = (i0 + 1) % WAVE_TABLE_SIZE;
        let frac = position - position.floor();
        table[i0] * (1.0 - frac) + table[i1] * frac
    }

    fn table_index_for_frequency(&self, fundamental_hz: f32) -> usize {
        let mut best = 0usize;
        for (i, &top_hz) in self.range_top_hz.iter().enumerate() {
            if top_hz >= fundamental_hz {
                best = i;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_peak_and_shape() {
        let wave = PeriodicWave::new(StandardWaveType::Sine, 44100.0);
        let mut max_abs: f32 = 0.0;
        let mut min: f32 = 0.0;
        for i in 0..WAVE_TABLE_SIZE {
            let phase = i as f32 / WAVE_TABLE_SIZE as f32;
            let s = wave.sample(phase, 440.0);
            max_abs = max_abs.max(s.abs());
            min = min.min(s);
        }
        assert!((max_abs - 1.0).abs() < 0.01);
        assert!(min < -0.9);
    }

    #[test]
    fn triangle_mean_abs() {
        let wave = PeriodicWave::new(StandardWaveType::Triangle, 44100.0);
        let n = 1024usize;
        let mut sum = 0.0f32;
        for i in 0..n {
            let phase = i as f32 / n as f32;
            sum += wave.sample(phase, 440.0).abs();
        }
        let mean = sum / n as f32;
        assert!((0.40..=0.60).contains(&mean));
    }

    #[test]
    fn deterministic() {
        let a = PeriodicWave::new(StandardWaveType::Triangle, 44100.0);
        let b = PeriodicWave::new(StandardWaveType::Triangle, 44100.0);
        for (i, (ta, tb)) in a.tables.iter().zip(b.tables.iter()).enumerate() {
            assert_eq!(ta, tb, "table {i} not deterministic");
        }
    }
}
