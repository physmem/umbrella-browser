//! Windows DPAPI wrapping for convenience unlock. Blobs are bound to the
//! current Windows user on this machine and do not roam.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DpapiError {
    #[error("DPAPI call failed with Windows error {0}")]
    Os(u32),
    #[error("data is too large for DPAPI")]
    TooLarge,
    #[error("DPAPI is only available on Windows")]
    Unsupported,
}

#[cfg(windows)]
pub use imp::{protect, unprotect};

#[cfg(not(windows))]
pub fn protect(_data: &[u8], _entropy: &[u8]) -> Result<Vec<u8>, DpapiError> {
    Err(DpapiError::Unsupported)
}

#[cfg(not(windows))]
pub fn unprotect(_blob: &[u8], _entropy: &[u8]) -> Result<zeroize::Zeroizing<Vec<u8>>, DpapiError> {
    Err(DpapiError::Unsupported)
}

#[cfg(windows)]
mod imp {
    use std::ptr;

    use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use zeroize::Zeroizing;

    use super::DpapiError;

    pub fn protect(data: &[u8], entropy: &[u8]) -> Result<Vec<u8>, DpapiError> {
        let input = blob(data)?;
        let entropy = blob(entropy)?;
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: input blobs borrow live slices that DPAPI only reads; on
        // success `output` is a LocalAlloc buffer we own and free below.
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                &entropy,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(take(&output, false).to_vec())
    }

    pub fn unprotect(blob_in: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>, DpapiError> {
        let input = blob(blob_in)?;
        let entropy = blob(entropy)?;
        let mut output = CRYPT_INTEGER_BLOB::default();
        // SAFETY: same contract as `protect`.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &entropy,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(take(&output, true))
    }

    // DPAPI takes *mut but never writes through input blobs.
    fn blob(data: &[u8]) -> Result<CRYPT_INTEGER_BLOB, DpapiError> {
        let len = u32::try_from(data.len()).map_err(|_| DpapiError::TooLarge)?;
        Ok(CRYPT_INTEGER_BLOB {
            cbData: len,
            pbData: data.as_ptr().cast_mut(),
        })
    }

    /// Copies a DPAPI output buffer, optionally wipes it, then frees it.
    fn take(output: &CRYPT_INTEGER_BLOB, wipe: bool) -> Zeroizing<Vec<u8>> {
        let len = output.cbData as usize;
        // SAFETY: on success DPAPI returns `cbData` valid bytes at `pbData`,
        // allocated with LocalAlloc; we free it exactly once here.
        unsafe {
            let bytes = std::slice::from_raw_parts(output.pbData, len);
            let copy = Zeroizing::new(bytes.to_vec());
            if wipe {
                ptr::write_bytes(output.pbData, 0, len);
            }
            LocalFree(output.pbData.cast());
            copy
        }
    }

    fn last_error() -> DpapiError {
        // SAFETY: reads thread-local error state, no preconditions.
        DpapiError::Os(unsafe { GetLastError() })
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn round_trip_with_entropy() -> Result<(), DpapiError> {
        let blob = protect(b"vault key bytes", b"entropy")?;
        assert_eq!(unprotect(&blob, b"entropy")?.as_slice(), b"vault key bytes");
        assert!(unprotect(&blob, b"other").is_err());
        Ok(())
    }
}
