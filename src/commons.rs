// common types and functions

use rand::Rng;
use std::path::PathBuf;
use std::{
    fs::{self, File},
    io::BufWriter,
    path::Path,
};

/// Generates a temporary filename in the Linux standard /tmp directory
///
/// # Arguments
/// * `prefix` - A string to use as the prefix for the temporary filename
///
/// # Returns
/// A PathBuf containing the path to a potential temporary file
pub fn temp_filename(prefix: &str) -> PathBuf {
    // Linux standard temporary directory
    let tmp_dir = PathBuf::from("/tmp");

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

/// Linux-specific struct to manage a protected directory
pub struct ProtectedDir {
    path: PathBuf,
    // Open and maintain a handle to the directory (prevents deletion on Linux)
    #[allow(dead_code)]
    dir_handle: File,
}

impl ProtectedDir {
    /// Create or reuse a directory and protect it (Linux-only)
    pub fn from_path_str(dir_path: &str) -> std::io::Result<Self> {
        let path = Path::new(dir_path);
        Self::from_path(path)
    }

    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        // Create directory if it doesn't exist
        fs::create_dir_all(&path)?;

        // On Linux, we can open a directory directly with File::open
        // This holds a handle that prevents deletion/renaming
        let dir_handle = File::open(&path)?;

        Ok(Self {
            path: path.to_path_buf(),
            dir_handle,
        })
    }

    /// Create a file in the protected directory and return a BufWriter
    pub fn create_output_file(
        &self,
        filename: &str,
    ) -> std::io::Result<BufWriter<File>> {
        let file_path = self.path.join(filename);
        let file = File::create(&file_path)?;
        Ok(BufWriter::new(file))
    }

    pub fn full_file_name(&self, filename: &str) -> String {
        self.path.join(filename).to_string_lossy().to_string()
    }
}

/// Converts a path string (relative or absolute) to an absolute path string.
///
/// # Arguments
/// * `path_str` - The input path string to convert.
///
/// # Returns
/// A `Result` containing the absolute path string on success, or an `std::io::Error` on failure.
/// Failures can occur if the path is invalid or if the current working directory can't be retrieved.
pub fn to_absolute_path(path_str: &str) -> std::io::Result<String> {
    let path = Path::new(path_str);

    // Check if the path is already absolute
    let absolute_path = if path.is_absolute() {
        PathBuf::from(path)
    } else {
        // Get current working directory and append the relative path
        let cwd = fs::canonicalize(".")?;
        cwd.join(path)
    };

    // Canonicalize to resolve any symbolic links and ".." components
    let canonical_path = fs::canonicalize(absolute_path)?;

    // Convert PathBuf to String
    canonical_path
        .to_str()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Path contains invalid Unicode characters",
            )
        })
        .map(|s| s.to_string())
}
