use strum::VariantArray;

use super::{Emulation, EmulationOS, EmulationOption};

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
}
