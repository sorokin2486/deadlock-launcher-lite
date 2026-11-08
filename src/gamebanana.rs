//! Minimal GameBanana API client: list, search, mod details and downloads.
//!
//! Everything is fetched on demand; there is no local catalog copy. JSON
//! requests ask only for the fields the UI shows (`_csvProperties`).

use std::fs::File;
use std::io::{self, Read, Write};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use md5::{Digest, Md5};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::archive::Source;

const API: &str = "https://gamebanana.com/apiv11";
const DEADLOCK_GAME_ID: &str = "20948";
pub const PAGE_SIZE: usize = 20;
/// GameBanana search always returns this many results per page.
pub const SEARCH_PAGE_SIZE: usize = 15;
/// Downloads above this size go to a temp file instead of RAM.
const MEMORY_DOWNLOAD_LIMIT: u64 = 256 * 1024 * 1024;
const JSON_LIMIT: u64 = 8 * 1024 * 1024;
/// Fields of a single mod the details dialog needs. The single-item endpoint
/// calls the adult flag `_bIsNsfw`; the list endpoints call it `_bHasContentRatings`.
const DETAIL_FIELDS: &str = concat!(
    "_sName,_aSubmitter,_sText,_aFiles,_aPreviewMedia,_tsDateUpdated,",
    "_nLikeCount,_nDownloadCount,_bIsNsfw,_aCategory,_aSuperCategory"
);

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .build()
        .new_agent()
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Popular,
    Liked,
    Newest,
    Updated,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Popular, Sort::Liked, Sort::Newest, Sort::Updated];

    pub fn label(self) -> &'static str {
        match self {
            Sort::Popular => "Популярные",
            Sort::Liked => "По лайкам",
            Sort::Newest => "Новые",
            Sort::Updated => "Обновлённые",
        }
    }

    fn api_value(self) -> &'static str {
        match self {
            Sort::Popular => "Generic_MostDownloaded",
            Sort::Liked => "Generic_MostLiked",
            Sort::Newest => "Generic_Newest",
            Sort::Updated => "Generic_LatestUpdated",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Image {
    #[serde(rename = "_sBaseUrl", default)]
    base: String,
    #[serde(rename = "_sFile220", default)]
    file220: Option<String>,
    #[serde(rename = "_sFile530", default)]
    file530: Option<String>,
    #[serde(rename = "_sFile", default)]
    file: String,
}

impl Image {
    pub fn small(&self) -> String {
        format!(
            "{}/{}",
            self.base,
            self.file220.as_ref().unwrap_or(&self.file)
        )
    }

    pub fn medium(&self) -> String {
        format!(
            "{}/{}",
            self.base,
            self.file530.as_ref().unwrap_or(&self.file)
        )
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
struct Media {
    #[serde(rename = "_aImages", default)]
    images: Vec<Image>,
}

#[derive(Clone, Debug, Deserialize, Default)]
struct Named {
    #[serde(rename = "_sName", default)]
    name: String,
}

/// A GameBanana mod category: a catalog section or a hero (a subcategory of Skins).
#[derive(Clone, Debug, Deserialize, Default)]
pub struct Category {
    #[serde(rename = "_idRow", default)]
    pub id: u64,
    #[serde(rename = "_sName", default)]
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModCard {
    #[serde(rename = "_idRow")]
    pub id: u64,
    #[serde(rename = "_sName", default)]
    pub name: String,
    #[serde(rename = "_aSubmitter", default)]
    submitter: Named,
    #[serde(rename = "_aRootCategory", default)]
    category: Named,
    #[serde(rename = "_nLikeCount", default)]
    pub likes: u64,
    #[serde(rename = "_bHasContentRatings", default)]
    pub adult: bool,
    #[serde(rename = "_aPreviewMedia", default)]
    media: Media,
}

impl ModCard {
    pub fn author(&self) -> &str {
        &self.submitter.name
    }
    pub fn category(&self) -> &str {
        &self.category.name
    }
    pub fn thumb(&self) -> Option<String> {
        self.media.images.first().map(Image::small)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct GbFile {
    #[serde(rename = "_idRow")]
    pub id: u64,
    #[serde(rename = "_sFile", default)]
    pub name: String,
    #[serde(rename = "_nFilesize", default)]
    pub size: u64,
    #[serde(rename = "_sDownloadUrl")]
    pub url: String,
    #[serde(rename = "_sMd5Checksum", default)]
    pub md5: String,
    #[serde(rename = "_sDescription", default)]
    pub description: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModDetails {
    #[serde(skip)]
    pub id: u64,
    #[serde(rename = "_sName", default)]
    pub name: String,
    #[serde(rename = "_aSubmitter", default)]
    submitter: Named,
    #[serde(rename = "_sText", default)]
    pub text: String,
    #[serde(rename = "_aFiles", default)]
    pub files: Vec<GbFile>,
    #[serde(rename = "_aPreviewMedia", default)]
    media: Media,
    #[serde(rename = "_tsDateUpdated", default)]
    pub updated_at: i64,
    #[serde(rename = "_nLikeCount", default)]
    pub likes: u64,
    #[serde(rename = "_nDownloadCount", default)]
    pub downloads: u64,
    #[serde(rename = "_bIsNsfw", default)]
    pub adult: bool,
    /// The most specific category: a hero, or a hero's subcategory such as
    /// "Faithful/Vanilla+".
    #[serde(rename = "_aCategory", default)]
    pub category: Category,
    /// Parent of `category`; the hero when `category` is a hero's subcategory.
    #[serde(rename = "_aSuperCategory", default)]
    pub super_category: Category,
}

impl ModDetails {
    pub fn author(&self) -> &str {
        &self.submitter.name
    }
    pub fn image(&self) -> Option<String> {
        self.media.images.first().map(Image::medium)
    }
    pub fn page_url(&self) -> String {
        format!("https://gamebanana.com/mods/{}", self.id)
    }
}

#[derive(Debug, Deserialize)]
struct Metadata {
    #[serde(rename = "_nRecordCount", default)]
    total: u64,
    #[serde(rename = "_bIsComplete", default)]
    complete: bool,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(rename = "_aMetadata")]
    meta: Metadata,
    #[serde(rename = "_aRecords", default)]
    records: Vec<ModCard>,
}

pub struct Page {
    pub mods: Vec<ModCard>,
    pub has_more: bool,
    pub total: u64,
}

fn get_json<T: DeserializeOwned>(url: &str, query: &[(&str, &str)]) -> io::Result<T> {
    let mut request = AGENT.get(url);
    for (key, value) in query {
        request = request.query(*key, *value);
    }
    let mut response = request.call().map_err(net_error)?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(JSON_LIMIT)
        .read_to_vec()
        .map_err(net_error)?;
    serde_json::from_slice(&bytes)
        .map_err(|e| io::Error::other(format!("некорректный ответ GameBanana: {e}")))
}

fn net_error(error: ureq::Error) -> io::Error {
    io::Error::other(format!("ошибка сети: {error}"))
}

/// Lists mods, optionally limited to one category (a section or a hero).
pub fn browse(sort: Sort, category: Option<u64>, page: usize) -> io::Result<Page> {
    let page_text = page.to_string();
    let per_page = PAGE_SIZE.to_string();
    let category_text = category.map(|id| id.to_string());
    let mut query = vec![
        ("_nPerpage", per_page.as_str()),
        ("_aFilters[Generic_Game]", DEADLOCK_GAME_ID),
        ("_nPage", page_text.as_str()),
        ("_sSort", sort.api_value()),
    ];
    if let Some(id) = &category_text {
        query.push(("_aFilters[Generic_Category]", id));
    }
    let response: ListResponse = get_json(&format!("{API}/Mod/Index"), &query)?;
    Ok(Page {
        has_more: !response.meta.complete,
        total: response.meta.total,
        mods: response.records,
    })
}

/// Searches all Deadlock mods; the API cannot search inside one category.
pub fn search(text: &str, page: usize) -> io::Result<Page> {
    let page_text = page.to_string();
    let response: ListResponse = get_json(
        &format!("{API}/Util/Search/Results"),
        &[
            ("_sModelName", "Mod"),
            ("_sOrder", "best_match"),
            ("_idGameRow", DEADLOCK_GAME_ID),
            ("_sSearchString", text),
            ("_nPage", &page_text),
        ],
    )?;
    Ok(Page {
        has_more: !response.meta.complete,
        total: response.meta.total,
        mods: response.records,
    })
}

pub fn details(id: u64) -> io::Result<ModDetails> {
    let mut details: ModDetails = get_json(
        &format!("{API}/Mod/{id}"),
        &[("_csvProperties", DETAIL_FIELDS)],
    )?;
    details.id = id;
    Ok(details)
}

/// Top-level catalog sections, or the subcategories of `parent`.
pub fn categories(parent: Option<u64>) -> io::Result<Vec<Category>> {
    let parent_text = parent.map(|id| id.to_string());
    let mut query = vec![("_sSort", "a_to_z"), ("_bShowEmpty", "true")];
    match &parent_text {
        Some(id) => query.push(("_idCategoryRow", id)),
        None => query.push(("_idGameRow", DEADLOCK_GAME_ID)),
    }
    get_json(&format!("{API}/Mod/Categories"), &query)
}

#[derive(Deserialize)]
struct UpdatedOnly {
    #[serde(rename = "_tsDateUpdated", default)]
    updated_at: i64,
}

pub fn updated_at(id: u64) -> io::Result<i64> {
    let response: UpdatedOnly = get_json(
        &format!("{API}/Mod/{id}"),
        &[("_csvProperties", "_tsDateUpdated")],
    )?;
    Ok(response.updated_at)
}

pub fn fetch_bytes(url: &str, limit: u64) -> io::Result<Vec<u8>> {
    let mut response = AGENT.get(url).call().map_err(net_error)?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(net_error)
}

/// Downloads a mod file, verifying its MD5. `progress` receives bytes read so far.
pub fn download(file: &GbFile, progress: &AtomicU64, on_chunk: &dyn Fn()) -> io::Result<Source> {
    let temp_path = std::env::temp_dir().join(format!(
        "deadlock-launcher-{}-{}",
        std::process::id(),
        file.id
    ));
    let result = download_into(file, &temp_path, progress, on_chunk);
    if !matches!(result, Ok(Source::File(_))) {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn download_into(
    file: &GbFile,
    temp_path: &std::path::Path,
    progress: &AtomicU64,
    on_chunk: &dyn Fn(),
) -> io::Result<Source> {
    let mut response = AGENT.get(&file.url).call().map_err(net_error)?;
    let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
    let to_disk =
        file.size > MEMORY_DOWNLOAD_LIMIT || file.name.to_ascii_lowercase().ends_with(".rar");
    let mut disk = match to_disk {
        true => Some(io::BufWriter::new(File::create(temp_path)?)),
        false => None,
    };
    let mut memory = Vec::with_capacity(if to_disk { 0 } else { file.size as usize });
    let mut hasher = Md5::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        hasher.update(chunk);
        match disk.as_mut() {
            Some(writer) => writer.write_all(chunk)?,
            None => memory.extend_from_slice(chunk),
        }
        total += read as u64;
        progress.store(total, Ordering::Relaxed);
        on_chunk();
    }
    if let Some(mut writer) = disk {
        writer.flush()?;
    }

    let digest: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !file.md5.is_empty() && !digest.eq_ignore_ascii_case(&file.md5) {
        return Err(io::Error::other(
            "файл повреждён при скачивании, повторите попытку",
        ));
    }
    Ok(match to_disk {
        true => Source::File(temp_path.to_path_buf()),
        false => Source::Memory(memory),
    })
}

/// Turns GameBanana's HTML description into plain text.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let name = tag
            .trim_start_matches('/')
            .split([' ', '/'])
            .next()
            .unwrap_or("");
        match name {
            "br" | "p" | "div" | "h1" | "h2" | "h3" | "h4" | "ul" | "ol" | "tr" => out.push('\n'),
            "li" if !tag.starts_with('/') => out.push_str("\n• "),
            _ => {}
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    let decoded = decode_entities(&out);
    collapse_blank_lines(&decoded)
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace('\u{a0}', " ")
        .replace("&amp;", "&")
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = Vec::new();
    let mut blank = 0;
    for line in text.lines().map(str::trim_end) {
        blank = if line.trim().is_empty() { blank + 1 } else { 0 };
        if blank <= 1 {
            out.push(line);
        }
    }
    out.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_description_becomes_readable_text() {
        let html = "<b>Instructions:<br></b><ul><li>Uninstall &amp; retry</li><li>Keep #1</li></ul><p></p><p></p>End";
        assert_eq!(
            html_to_text(html),
            "Instructions:\n\n• Uninstall & retry\n• Keep #1\n\nEnd"
        );
    }

    #[test]
    fn parses_details_with_hero_category_and_adult_flag() {
        let json = r#"{"_sName":"Noir","_bIsNsfw":true,"_aCategory":{"_idRow":33311,"_sName":"Haze"},"_aFiles":[{"_idRow":1,"_sFile":"a.zip","_nFilesize":10,"_sDownloadUrl":"https://gamebanana.com/dl/1","_sMd5Checksum":"x"}]}"#;
        let details: ModDetails = serde_json::from_str(json).unwrap();
        assert!(details.adult);
        assert_eq!(details.category.id, 33311);
        assert_eq!(details.files[0].name, "a.zip");
    }

    #[test]
    fn parses_index_record() {
        let json = r#"{"_aMetadata":{"_nRecordCount":5848,"_bIsComplete":false},"_aRecords":[{"_idRow":650634,"_sName":"QOL","_aSubmitter":{"_sName":"civo"},"_aRootCategory":{"_sName":"Fixes"},"_nLikeCount":304,"_bHasContentRatings":false,"_aPreviewMedia":{"_aImages":[{"_sBaseUrl":"https://images.gamebanana.com/img/ss/mods","_sFile":"a.jpg","_sFile220":"220-90_a.jpg"}]}}]}"#;
        let response: ListResponse = serde_json::from_str(json).unwrap();
        let card = &response.records[0];
        assert_eq!(card.author(), "civo");
        assert_eq!(
            card.thumb().unwrap(),
            "https://images.gamebanana.com/img/ss/mods/220-90_a.jpg"
        );
        assert!(!response.meta.complete);
    }
}
