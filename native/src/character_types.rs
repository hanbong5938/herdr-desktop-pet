use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_RENDERER_EPOCH: AtomicU64 = AtomicU64::new(1);

/// A stable identity for a character pack revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterRef {
    pub id: String,
    pub revision: u64,
}

impl CharacterRef {
    pub fn builtin() -> Self {
        Self {
            id: "default".to_string(),
            revision: 0,
        }
    }

    pub fn is_builtin(&self) -> bool {
        self.revision == 0 && self.id == "default"
    }
}

/// Identity carried through native preparation and the first applied surface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererToken {
    pub operation_id: String,
    pub reference: CharacterRef,
    pub content_digest: String,
    pub backend_epoch: u64,
}

impl RendererToken {
    pub fn new(
        operation_id: String,
        reference: CharacterRef,
        content_digest: String,
    ) -> Result<Self, String> {
        if content_digest.len() != 64
            || !content_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("renderer content digest is invalid".to_owned());
        }
        let backend_epoch = NEXT_RENDERER_EPOCH
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |epoch| {
                epoch.checked_add(1)
            })
            .map_err(|_| "renderer epoch exhausted".to_owned())?;
        Ok(Self {
            operation_id,
            reference,
            content_digest,
            backend_epoch,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRecord {
    pub id: String,
    pub name: String,
    pub head: u64,
    pub revisions: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackListing {
    pub generation: u64,
    pub selected: CharacterRef,
    pub active: Option<CharacterRef>,
    pub override_active: bool,
    pub packs: Vec<PackRecord>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum PackAction {
    Import { path: PathBuf },
    Select { id: String },
    Update { id: String, path: PathBuf },
    Restore { id: String, revision: u64 },
    Remove { id: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRequest {
    pub operation_id: String,
    pub expected_generation: Option<u64>,
    pub action: PackAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackOperation {
    pub operation_id: String,
    pub state: String,
    pub committed: bool,
    pub ui_applied: bool,
    pub generation: Option<u64>,
    pub error: Option<String>,
}

/// Validate a user-managed pack identifier.
///
/// The builtin `default` character owns that reserved id and cannot be
/// imported or updated as a managed pack.
pub fn validate_pack_id(id: &str) -> Result<(), String> {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err("managed character id must be 1..64 bytes".to_string());
    }
    if id == "default" {
        return Err("managed character id default is reserved".to_string());
    }
    if !bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit() {
        return Err("managed character id must start with lowercase ASCII or a digit".to_string());
    }
    if !bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
    }) {
        return Err("managed character id contains unsafe characters".to_string());
    }
    Ok(())
}

/// Validate a user-managed pack display name.
pub fn validate_pack_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 128 {
        return Err("managed character name must be 1..128 UTF-8 bytes".to_string());
    }
    if name.chars().any(char::is_control) {
        return Err("managed character name must not contain control characters".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_id_grammar_covers_reserved_and_length_boundaries() {
        assert!(validate_pack_id("a").is_ok());
        assert!(validate_pack_id("a".repeat(64).as_str()).is_ok());
        assert!(validate_pack_id("").is_err());
        assert!(validate_pack_id("a".repeat(65).as_str()).is_err());
        assert!(validate_pack_id("default").is_err());
        assert!(validate_pack_id("A-pet").is_err());
        assert!(validate_pack_id("pet name").is_err());
        assert!(validate_pack_id("pet/child").is_err());
        assert!(validate_pack_id("é").is_err());
    }

    #[test]
    fn pack_name_rejects_controls_and_counts_utf8_bytes() {
        assert!(validate_pack_name("Pet é").is_ok());
        assert!(validate_pack_name("Pet\n").is_err());
        assert!(validate_pack_name(&"é".repeat(64)).is_ok());
        assert!(validate_pack_name(&"é".repeat(65)).is_err());
    }
}
