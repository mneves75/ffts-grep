use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn sync_file(path: &Path) -> io::Result<()> {
    let file = fs::File::open(path)?;
    file.sync_all()
}

pub(crate) fn sync_parent_dir(path: &Path) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    #[cfg(unix)]
    {
        let dir = fs::File::open(parent)?;
        dir.sync_all()
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, FlushFileBuffers, OPEN_EXISTING,
        };

        let mut wide: Vec<u16> = parent.as_os_str().encode_wide().collect();
        wide.push(0);

        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                0,
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        let result = unsafe { FlushFileBuffers(handle) };
        let flush_err = if result == 0 { Some(io::Error::last_os_error()) } else { None };
        let close_result = unsafe { CloseHandle(handle) };

        if let Some(err) = flush_err {
            return Err(err);
        }
        if close_result == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = parent;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_sync_file_ok() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.txt");
        fs::write(&path, "data").unwrap();
        sync_file(&path).unwrap();
    }

    #[test]
    fn test_sync_parent_dir_ok() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("file.txt");
        fs::write(&path, "data").unwrap();
        sync_parent_dir(&path).unwrap();
    }

    #[test]
    fn test_sync_parent_dir_missing_parent_fails() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing").join("file.txt");
        assert!(sync_parent_dir(&missing).is_err());
    }
}
