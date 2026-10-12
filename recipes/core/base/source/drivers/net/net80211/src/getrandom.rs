// Custom getrandom implementation for Redox OS no_std target
// This provides a minimal getrandom implementation for the kernel target

#[cfg(all(target_arch = "aarch64", target_os = "none"))]
pub mod custom {
    use getrandom::Error;

    /// Custom getrandom implementation for Redox OS kernel
    /// This provides a minimal entropy source for the kernel
    pub fn getrandom(buf: &mut [u8]) -> Result<(), Error> {
        // In a real kernel, this would read from a hardware RNG or entropy pool
        // For now, we fill with zeros (not cryptographically secure!)
        // TODO: Implement proper entropy source from hardware RNG
        for byte in buf.iter_mut() {
            *byte = 0;
        }
        Ok(())
    }
}

/// Register the custom getrandom backend
#[cfg(all(target_arch = "aarch64", target_os = "none"))]
#[no_mangle]
pub extern "C" fn getrandom_custom(buf: *mut u8, len: usize) -> i32 {
    if buf.is_null() || len == 0 {
        return 22; // EINVAL
    }
    
    let buf = unsafe { core::slice::from_raw_parts_mut(buf, len) };
    match custom::getrandom(buf) {
        Ok(_) => 0,
        Err(_) => 5, // EIO
    }
}