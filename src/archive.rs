//! Finding and extracting VPK packages from mod archives (zip, 7z, rar or a bare vpk).
//!
//! Archives are read from memory whenever possible; only the selected VPKs are
//! written, straight to their final location. RAR is the exception: the unrar
//! library only reads from a path, so a RAR in memory gets a short-lived temp file.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Cursor, Read, Seek};
use std::path::{Path, PathBuf};

pub enum Source {
    Memory(Vec<u8>),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Zip,
    SevenZ,
    Rar,
    Vpk,
}

/// One installable mod variant: a `*_dir.vpk` and its numbered data parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    /// Path of the main file inside the archive, shown to the user.
    pub label: String,
    /// `(part suffix, entry name in archive)`, main file first.
    pub entries: Vec<(String, String)>,
}

pub fn list_packages(source: &Source) -> io::Result<Vec<Package>> {
    let names = match detect(source)? {
        Kind::Zip => with_reader(source, |r| zip_names(r))?,
        Kind::SevenZ => with_reader(source, |r| sevenz_names(r))?,
        Kind::Rar => with_rar_path(source, rar_names)?,
        Kind::Vpk => vec![String::from("mod.vpk")],
    };
    Ok(group_packages(&names))
}

/// Extracts the given packages; `dest(package_index, part)` names each output file.
pub fn extract(
    source: &Source,
    packages: &[Package],
    dest: impl Fn(usize, &str) -> PathBuf,
) -> io::Result<()> {
    let wanted: HashMap<String, PathBuf> = packages
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            p.entries
                .iter()
                .map(move |(part, name)| (name.clone(), (i, part)))
        })
        .map(|(name, (i, part))| (name, dest(i, part)))
        .collect();

    match detect(source)? {
        Kind::Zip => with_reader(source, |r| zip_extract(r, &wanted))?,
        Kind::SevenZ => with_reader(source, |r| sevenz_extract(r, &wanted))?,
        Kind::Rar => with_rar_path(source, |p| rar_extract(p, &wanted))?,
        Kind::Vpk => {
            let target = wanted
                .values()
                .next()
                .ok_or_else(|| io::Error::other("нечего распаковывать"))?;
            match source {
                Source::Memory(bytes) => fs::write(target, bytes)?,
                Source::File(path) => drop(fs::copy(path, target)?),
            }
        }
    }
    let missing = wanted.values().filter(|p| !p.is_file()).count();
    match missing {
        0 => Ok(()),
        n => Err(io::Error::other(format!("не распаковано файлов: {n}"))),
    }
}

fn detect(source: &Source) -> io::Result<Kind> {
    let mut head = [0u8; 8];
    let len = match source {
        Source::Memory(bytes) => {
            let n = bytes.len().min(8);
            head[..n].copy_from_slice(&bytes[..n]);
            n
        }
        Source::File(path) => File::open(path)?.read(&mut head)?,
    };
    let head = &head[..len];
    if head.starts_with(b"PK\x03\x04") {
        Ok(Kind::Zip)
    } else if head.starts_with(b"7z\xBC\xAF\x27\x1C") {
        Ok(Kind::SevenZ)
    } else if head.starts_with(b"Rar!\x1A\x07") {
        Ok(Kind::Rar)
    } else if head.starts_with(&[0x34, 0x12, 0xAA, 0x55]) {
        Ok(Kind::Vpk)
    } else {
        Err(io::Error::other(
            "неподдерживаемый формат: нужен zip, 7z, rar или vpk",
        ))
    }
}

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

fn with_reader<T>(
    source: &Source,
    f: impl FnOnce(&mut dyn ReadSeek) -> io::Result<T>,
) -> io::Result<T> {
    match source {
        Source::Memory(bytes) => f(&mut Cursor::new(bytes.as_slice())),
        Source::File(path) => f(&mut io::BufReader::new(File::open(path)?)),
    }
}

fn with_rar_path<T>(source: &Source, f: impl FnOnce(&Path) -> io::Result<T>) -> io::Result<T> {
    match source {
        Source::File(path) => f(path),
        Source::Memory(bytes) => {
            let tmp =
                std::env::temp_dir().join(format!("deadlock-launcher-{}.rar", std::process::id()));
            fs::write(&tmp, bytes)?;
            let result = f(&tmp);
            let _ = fs::remove_file(&tmp);
            result
        }
    }
}

fn create(path: &Path) -> io::Result<BufWriter<File>> {
    Ok(BufWriter::with_capacity(1 << 20, File::create(path)?))
}

fn zip_names(reader: &mut dyn ReadSeek) -> io::Result<Vec<String>> {
    let archive = zip::ZipArchive::new(reader).map_err(io::Error::other)?;
    archive
        .file_names()
        .map(|name| name.map(|n| normalize(&n)).map_err(io::Error::other))
        .collect()
}

fn zip_extract(reader: &mut dyn ReadSeek, wanted: &HashMap<String, PathBuf>) -> io::Result<()> {
    let mut archive = zip::ZipArchive::new(reader).map_err(io::Error::other)?;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(io::Error::other)?;
        let name = normalize(&file.name().map_err(io::Error::other)?);
        if let Some(target) = wanted.get(&name) {
            io::copy(&mut file, &mut create(target)?)?;
        }
    }
    Ok(())
}

fn sevenz_names(reader: &mut dyn ReadSeek) -> io::Result<Vec<String>> {
    let archive = sevenz_rust2::ArchiveReader::new(reader, sevenz_rust2::Password::empty())
        .map_err(io::Error::other)?;
    Ok(archive
        .archive()
        .files
        .iter()
        .filter(|e| !e.is_directory())
        .map(|e| normalize(e.name()))
        .collect())
}

