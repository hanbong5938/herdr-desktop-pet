use crate::character_types::{validate_pack_id, validate_pack_name, OfficialPackIdentity};
use serde::Deserialize;
use std::collections::HashSet;
use url::Url;

pub const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/hanbong5938/herdr-characters/main/catalog.json";
pub const REPOSITORY_URL: &str = "https://github.com/hanbong5938/herdr-characters";
pub const MAX_CATALOG_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug)]
pub struct LocalizedText {
    pub ko: String,
    pub en: String,
}

#[derive(Clone, Debug)]
pub struct OfficialEntry {
    pub identity: OfficialPackIdentity,
    pub name: String,
    pub variant_name: LocalizedText,
    pub description: LocalizedText,
    pub tags: Vec<String>,
    pub author: String,
    pub format_version: u32,
    pub render_mode: String,
    pub download_bytes: u64,
    pub preview_idle_url: String,
    pub install_supported: bool,
    pub(crate) download_url: String,
}

#[derive(Clone, Debug)]
pub struct OfficialCatalogSnapshot {
    pub revision: u64,
    pub entries: Vec<OfficialEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    schema_version: u32,
    repository_url: String,
    release_tag: String,
    characters: Vec<Character>,
}
#[derive(Deserialize)]
struct Character {
    id: String,
    name: String,
    description: Text,
    tags: Vec<String>,
    author: String,
    variants: Vec<Variant>,
}
#[derive(Deserialize)]
struct Text {
    ko: String,
    en: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Variant {
    id: String,
    name: Text,
    version: String,
    format_version: u32,
    render_mode: String,
    download: Download,
    preview: Preview,
}
#[derive(Deserialize)]
struct Download {
    path: String,
    bytes: u64,
    sha256: String,
}
#[derive(Deserialize)]
struct Preview {
    idle: String,
}

fn label(text: &str, max: usize) -> Result<(), String> {
    if text.is_empty() || text.len() > max || text.chars().any(char::is_control) {
        return Err("official catalog contains invalid text".into());
    }
    Ok(())
}
fn token(text: &str, max: usize) -> Result<(), String> {
    if text.is_empty()
        || text.len() > max
        || !text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        || text == "."
        || text == ".."
    {
        return Err("official catalog contains invalid identifier or path".into());
    }
    Ok(())
}
fn path_component(path: &str, prefix: &str, suffix: &str) -> Result<String, String> {
    let basename = path
        .strip_prefix(prefix)
        .ok_or("invalid official asset path")?;
    token(basename, 128)?;
    if !basename.ends_with(suffix) {
        return Err("invalid official asset extension".into());
    }
    Ok(basename.to_owned())
}
fn encoded_url(host: &str, segments: &[&str]) -> Result<String, String> {
    let mut url = Url::parse(host).map_err(|_| "invalid trusted origin")?;
    url.path_segments_mut()
        .map_err(|_| "invalid trusted origin")?
        .extend(segments);
    Ok(url.to_string())
}
pub fn parse_catalog(
    bytes: &[u8],
    builtin_id: &str,
    revision: u64,
) -> Result<OfficialCatalogSnapshot, String> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err("official catalog exceeds size limit".into());
    }
    let catalog: Catalog =
        serde_json::from_slice(bytes).map_err(|e| format!("official catalog is invalid: {e}"))?;
    if catalog.schema_version != 1 || catalog.repository_url != REPOSITORY_URL {
        return Err("unsupported official catalog repository or schema".into());
    }
    token(&catalog.release_tag, 96)?;
    if catalog.characters.len() > 64 {
        return Err("official catalog has too many characters".into());
    }
    let mut entries = Vec::new();
    let mut identities = HashSet::new();
    for character in catalog.characters {
        if character.id != "default" {
            validate_pack_id(&character.id)?;
        }
        validate_pack_name(&character.name)?;
        label(&character.description.ko, 2048)?;
        label(&character.description.en, 2048)?;
        label(&character.author, 128)?;
        if character.tags.len() > 16 || character.variants.len() > 8 {
            return Err("official catalog inventory exceeds limit".into());
        }
        for tag in &character.tags {
            label(tag, 48)?;
        }
        for variant in character.variants {
            if variant.id != "default" {
                validate_pack_id(&variant.id)?;
            }
            label(&variant.name.ko, 128)?;
            label(&variant.name.en, 128)?;
            token(&variant.version, 64)?;
            let basename = path_component(&variant.download.path, "downloads/", ".herdrchar")?;
            let idle = path_component(&variant.preview.idle, "previews/", ".png")?;
            if variant.download.bytes == 0 || variant.download.bytes > 64 * 1024 * 1024 {
                return Err("official archive size exceeds limit".into());
            }
            if variant.download.sha256.len() != 64
                || !variant
                    .download
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err("invalid official archive SHA-256".into());
            }
            label(&variant.render_mode, 16)?;
            let identity = OfficialPackIdentity {
                id: variant.id.clone(),
                version: variant.version,
                release_tag: catalog.release_tag.clone(),
                sha256: variant.download.sha256,
            };
            if !identities.insert(identity.id.clone()) {
                return Err("duplicate official pack id".into());
            }
            let install_supported = matches!(
                (variant.format_version, variant.render_mode.as_str()),
                (2 | 3, "png") | (4, "png" | "rig") | (5, "rig")
            );
            if character.id == "default"
                || character.id == builtin_id
                || variant.id == "default"
                || variant.id == builtin_id
            {
                continue;
            }
            let download_url = encoded_url(
                "https://github.com",
                &[
                    "hanbong5938",
                    "herdr-characters",
                    "releases",
                    "download",
                    &catalog.release_tag,
                    &basename,
                ],
            )?;
            let preview_idle_url = encoded_url(
                "https://raw.githubusercontent.com",
                &["hanbong5938", "herdr-characters", "main", "previews", &idle],
            )?;
            entries.push(OfficialEntry {
                identity,
                name: character.name.clone(),
                variant_name: LocalizedText {
                    ko: variant.name.ko,
                    en: variant.name.en,
                },
                description: LocalizedText {
                    ko: character.description.ko.clone(),
                    en: character.description.en.clone(),
                },
                tags: character.tags.clone(),
                author: character.author.clone(),
                format_version: variant.format_version,
                render_mode: variant.render_mode,
                download_bytes: variant.download.bytes,
                preview_idle_url,
                install_supported,
                download_url,
            });
        }
    }
    Ok(OfficialCatalogSnapshot { revision, entries })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(path: &str, repo: &str) -> Vec<u8> {
        serde_json::json!({"schemaVersion":1,"repositoryUrl":repo,"releaseTag":"packs-v1","characters":[{"id":"test-cat","name":"Cat","description":{"ko":"고양이","en":"Cat"},"tags":["cat"],"author":"Author","variants":[{"id":"test-cat","name":{"ko":"원본","en":"Original"},"version":"1.0","formatVersion":4,"renderMode":"png","download":{"path":path,"bytes":100,"sha256":"a".repeat(64)},"preview":{"idle":"previews/cat.png"}}]}]}).to_string().into_bytes()
    }
    #[test]
    fn rejects_untrusted_source_and_path_escapes() {
        assert!(parse_catalog(
            &fixture("downloads/cat.herdrchar", "https://evil.example"),
            "rubelia",
            1
        )
        .is_err());
        for path in [
            "downloads/../cat.herdrchar",
            "downloads/%2fcat.herdrchar",
            "https://evil.example/cat.herdrchar",
            "downloads/cat.zip",
        ] {
            assert!(
                parse_catalog(&fixture(path, REPOSITORY_URL), "rubelia", 1).is_err(),
                "{path}"
            );
        }
    }
    #[test]
    fn unsupported_format_is_visible_but_not_installable() {
        let mut value: serde_json::Value =
            serde_json::from_slice(&fixture("downloads/cat.herdrchar", REPOSITORY_URL)).unwrap();
        value["characters"][0]["variants"][0]["formatVersion"] = 6.into();
        let snapshot = parse_catalog(&serde_json::to_vec(&value).unwrap(), "rubelia", 1).unwrap();
        assert_eq!(snapshot.entries.len(), 1);
        assert!(!snapshot.entries[0].install_supported);
    }
    #[test]
    fn builtin_and_variant_identity_are_distinct_from_display_group() {
        let mut value: serde_json::Value =
            serde_json::from_slice(&fixture("downloads/cat.herdrchar", REPOSITORY_URL)).unwrap();
        let variant = &mut value["characters"][0]["variants"][0]["id"];
        *variant = "test-cat-blue".into();
        let bytes = serde_json::to_vec(&value).unwrap();
        let snapshot = parse_catalog(&bytes, "test-cat", 7).unwrap();
        assert!(snapshot.entries.is_empty());
        let snapshot = parse_catalog(&bytes, "rubelia-at12-multi-pose", 8).unwrap();
        assert_eq!(snapshot.entries[0].identity.id, "test-cat-blue");
        assert!(snapshot.entries[0].download_url.starts_with(
            "https://github.com/hanbong5938/herdr-characters/releases/download/packs-v1/"
        ));
    }
    #[test]
    fn rejects_duplicate_variant_id_even_across_versions_and_characters() {
        let mut value: serde_json::Value =
            serde_json::from_slice(&fixture("downloads/cat.herdrchar", REPOSITORY_URL)).unwrap();
        let mut second = value["characters"][0]["variants"][0].clone();
        second["version"] = "2.0".into();
        value["characters"][0]["variants"]
            .as_array_mut()
            .unwrap()
            .push(second);
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap(), "rubelia", 1).is_err());
        let second_character = value["characters"][0].clone();
        value["characters"][0]["variants"]
            .as_array_mut()
            .unwrap()
            .pop();
        value["characters"]
            .as_array_mut()
            .unwrap()
            .push(second_character);
        assert!(parse_catalog(&serde_json::to_vec(&value).unwrap(), "rubelia", 1).is_err());
    }
}
