//! Saving downloads and handing them to the desktop (open) or the printer.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Where downloads go: `$BROMINE_DOWNLOADS`, else `~/Downloads`.
pub fn download_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("BROMINE_DOWNLOADS") {
        return Ok(dir.into());
    }
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("no HOME to find your Downloads folder in"))?;
    Ok(Path::new(&home).join("Downloads"))
}

/// `name`, or `stem (2).ext`, `stem (3).ext`, ... the first that does not exist. Never overwrites.
pub fn save_unique(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let candidate = dir.join(if n == 1 { name.to_owned() } else { format!("{stem} ({n}){ext}") });
        match OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(mut f) => {
                f.write_all(bytes)?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("the loop only ends by returning")
}

/// Open with the desktop's default program, without waiting for it.
pub fn open(path: &Path) -> io::Result<()> {
    open::that_detached(path)
}

pub fn is_pdf(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".pdf")
}

/// Print a PDF double-sided (long edge) on the default printer.
pub fn print(path: &Path) -> io::Result<()> {
    let out = Command::new("lpr").args(["-o", "sides=two-sided-long-edge"]).arg(path).stdin(Stdio::null()).output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("lpr failed: {}", String::from_utf8_lossy(&out.stderr).trim())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bromine-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn saving_never_overwrites_and_numbers_the_copies() {
        let dir = scratch("unique");
        let a = save_unique(&dir, "Lab.pdf", b"one").unwrap();
        let b = save_unique(&dir, "Lab.pdf", b"two").unwrap();
        let c = save_unique(&dir, "notes", b"x").unwrap();
        let d = save_unique(&dir, "notes", b"y").unwrap();
        let names: Vec<_> = [&a, &b, &c, &d].iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Lab.pdf", "Lab (2).pdf", "notes", "notes (2)"]);
        assert_eq!(std::fs::read(&a).unwrap(), b"one");
        assert_eq!(std::fs::read(&b).unwrap(), b"two");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_pdfs_are_printable() {
        assert!(is_pdf("Postlab.PDF") && !is_pdf("a.docx") && !is_pdf("pdf"));
    }
}
