//! Random passwords from OS randomness, the same source the vault uses for
//! keys and nonces. The result is zeroizing, like every other secret here.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroizing;

use super::crypto::{CryptoError, random_bytes};

pub const MIN_LENGTH: usize = 8;
pub const MAX_LENGTH: usize = 64;
pub const DEFAULT_LENGTH: usize = 20;

const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const DIGITS: &[u8] = b"0123456789";
/// Leaves out quotes, backslash, backtick, angle brackets, pipe and space,
/// which some sites reject or mangle.
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{};:,.?/~";

#[derive(Debug, Error)]
pub enum GeneratorError {
    #[error("no character classes are enabled")]
    NoClasses,
    #[error(transparent)]
    Rng(#[from] CryptoError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneratorOptions {
    pub length: usize,
    pub uppercase: bool,
    pub lowercase: bool,
    pub digits: bool,
    pub symbols: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            length: DEFAULT_LENGTH,
            uppercase: true,
            lowercase: true,
            digits: true,
            symbols: true,
        }
    }
}

impl GeneratorOptions {
    fn classes(&self) -> Vec<&'static [u8]> {
        [
            (self.uppercase, UPPER),
            (self.lowercase, LOWER),
            (self.digits, DIGITS),
            (self.symbols, SYMBOLS),
        ]
        .into_iter()
        .filter_map(|(on, set)| on.then_some(set))
        .collect()
    }

    /// How many classes are on; the settings UI keeps at least one.
    pub fn class_count(&self) -> usize {
        self.classes().len()
    }

    /// Strength if every character were drawn from the whole pool; the
    /// one-per-class guarantee costs a fraction of a bit.
    pub fn entropy_bits(&self) -> u32 {
        let pool: usize = self.classes().iter().map(|c| c.len()).sum();
        if pool < 2 {
            return 0;
        }
        ((pool as f64).log2() * self.effective_length() as f64).floor() as u32
    }

    /// The length actually produced: clamped to the supported range.
    pub fn effective_length(&self) -> usize {
        self.length.clamp(MIN_LENGTH, MAX_LENGTH)
    }
}

/// Builds a password with at least one character from every enabled class,
/// the rest drawn uniformly from all of them, then shuffled.
pub fn generate(options: &GeneratorOptions) -> Result<Zeroizing<String>, GeneratorError> {
    let classes = options.classes();
    if classes.is_empty() {
        return Err(GeneratorError::NoClasses);
    }
    let length = options.effective_length();
    let all: Vec<u8> = classes.iter().flat_map(|c| c.iter().copied()).collect();
    let mut rng = Rng::new();
    let mut bytes = Zeroizing::new(Vec::with_capacity(length));
    for class in &classes {
        bytes.push(class[rng.below(class.len())?]);
    }
    while bytes.len() < length {
        bytes.push(all[rng.below(all.len())?]);
    }
    for i in (1..bytes.len()).rev() {
        let j = rng.below(i + 1)?;
        bytes.swap(i, j);
    }
    let mut out = Zeroizing::new(String::with_capacity(length));
    out.extend(bytes.iter().map(|&b| char::from(b)));
    Ok(out)
}

/// Buffered OS randomness with unbiased bounded draws.
struct Rng {
    pool: Zeroizing<[u8; 64]>,
    used: usize,
}

impl Rng {
    fn new() -> Self {
        Self {
            pool: Zeroizing::new([0; 64]),
            used: 64,
        }
    }

    fn next_u32(&mut self) -> Result<u32, CryptoError> {
        if self.used + 4 > self.pool.len() {
            *self.pool = random_bytes::<64>()?;
            self.used = 0;
        }
        let mut word = [0u8; 4];
        word.copy_from_slice(&self.pool[self.used..self.used + 4]);
        self.used += 4;
        Ok(u32::from_le_bytes(word))
    }

