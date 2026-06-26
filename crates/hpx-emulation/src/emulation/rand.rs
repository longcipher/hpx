use strum::VariantArray;

use super::{Emulation, EmulationOS, EmulationOption};

// Market share weights per browser family.
const CHROME_WEIGHT: f32 = 0.714;
const SAFARI_WEIGHT: f32 = 0.150;
const EDGE_WEIGHT: f32 = 0.050;
const FIREFOX_WEIGHT: f32 = 0.030;
const OPERA_WEIGHT: f32 = 0.020;
const OKHTTP_WEIGHT: f32 = 0.010;

// ponytail: flat per-variant distribution within each family.
// Upgrade to per-variant weights when old-version usage data is available.
macro_rules! weight_variants {
    ($weight:expr, $($v:expr),+ $(,)?) => {{
        const COUNT: usize = [$(stringify!($v)),+].len();
        const W: f32 = $weight / COUNT as f32;
        &[$(($v, W)),+] as &[(Emulation, f32)]
    }};
}

const WEIGHTED: &[&[(Emulation, f32)]] = &[
    weight_variants!(
        CHROME_WEIGHT,
        Emulation::Chrome100,
        Emulation::Chrome101,
        Emulation::Chrome104,
        Emulation::Chrome105,
        Emulation::Chrome106,
        Emulation::Chrome107,
        Emulation::Chrome108,
        Emulation::Chrome109,
        Emulation::Chrome110,
        Emulation::Chrome114,
        Emulation::Chrome116,
        Emulation::Chrome117,
        Emulation::Chrome118,
        Emulation::Chrome119,
        Emulation::Chrome120,
        Emulation::Chrome123,
        Emulation::Chrome124,
        Emulation::Chrome126,
        Emulation::Chrome127,
        Emulation::Chrome128,
        Emulation::Chrome129,
        Emulation::Chrome130,
        Emulation::Chrome131,
        Emulation::Chrome132,
        Emulation::Chrome133,
        Emulation::Chrome134,
        Emulation::Chrome135,
        Emulation::Chrome136,
        Emulation::Chrome137,
        Emulation::Chrome138,
        Emulation::Chrome139,
        Emulation::Chrome140,
        Emulation::Chrome141,
        Emulation::Chrome142,
        Emulation::Chrome143,
        Emulation::Chrome144,
        Emulation::Chrome145,
        Emulation::Chrome146,
        Emulation::Chrome147,
        Emulation::Chrome148,
        Emulation::Chrome149,
    ),
    weight_variants!(
        SAFARI_WEIGHT,
        Emulation::SafariIos17_2,
        Emulation::SafariIos17_4_1,
        Emulation::SafariIos16_5,
        Emulation::Safari15_3,
        Emulation::Safari15_5,
        Emulation::Safari15_6_1,
        Emulation::Safari16,
        Emulation::Safari16_5,
        Emulation::Safari17_0,
        Emulation::Safari17_2_1,
        Emulation::Safari17_4_1,
        Emulation::Safari17_5,
        Emulation::Safari17_6,
        Emulation::Safari18,
        Emulation::SafariIPad18,
        Emulation::Safari18_2,
        Emulation::SafariIos18_1_1,
        Emulation::Safari18_3,
        Emulation::Safari18_3_1,
        Emulation::Safari18_5,
        Emulation::Safari26,
        Emulation::Safari26_1,
        Emulation::Safari26_2,
        Emulation::SafariIPad26,
        Emulation::SafariIPad26_2,
        Emulation::SafariIos26,
        Emulation::SafariIos26_2,
        Emulation::Safari19,
        Emulation::SafariIos19,
        Emulation::SafariIPad19,
        Emulation::Safari20,
        Emulation::SafariIos20,
        Emulation::SafariIPad20,
        Emulation::Safari21,
        Emulation::SafariIos21,
        Emulation::SafariIPad21,
        Emulation::Safari22,
        Emulation::SafariIos22,
        Emulation::SafariIPad22,
        Emulation::Safari23,
        Emulation::SafariIos23,
        Emulation::SafariIPad23,
        Emulation::Safari24,
        Emulation::SafariIos24,
        Emulation::SafariIPad24,
        Emulation::Safari25,
        Emulation::SafariIos25,
        Emulation::SafariIPad25,
        Emulation::Safari26_3,
        Emulation::SafariIos26_3,
        Emulation::SafariIPad26_3,
        Emulation::Safari26_4,
        Emulation::SafariIos26_4,
        Emulation::SafariIPad26_4,
    ),
    weight_variants!(
        EDGE_WEIGHT,
        Emulation::Edge101,
        Emulation::Edge122,
        Emulation::Edge127,
        Emulation::Edge131,
        Emulation::Edge134,
        Emulation::Edge135,
        Emulation::Edge136,
        Emulation::Edge137,
        Emulation::Edge138,
        Emulation::Edge139,
        Emulation::Edge140,
        Emulation::Edge141,
        Emulation::Edge142,
        Emulation::Edge143,
        Emulation::Edge144,
        Emulation::Edge145,
        Emulation::Edge146,
        Emulation::Edge147,
        Emulation::Edge148,
    ),
    weight_variants!(
        FIREFOX_WEIGHT,
        Emulation::Firefox109,
        Emulation::Firefox117,
        Emulation::Firefox128,
        Emulation::Firefox133,
        Emulation::Firefox135,
        Emulation::FirefoxPrivate135,
        Emulation::FirefoxAndroid135,
        Emulation::Firefox136,
        Emulation::FirefoxPrivate136,
        Emulation::Firefox137,
        Emulation::Firefox138,
        Emulation::Firefox139,
        Emulation::Firefox140,
        Emulation::Firefox141,
        Emulation::Firefox142,
        Emulation::Firefox143,
        Emulation::Firefox144,
        Emulation::Firefox145,
        Emulation::Firefox146,
        Emulation::Firefox147,
        Emulation::Firefox148,
        Emulation::Firefox149,
        Emulation::Firefox150,
        Emulation::Firefox151,
    ),
    weight_variants!(
        OPERA_WEIGHT,
        Emulation::Opera116,
        Emulation::Opera117,
        Emulation::Opera118,
        Emulation::Opera119,
        Emulation::Opera120,
        Emulation::Opera121,
        Emulation::Opera122,
        Emulation::Opera123,
        Emulation::Opera124,
        Emulation::Opera125,
        Emulation::Opera126,
        Emulation::Opera127,
        Emulation::Opera128,
        Emulation::Opera129,
        Emulation::Opera130,
        Emulation::Opera131,
    ),
    weight_variants!(
        OKHTTP_WEIGHT,
        Emulation::OkHttp3_9,
        Emulation::OkHttp3_11,
        Emulation::OkHttp3_13,
        Emulation::OkHttp3_14,
        Emulation::OkHttp4_9,
        Emulation::OkHttp4_10,
        Emulation::OkHttp4_12,
        Emulation::OkHttp5,
    ),
];

