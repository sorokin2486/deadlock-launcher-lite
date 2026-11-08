//! Background work. Every job runs on a short-lived thread (or the small image
//! pool) and reports back through one channel; the UI repaints only when a
//! message arrives, so an idle launcher uses no CPU.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage};

use crate::archive::{self, Package, Source};
use crate::gamebanana::{self, Category, GbFile, ModDetails, Page, Sort};
use crate::library::GbSource;

const IMAGE_THREADS: usize = 2;
const IMAGE_LIMIT: u64 = 4 * 1024 * 1024;
const PROGRESS_REPAINT: Duration = Duration::from_millis(150);

pub enum Query {
    Browse { sort: Sort, category: Option<u64> },
    Search(String),
}

/// Sidebar contents: catalog sections and heroes (subcategories of Skins).
pub struct Catalog {
    pub sections: Vec<Category>,
    pub heroes: Vec<Category>,
}

/// What an install is for; travels with the job and back to the UI.
#[derive(Clone)]
pub struct InstallTarget {
    pub name: String,
    pub source: Option<GbSource>,
    /// Installed mods removed once the new one is in place; the new mod takes
    /// the list position of the first of them.
    pub replaces: Vec<u64>,
    /// An update keeps the old mod's on/off state; a hero replacement is enabled.
    pub keep_state: bool,
}

/// An archive that is downloaded and inspected, waiting for the variant choice.
pub struct Prepared {
    pub target: InstallTarget,
    pub archive: Source,
    /// Downloaded temp files are removed after extraction; dropped user files are not.
    pub temp: bool,
    pub packages: Vec<Package>,
}

pub struct Extracted {
    pub target: InstallTarget,
    /// `(uid, variant label, part suffixes)` for every extracted package.
    pub items: Vec<(u64, String, Vec<String>)>,
}

pub enum Msg {
    Page {
        generation: u64,
        result: io::Result<Page>,
    },
    Details {
        id: u64,
        result: io::Result<ModDetails>,
    },
    Catalog(io::Result<Catalog>),
    Image {
        generation: u64,
        url: String,
        image: Option<ColorImage>,
    },
    Prepared(io::Result<Prepared>),
    Extracted(io::Result<Extracted>),
    Updates(Vec<(u64, i64)>),
}

pub struct Tasks {
    ctx: egui::Context,
    tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
    images: Sender<(u64, String)>,
    /// Bumped on every page change so queued thumbnails of old pages are skipped.
    pub image_generation: Arc<AtomicU64>,
    pub progress: Arc<AtomicU64>,
}

impl Tasks {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();
        let (images, image_rx) = mpsc::channel::<(u64, String)>();
        let image_rx = Arc::new(Mutex::new(image_rx));
        let image_generation = Arc::new(AtomicU64::new(0));
        for _ in 0..IMAGE_THREADS {
            let (rx, tx, ctx, current) = (
                image_rx.clone(),
                tx.clone(),
                ctx.clone(),
                image_generation.clone(),
            );
            std::thread::spawn(move || image_worker(&rx, &tx, &ctx, &current));
        }
        Self {
            ctx,
            tx,
            rx,
            images,
            image_generation,
            progress: Arc::new(AtomicU64::new(0)),
        }
    }

    fn spawn(&self, job: impl FnOnce() -> Msg + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }

    pub fn load_page(&self, query: Query, page: usize, generation: u64) {
        self.spawn(move || Msg::Page {
            generation,
            result: match query {
                Query::Browse { sort, category } => gamebanana::browse(sort, category, page),
                Query::Search(text) => gamebanana::search(&text, page),
            },
        });
    }

    pub fn load_details(&self, id: u64) {
        self.spawn(move || Msg::Details {
            id,
            result: gamebanana::details(id),
        });
    }

    pub fn load_catalog(&self) {
        self.spawn(|| Msg::Catalog(load_catalog()));
    }

    pub fn load_image(&self, generation: u64, url: String) {
        let _ = self.images.send((generation, url));
    }

    pub fn download(&self, file: GbFile, target: InstallTarget) {
        let progress = self.progress.clone();
        progress.store(0, Ordering::Relaxed);
        let ctx = self.ctx.clone();
        self.spawn(move || {
            let last = Mutex::new(Instant::now());
            let repaint = || {
                let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
                if last.elapsed() >= PROGRESS_REPAINT {
                    *last = Instant::now();
                    ctx.request_repaint();
                }
            };
            Msg::Prepared(
                gamebanana::download(&file, &progress, &repaint).and_then(|archive| {
                    let temp = matches!(archive, Source::File(_));
                    inspect(target, archive, temp)
                }),
            )
        });
    }

    pub fn open_local(&self, path: PathBuf, target: InstallTarget) {
        self.spawn(move || Msg::Prepared(inspect(target, Source::File(path), false)));
    }

    /// Extracts the chosen packages; `uids[i]` and `dest(uid, part)` name the outputs.
    pub fn extract(
        &self,
        prepared: Prepared,
        uids: Vec<u64>,
        dest: impl Fn(u64, &str) -> PathBuf + Send + 'static,
    ) {
        self.spawn(move || {
            let Prepared {
                target,
                archive,
                temp,
                packages,
            } = prepared;
            let result = archive::extract(&archive, &packages, |i, part| dest(uids[i], part));
            if let (true, Source::File(path)) = (temp, &archive) {
                let _ = std::fs::remove_file(path);
            }
            drop(archive);
            Msg::Extracted(result.map(|()| {
                Extracted {
                    target,
                    items: packages
                        .into_iter()
                        .zip(uids)
                        .map(|(p, uid)| {
                            (
                                uid,
                                p.label,
                                p.entries.into_iter().map(|(part, _)| part).collect(),
                            )
                        })
                        .collect(),
                }
            }))
        });
    }

    pub fn check_updates(&self, mods: Vec<u64>) {
        self.spawn(move || {
            Msg::Updates(
                mods.into_iter()
                    .filter_map(|id| gamebanana::updated_at(id).ok().map(|ts| (id, ts)))
                    .collect(),
            )
        });
    }
}