    /// A uniform value in `0..n`. Rejection sampling avoids modulo bias.
    fn below(&mut self, n: usize) -> Result<usize, CryptoError> {
        let n = n as u64;
        let span = 1u64 << 32;
        let limit = span - span % n;
        loop {
            let x = u64::from(self.next_u32()?);
            if x < limit {
                return Ok((x % n) as usize);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), GeneratorError>;

    fn has(password: &str, set: &[u8]) -> bool {
        password.bytes().any(|b| set.contains(&b))
    }

    #[test]
    fn default_uses_every_class() -> TestResult {
        for _ in 0..200 {
            let p = generate(&GeneratorOptions::default())?;
            assert_eq!(p.len(), DEFAULT_LENGTH);
            assert!(has(&p, UPPER) && has(&p, LOWER) && has(&p, DIGITS) && has(&p, SYMBOLS));
        }
        Ok(())
    }

    #[test]
    fn only_enabled_classes_appear() -> TestResult {
        let options = GeneratorOptions {
            length: 32,
            uppercase: false,
            lowercase: true,
            digits: true,
            symbols: false,
        };
        for _ in 0..200 {
            let p = generate(&options)?;
            assert!(p.bytes().all(|b| LOWER.contains(&b) || DIGITS.contains(&b)));
            assert!(has(&p, LOWER) && has(&p, DIGITS));
        }
        Ok(())
    }

    #[test]
    fn guarantee_holds_at_minimum_length() -> TestResult {
        let options = GeneratorOptions {
            length: 1,
            ..GeneratorOptions::default()
        };
        for _ in 0..500 {
            let p = generate(&options)?;
            assert_eq!(p.len(), MIN_LENGTH);
            assert!(has(&p, UPPER) && has(&p, LOWER) && has(&p, DIGITS) && has(&p, SYMBOLS));
        }
        Ok(())
    }

    #[test]
    fn length_is_clamped() -> TestResult {
        let long = GeneratorOptions {
            length: 1000,
            ..GeneratorOptions::default()
        };
        assert_eq!(generate(&long)?.len(), MAX_LENGTH);
        Ok(())
    }

    #[test]
    fn entropy_reflects_pool_and_length() {
        assert_eq!(GeneratorOptions::default().entropy_bits(), 128);
        let digits = GeneratorOptions {
            length: 10,
            uppercase: false,
            lowercase: false,
            symbols: false,
            ..GeneratorOptions::default()
        };
        assert_eq!(digits.entropy_bits(), 33);
    }

    #[test]
    fn no_classes_is_an_error() {
        let none = GeneratorOptions {
            uppercase: false,
            lowercase: false,
            digits: false,
            symbols: false,
            ..GeneratorOptions::default()
        };
        assert!(matches!(generate(&none), Err(GeneratorError::NoClasses)));
    }

    #[test]
    fn passwords_differ() -> TestResult {
        let a = generate(&GeneratorOptions::default())?;
        let b = generate(&GeneratorOptions::default())?;
        assert_ne!(a.as_str(), b.as_str());
        Ok(())
    }

    #[test]
    fn digits_are_roughly_uniform() -> TestResult {
        let options = GeneratorOptions {
            length: MAX_LENGTH,
            uppercase: false,
            lowercase: false,
            digits: true,
            symbols: false,
        };
        let mut counts = [0u32; 10];
        for _ in 0..200 {
            for b in generate(&options)?.bytes() {
                counts[usize::from(b - b'0')] += 1;
            }
        }
        // 12,800 draws, 1,280 expected per digit; a fair source stays well
        // inside these bounds.
        assert!(counts.iter().all(|&c| (1000..1600).contains(&c)), "{counts:?}");
        Ok(())
    }

    #[test]
    fn below_stays_in_range() -> Result<(), CryptoError> {
        let mut rng = Rng::new();
        for n in [1, 2, 3, 7, 10, 89] {
            for _ in 0..100 {
                assert!(rng.below(n)? < n);
            }
        }
        Ok(())
    }
}
