use amow_application::{PortError, PortResult};

/// Minimal filesystem access the camera controller needs from sysfs.
///
/// Injected as a port — exactly like [`CommandRunner`] — so the camera adapter's
/// bind/unbind logic is unit-tested against a fake instead of a live `/sys` tree
/// (which would require a real UVC webcam and root privileges).
///
/// [`CommandRunner`]: crate::CommandRunner
pub trait Sysfs: Send + Sync {
    /// Names of the entries directly inside `path` (non-recursive).
    ///
    /// Returns an error if the directory cannot be read — which the camera
    /// adapter treats as "the uvcvideo driver is not present", i.e. there is no
    /// controllable camera here.
    fn list_dir(&self, path: &str) -> PortResult<Vec<String>>;

    /// Write `value` to the sysfs attribute file at `path`.
    fn write(&self, path: &str, value: &str) -> PortResult<()>;
}

/// Real implementation backed by [`std::fs`].
pub struct RealSysfs;

impl Sysfs for RealSysfs {
    fn list_dir(&self, path: &str) -> PortResult<Vec<String>> {
        let mut names = Vec::new();
        let entries =
            std::fs::read_dir(path).map_err(|e| PortError::new(format!("read_dir {path}: {e}")))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| PortError::new(format!("read_dir entry in {path}: {e}")))?;
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
        Ok(names)
    }

    fn write(&self, path: &str, value: &str) -> PortResult<()> {
        std::fs::write(path, value).map_err(|e| PortError::new(format!("write {path}: {e}")))
    }
}
