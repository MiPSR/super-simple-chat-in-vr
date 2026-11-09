//! Linux OFD byte-range pins: one tiny lock file, no per-access metadata writes.
//! Hash collisions only conservatively pin an unrelated entry; never unpin one.
use godot::{classes::RefCounted, prelude::*};
use std::{fs::{File,OpenOptions}, io, os::{fd::AsRawFd,unix::fs::OpenOptionsExt}, path::Path};

fn valid(path: &Path) -> bool {
    path.file_name().and_then(|s|s.to_str()).is_some_and(|name| {
        name.strip_suffix(".zip").is_some_and(|hash| hash.len()==64
            && hash.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    })
}
fn lock(path: &Path, exclusive: bool) -> io::Result<File> {
    if !valid(path) { return Err(io::Error::new(io::ErrorKind::InvalidInput,"Invalid cache filename")); }
    let name = path.file_name().unwrap().to_str().unwrap();
    let offset = u64::from_str_radix(&name[..16],16).unwrap() & i64::MAX as u64;
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path.parent().unwrap().join(".ssc-cache.lock"))?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,"Cache lock is not a regular file"));
    }
    // SAFETY: initialized flock; the kernel copies it during this nonblocking
    // operation. OFD locks belong to this independently opened file description.
    let mut record: libc::flock = unsafe { std::mem::zeroed() };
    record.l_type = (if exclusive {libc::F_WRLCK} else {libc::F_RDLCK}) as _;
    record.l_whence = libc::SEEK_SET as _;
    record.l_start = offset as _;
    record.l_len = 1;
    if unsafe {libc::fcntl(file.as_raw_fd(),libc::F_OFD_SETLK,&record)} == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}
fn regular(path: &Path) -> io::Result<()> {
    if !std::fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,"Cache entry is not a regular file"));
    }
    Ok(())
}
fn remove(path: &Path) -> io::Result<()> {
    let _lock = lock(path,true)?;
    regular(path)?;
    std::fs::remove_file(path)
}

#[derive(GodotClass)]
#[class(base=RefCounted,init)]
struct CacheLease {
    file: Option<File>,
    base: Base<RefCounted>,
}
#[godot_api]
impl CacheLease {
    #[func]
    fn pin(&mut self, path: GString) -> GString {
        if self.file.is_some() { return "Cache lease already owns a pin".into(); }
        let path = path.to_string();
        let path = Path::new(&path);
        match lock(path,false).and_then(|file| { regular(path)?; Ok(file) }) {
            Ok(file) => { self.file=Some(file); GString::new() },
            Err(error) => error.to_string().as_str().into(),
        }
    }
    #[func]
    fn publish_download(&mut self, partial: GString, target: GString) -> GString {
        if self.file.is_some() { return "Cache lease already owns a pin".into(); }
        let partial = partial.to_string();
        let target = target.to_string();
        let partial = Path::new(&partial);
        let target = Path::new(&target);
        let result = (|| {
            if !valid(target) || partial.parent()!=target.parent()
                || !partial.file_name().and_then(|s|s.to_str()).is_some_and(|name| {
                    name.starts_with(&format!("{}.part-",target.file_name().unwrap().to_str().unwrap()))
                }) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput,"Invalid download publication paths"));
            }
            let file=lock(target,false)?;
            regular(partial)?;
            match std::fs::hard_link(partial,target) {
                Ok(())=>{},
                Err(error) if error.kind()==io::ErrorKind::AlreadyExists=>regular(target)?,
                Err(error)=>return Err(error),
            }
            std::fs::remove_file(partial)?;
            Ok(file)
        })();
        match result {
            Ok(file)=>{self.file=Some(file);GString::new()},
            Err(error)=>error.to_string().as_str().into(),
        }
    }
    /// 0 removed, 1 pinned/busy, 2 filesystem/validation failure.
    #[func]
    fn remove_completed(&self, path: GString) -> i32 {
        match remove(Path::new(&path.to_string())) {
            Ok(())=>0,
            Err(error) if matches!(error.raw_os_error(),Some(libc::EAGAIN | libc::EACCES))=>1,
            Err(_)=>2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn another_process_pin_survives_until_process_exit() {
        use std::io::{BufRead,BufReader};
        use std::process::{Command,Stdio};
        let directory=tempfile::tempdir().unwrap();
        let path=directory.path().join(format!("{}.zip","c".repeat(64)));
        std::fs::write(&path,b"zip").unwrap();
        let mut child=Command::new(std::env::current_exe().unwrap())
            .args(["--exact","cache_lock::tests::pin_child","--ignored","--nocapture"])
            .env("SSC_CACHE_PIN_FIXTURE",&path).stdout(Stdio::piped()).stdin(Stdio::piped()).spawn().unwrap();
        let mut output=BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line=String::new();
            assert!(output.read_line(&mut line).unwrap()>0,"Pin child exited before readiness");
            if line.contains("PINNED") { break; }
        }
        assert!(remove(&path).is_err());
        child.kill().unwrap();
        child.wait().unwrap();
        remove(&path).unwrap();
    }
    #[test]
    #[ignore = "launched by parent to verify cross-process crash release"]
    fn pin_child() {
        use std::io::{Read,Write};
        let path=std::env::var_os("SSC_CACHE_PIN_FIXTURE").unwrap();
        let _pin=lock(Path::new(&path),false).unwrap();
        println!("PINNED");
        std::io::stdout().flush().unwrap();
        let _=std::io::stdin().read(&mut [0u8;1]);
    }
    #[test]
    fn independent_open_descriptions_preserve_pins_until_last_reader_retires() {
        let directory=tempfile::tempdir().unwrap();
        let path=directory.path().join(format!("{}.zip","a".repeat(64)));
        std::fs::write(&path,b"zip").unwrap();
        let first=lock(&path,false).unwrap();
        let second=lock(&path,false).unwrap();
        assert!(remove(&path).is_err());
        drop(first);
        assert!(remove(&path).is_err());
        drop(second);
        remove(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(std::fs::metadata(directory.path().join(".ssc-cache.lock")).unwrap().len(),0);
    }
    #[test]
    fn symlink_entries_and_lock_files_are_not_followed() {
        let directory=tempfile::tempdir().unwrap();
        let outside=directory.path().join("outside");
        std::fs::write(&outside,b"keep").unwrap();
        let path=directory.path().join(format!("{}.zip","b".repeat(64)));
        std::os::unix::fs::symlink(&outside,&path).unwrap();
        assert!(remove(&path).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(),b"keep");
        std::fs::remove_file(directory.path().join(".ssc-cache.lock")).unwrap();
        std::os::unix::fs::symlink(&outside,directory.path().join(".ssc-cache.lock")).unwrap();
        assert!(lock(&path,false).is_err());
    }
}