impl Emulation {
    /// Returns a random variant of the `Emulation` enum.
    ///
    /// This method uses the `rand` crate to select a random variant
    /// from the `Emulation::VARIANTS` array.
    ///
    /// # Examples
    ///
    /// ```
    /// use hpx_emulation::Emulation;
    ///
    /// let random_emulation = Emulation::random();
    /// println!("{:?}", random_emulation);
    /// ```
    ///
    /// # Panics
    ///
    /// This method will panic if the `Emulation::VARIANTS` array is empty.
    #[inline]
    pub fn random() -> EmulationOption {
        let emulation = Emulation::VARIANTS;
        let emulation_os = EmulationOS::VARIANTS;
        let rand = rand::random::<u64>() as usize;
        EmulationOption::builder()
            .emulation(emulation[rand % emulation.len()])
            .emulation_os(emulation_os[rand % emulation_os.len()])
            .build()
    }

    /// Returns a weighted-random `EmulationOption` biased by browser market share.
    ///
    /// Weights approximate global desktop+mobile browser share:
    /// Chrome ~71.4%, Safari ~15%, Edge ~5%, Firefox ~3%, Opera ~2%, OkHttp ~1%.
    ///
    /// The OS is chosen uniformly from [`EmulationOS::VARIANTS`].
    ///
    /// # Examples
    ///
    /// ```
    /// use hpx_emulation::Emulation;
    ///
    /// let opt = Emulation::weighted_random();
    /// println!("{:?}", opt);
    /// ```
    #[inline]
    pub fn weighted_random() -> EmulationOption {
        let r = rand::random::<f32>();
        let mut cumulative = 0.0f32;
        let emulation = 'outer: {
            for variants in WEIGHTED {
                for &(emu, w) in *variants {
                    cumulative += w;
                    if r < cumulative {
                        break 'outer emu;
                    }
                }
            }
            // ponytail: float rounding tail — pick last variant.
            Emulation::OkHttp5
        };
        let emulation_os = EmulationOS::VARIANTS;
        let os_idx = rand::random::<u64>() as usize % emulation_os.len();
        EmulationOption::builder()
            .emulation(emulation)
            .emulation_os(emulation_os[os_idx])
            .build()
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use parking_lot::Mutex;

    use super::*;

    #[test]
    fn test_concurrent_get_random_emulation() {
        const THREAD_COUNT: usize = 10;
        const ITERATIONS: usize = 100;

        let results = Arc::new(Mutex::new(Vec::new()));

        let mut handles = vec![];

        for _ in 0..THREAD_COUNT {
            let results = Arc::clone(&results);
            let handle = thread::spawn(move || {
                for _ in 0..ITERATIONS {
                    let emulation = Emulation::random();
                    let mut results = results.lock();
                    results.push(emulation);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().expect("worker thread panicked");
        }

        let results = results.lock();
        println!("Total results: {}", results.len());
    }

    #[test]
    fn test_weighted_random_chrome_frequency() {
        const N: usize = 10_000;
        let mut chrome_count: usize = 0;
        for _ in 0..N {
            let opt = Emulation::weighted_random();
            if format!("{:?}", opt.emulation).starts_with("Chrome") {
                chrome_count += 1;
            }
        }
        let freq = chrome_count as f64 / N as f64;
        assert!(
            (0.66..=0.77).contains(&freq),
            "Chrome frequency {freq:.3} outside [0.66, 0.77]"
        );
    }
}
