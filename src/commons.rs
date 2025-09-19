// common types and functions

use rand::Rng;
use std::path::PathBuf;
use std::time;

/// Generates a temporary filename in the Linux standard /tmp directory
///
/// # Arguments
/// * `prefix` - A string to use as the prefix for the temporary filename
///
/// # Returns
/// A PathBuf containing the path to a potential temporary file
pub fn temp_filename(prefix: &str) -> PathBuf {
    // standard temporary directory
    let tmp_dir = std::env::temp_dir();

    // Generate a random 16-character alphanumeric suffix
    let mut rng = rand::rng();
    const CHARSET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let suffix: String = (0..16)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();

    // Combine components: /tmp/prefixXXXXXX
    tmp_dir.join(format!("{}{}", prefix, suffix))
}

pub fn get_current_time_micros() -> u128 {
    time::SystemTime::now()
        .duration_since(time::UNIX_EPOCH)
        .unwrap()
        .as_micros()
}
