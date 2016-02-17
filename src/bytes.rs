use std::fmt;

#[cfg(all(feature = "unstable", target_arch = "x86_64"))]
use super::{PackedCompareOperation, UnalignedByteSliceHandler};
use super::MAX_BYTES;

#[derive(Copy, Clone)]
pub struct Bytes {
    needle_lo: u64,
    needle_hi: u64,
    count: u8,
}

impl Bytes {
    #[inline]
    pub const fn new() -> Bytes {
        Self::from_words(0, 0, 0)
    }

    #[inline]
    /// Create a Bytes with the bytes from `lo` and `hi`, using only
    /// the first `count` bytes.
    pub const fn from_words(lo: u64, hi: u64, count: usize) -> Bytes {
        // This is memory safe even if the user specifies a count > 16
        // here; the PCMPxSTRx instructions will saturate at 16.
        Bytes {
            needle_lo: lo,
            needle_hi: hi,
            count: count as u8,
        }
    }

    /// Add a new byte to the set to search for.
    ///
    /// ### Panics
    ///
    /// - If you add more than 16 bytes.
    pub fn push(&mut self, byte: u8) {
        assert!(self.count < MAX_BYTES);
        self.needle_hi <<= 8;
        self.needle_hi |= self.needle_lo >> (64 - 8);
        self.needle_lo <<= 8;
        self.needle_lo |= byte as u64;
        self.count += 1;
    }

    /// Builds a searcher with a fallback implementation for when the
    /// optimized version is not available. The fallback should search
    /// for the **exact** same set of bytes.
    pub fn with_fallback<F>(self, fallback: F) -> BytesWithFallback<F>
        where F: Fn(u8) -> bool
    {
        BytesWithFallback { inner: self, fallback: fallback }
    }

    /// Find the first index of a byte in the set.
    #[cfg(all(feature = "unstable", target_arch = "x86_64"))]
    #[inline]
    pub fn position(self, haystack: &[u8]) -> Option<usize> {
        UnalignedByteSliceHandler { operation: self }.find(haystack)
    }
}

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Bytes {{ lo: 0x{:016x}, hi: 0x{:016x}, count: {} }}",
               self.needle_lo, self.needle_hi, self.count)
    }
}

#[cfg(all(feature = "unstable", target_arch = "x86_64"))]
impl PackedCompareOperation for Bytes {
    unsafe fn initial(&self, ptr: *const u8, offset: usize, len: usize) -> u64 {
        let matching_bytes;

        asm!("movlhps $2, $1
              pcmpestrm $$0, ($3), $1"
             : // output operands
             "={xmm0}"(matching_bytes)
             : // input operands
             "x"(self.needle_lo),
             "x"(self.needle_hi),
             "r"(ptr),
             "{rdx}"(offset + len), // saturates at 16
             "{rax}"(self.count as u64)
             : // clobbers
             "cc"
             : // options
        );

        matching_bytes
    }

    unsafe fn body(&self, ptr: *const u8, offset: usize, len: usize) -> u32 {
        let res;

        asm!("# Move low word of $2 to high word of $1
              movlhps $2, $1
              pcmpestri $$0, ($3, $4), $1"
             : // output operands
             "={ecx}"(res)
             : // input operands
             "x"(self.needle_lo),
             "x"(self.needle_hi),
             "r"(ptr),
             "r"(offset)
             "{rdx}"(len),              // haystack length
             "{rax}"(self.count as u64) // needle_lo length
             : // clobbers
             "cc"
             : // options
         );

        res
    }
}

/// Provides a hook for a user-supplied fallback implementation, used
/// when the optimized instructions are not available.
///
/// Although this implementation is a bit ungainly, Rust's closure
/// inlining is top-notch and provides the best speed.
#[derive(Debug, Copy, Clone)]
pub struct BytesWithFallback<F> {
    inner: Bytes,
    fallback: F,
}

impl<F> BytesWithFallback<F>
    where F: Fn(u8) -> bool
{
    #[cfg(all(feature = "unstable", target_arch = "x86_64"))]
    pub fn position(&self, haystack: &[u8]) -> Option<usize> {
        self.inner.position(haystack)
    }

    #[cfg(not(all(feature = "unstable", target_arch = "x86_64")))]
    pub fn position(&self, haystack: &[u8]) -> Option<usize> {
        haystack.iter().cloned().position(&self.fallback)
    }
}

#[cfg(test)]
mod test {
    // The vast majority of interesting tests are driven from the
    // ASCII-only side of things, although they would probably make
    // more sense here.

    use super::Bytes;

    #[test]
    fn non_ascii_bytes_can_be_found() {
        let mut needle = Bytes::new();
        needle.push(0x80);
        let needle = needle.with_fallback(|b| b == 0x80);
        let haystack = [0xFF, 0x80];
        assert_eq!(Some(1), needle.position(&haystack));
    }
}