/// Root sections plus the per-hero subcategories of the Skins section.
fn load_catalog() -> io::Result<Catalog> {
    let mut sections = gamebanana::categories(None)?;
    let skins = sections
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case("Skins"));
    let heroes = match skins {
        Some(index) => gamebanana::categories(Some(sections.remove(index).id))?,
        None => Vec::new(),
    };
    Ok(Catalog { sections, heroes })
}

fn inspect(target: InstallTarget, archive: Source, temp: bool) -> io::Result<Prepared> {
    let packages = archive::list_packages(&archive);
    match packages {
        Ok(packages) if !packages.is_empty() => Ok(Prepared {
            target,
            archive,
            temp,
            packages,
        }),
        other => {
            if let (true, Source::File(path)) = (temp, &archive) {
                let _ = std::fs::remove_file(path);
            }
            match other {
                Ok(_) => Err(io::Error::other("в архиве нет файлов .vpk")),
                Err(e) => Err(e),
            }
        }
    }
}

fn image_worker(
    jobs: &Mutex<Receiver<(u64, String)>>,
    tx: &Sender<Msg>,
    ctx: &egui::Context,
    current: &AtomicU64,
) {
    loop {
        let job = jobs.lock().unwrap_or_else(|e| e.into_inner()).recv();
        let Ok((generation, url)) = job else { return };
        if generation != current.load(Ordering::Relaxed) {
            continue;
        }
        let image = gamebanana::fetch_bytes(&url, IMAGE_LIMIT)
            .ok()
            .and_then(|bytes| decode(&bytes));
        if tx
            .send(Msg::Image {
                generation,
                url,
                image,
            })
            .is_err()
        {
            return;
        }
        ctx.request_repaint();
    }
}

fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let rgba = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real network round trip: run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn downloads_and_extracts_a_real_mod() {
        let details = gamebanana::details(650_634).unwrap();
        let file = details.files.first().unwrap().clone();
        let progress = AtomicU64::new(0);
        let archive = gamebanana::download(&file, &progress, &|| {}).unwrap();
        let packages = archive::list_packages(&archive).unwrap();
        assert!(!packages.is_empty());

        let dir = std::env::temp_dir().join(format!("dl-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = &packages[..1];
        archive::extract(&archive, first, |_, part| {
            dir.join(format!("pak01_{part}.vpk"))
        })
        .unwrap();
        let vpk = std::fs::read(dir.join("pak01_dir.vpk")).unwrap();
        assert_eq!(&vpk[..4], &[0x34, 0x12, 0xAA, 0x55]);
        println!("packages: {}, vpk bytes: {}", packages.len(), vpk.len());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Real network round trip for the sidebar data.
    #[test]
    #[ignore]
    fn loads_sections_and_heroes() {
        let catalog = load_catalog().unwrap();
        assert!(catalog.heroes.len() > 30);
        assert!(!catalog.sections.iter().any(|s| s.name == "Skins"));
        println!(
            "sections: {}, heroes: {}",
            catalog.sections.len(),
            catalog.heroes.len()
        );
    }
}