fn sevenz_extract(reader: &mut dyn ReadSeek, wanted: &HashMap<String, PathBuf>) -> io::Result<()> {
    let mut archive = sevenz_rust2::ArchiveReader::new(reader, sevenz_rust2::Password::empty())
        .map_err(io::Error::other)?;
    archive
        .for_each_entries(|entry, data| {
            if let Some(target) = wanted.get(&normalize(entry.name())) {
                io::copy(data, &mut create(target)?)?;
            }
            Ok(true)
        })
        .map_err(io::Error::other)
}

fn rar_names(path: &Path) -> io::Result<Vec<String>> {
    let archive = unrar::Archive::new(path)
        .open_for_listing()
        .map_err(io::Error::other)?;
    let mut names = Vec::new();
    for header in archive {
        let header = header.map_err(io::Error::other)?;
        if header.is_file() {
            names.push(normalize(&header.filename.to_string_lossy()));
        }
    }
    Ok(names)
}

fn rar_extract(path: &Path, wanted: &HashMap<String, PathBuf>) -> io::Result<()> {
    let mut archive = unrar::Archive::new(path)
        .open_for_processing()
        .map_err(io::Error::other)?;
    while let Some(header) = archive.read_header().map_err(io::Error::other)? {
        let name = normalize(&header.entry().filename.to_string_lossy());
        archive = match wanted.get(&name) {
            Some(target) => header.extract_to(target).map_err(io::Error::other)?,
            None => header.skip().map_err(io::Error::other)?,
        };
    }
    Ok(())
}

fn normalize(name: &str) -> String {
    name.replace('\\', "/")
}

/// Splits archive entries into packages. A `foo_dir.vpk` takes `foo_000.vpk`,
/// `foo_001.vpk`... from the same folder; a lone `foo.vpk` is a single-file package.
fn group_packages(names: &[String]) -> Vec<Package> {
    let vpks: Vec<&String> = names
        .iter()
        .filter(|n| n.to_ascii_lowercase().ends_with(".vpk") && !n.contains("__MACOSX"))
        .collect();
    let mut packages = Vec::new();
    for name in &vpks {
        let stem = &name[..name.len() - 4];
        if let Some(base) = strip_suffix_ci(stem, "_dir") {
            let mut entries = vec![("dir".to_string(), (*name).clone())];
            for other in &vpks {
                let other_stem = &other[..other.len() - 4];
                if let Some(num) = other_stem
                    .strip_prefix(base)
                    .and_then(|r| r.strip_prefix('_'))
                    && num.len() == 3
                    && num.bytes().all(|b| b.is_ascii_digit())
                {
                    entries.push((num.to_string(), (*other).clone()));
                }
            }
            entries[1..].sort();
            packages.push(Package {
                label: (*name).clone(),
                entries,
            });
        } else if !is_numbered_part(stem) {
            packages.push(Package {
                label: (*name).clone(),
                entries: vec![("dir".to_string(), (*name).clone())],
            });
        }
    }
    packages.sort_by(|a, b| a.label.cmp(&b.label));
    packages
}

fn strip_suffix_ci<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let split = text.len().checked_sub(suffix.len())?;
    (text.is_char_boundary(split) && text[split..].eq_ignore_ascii_case(suffix))
        .then(|| &text[..split])
}

fn is_numbered_part(stem: &str) -> bool {
    stem.rsplit_once('_')
        .is_some_and(|(_, num)| num.len() == 3 && num.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn groups_dir_with_numbered_parts() {
        let packages = group_packages(&names(&[
            "a/pak01_dir.vpk",
            "a/pak01_001.vpk",
            "a/pak01_000.vpk",
            "readme.txt",
        ]));
        assert_eq!(packages.len(), 1);
        let parts: Vec<&str> = packages[0]
            .entries
            .iter()
            .map(|(p, _)| p.as_str())
            .collect();
        assert_eq!(parts, ["dir", "000", "001"]);
    }

    #[test]
    fn variants_in_folders_are_separate_packages() {
        let packages = group_packages(&names(&["Red/pak01_dir.vpk", "Blue/pak01_dir.vpk"]));
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].entries.len(), 1);
    }

    #[test]
    fn lone_vpk_is_a_package_but_orphan_part_is_not() {
        let packages = group_packages(&names(&[
            "cool_mod.vpk",
            "orphan_000.vpk",
            "__MACOSX/x_dir.vpk",
        ]));
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].label, "cool_mod.vpk");
    }

    #[test]
    fn extracts_selected_package_from_zip_in_memory() {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("Red/pak01_dir.vpk", opts).unwrap();
            zip.write_all(b"red").unwrap();
            zip.start_file("Blue/pak01_dir.vpk", opts).unwrap();
            zip.write_all(b"blue").unwrap();
            zip.finish().unwrap();
        }
        let source = Source::Memory(buf.into_inner());
        let packages = list_packages(&source).unwrap();
        let blue: Vec<Package> = packages
            .into_iter()
            .filter(|p| p.label.starts_with("Blue"))
            .collect();

        let out = std::env::temp_dir().join(format!("dl-zip-{}.vpk", std::process::id()));
        extract(&source, &blue, |_, _| out.clone()).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"blue");
        fs::remove_file(out).unwrap();
    }

    #[test]
    fn rejects_unknown_format() {
        assert!(list_packages(&Source::Memory(b"hello world".to_vec())).is_err());
    }
}
