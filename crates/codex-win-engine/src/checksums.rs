use serde::Serialize;

use crate::EngineError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecksumEntry {
    pub sha256: String,
    pub file_name: String,
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn parse_checksums(text: &str) -> Result<Vec<ChecksumEntry>, EngineError> {
    let mut entries = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(file_name) = parts.next() else {
            return Err(EngineError::Checksums(format!(
                "line {} missing file name",
                line_idx + 1
            )));
        };
        if !is_sha256_hex(hash) {
            return Err(EngineError::Checksums(format!(
                "line {} has invalid sha256",
                line_idx + 1
            )));
        }
        entries.push(ChecksumEntry {
            sha256: hash.to_ascii_lowercase(),
            file_name: file_name.trim_start_matches('*').to_string(),
        });
    }
    Ok(entries)
}

pub fn find_msix_sha256(text: &str, package_moniker: &str) -> Result<String, EngineError> {
    let entries = parse_checksums(text)?;
    let moniker = package_moniker.to_ascii_lowercase();

    entries
        .iter()
        .find(|entry| {
            let file = entry.file_name.to_ascii_lowercase();
            file.ends_with(".msix") && file.contains(&moniker)
        })
        .or_else(|| {
            entries
                .iter()
                .find(|entry| entry.file_name.to_ascii_lowercase().ends_with(".msix"))
        })
        .map(|entry| entry.sha256.clone())
        .ok_or_else(|| EngineError::Checksums("no MSIX checksum entry found".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_msix_hash_by_moniker() {
        let text = "\
b7d6e8e3d50ea620e736d0d9ea8df5bc6a0f00b1944ac053874f9d1de11d01b7  Codex-mac-arm64.dmg
6dc2e05ac2b760bbc77ce3f8a992efdb327363512c9c4744b9a146c41bc4d55a  OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0.Msix
";
        let sha = find_msix_sha256(text, "OpenAI.Codex_26.602.3474.0_x64__2p2nqsd0c76g0").unwrap();
        assert_eq!(
            sha,
            "6dc2e05ac2b760bbc77ce3f8a992efdb327363512c9c4744b9a146c41bc4d55a"
        );
    }

    #[test]
    fn rejects_bad_hash() {
        assert!(parse_checksums("not-a-hash file.msix").is_err());
    }
}
