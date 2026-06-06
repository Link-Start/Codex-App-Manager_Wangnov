use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::Serialize;

use crate::{EngineError, OPENAI_PACKAGE_IDENTITY};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsixIdentity {
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub processor_architecture: String,
}

pub fn parse_appx_manifest_xml(xml: &str) -> Result<MsixIdentity, EngineError> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| EngineError::Msix(format!("AppxManifest.xml: {e}")))?;
    let identity = doc
        .descendants()
        .find(|node| node.has_tag_name("Identity"))
        .ok_or_else(|| EngineError::Msix("AppxManifest.xml missing Identity".to_string()))?;

    let get = |name: &str| {
        identity
            .attribute(name)
            .map(str::to_string)
            .ok_or_else(|| EngineError::Msix(format!("Identity missing {name}")))
    };

    Ok(MsixIdentity {
        name: get("Name")?,
        publisher: get("Publisher")?,
        version: get("Version")?,
        processor_architecture: get("ProcessorArchitecture")?,
    })
}

pub fn read_msix_identity(path: &Path) -> Result<MsixIdentity, EngineError> {
    let file = File::open(path).map_err(|e| EngineError::Io(format!("open MSIX: {e}")))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| EngineError::Msix(format!("open MSIX zip: {e}")))?;
    let mut manifest = zip
        .by_name("AppxManifest.xml")
        .map_err(|e| EngineError::Msix(format!("read AppxManifest.xml: {e}")))?;
    let mut xml = String::new();
    manifest
        .read_to_string(&mut xml)
        .map_err(|e| EngineError::Msix(format!("decode AppxManifest.xml: {e}")))?;
    parse_appx_manifest_xml(&xml)
}

fn arch_matches(actual: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let actual = actual.to_ascii_lowercase();
    let expected = expected.to_ascii_lowercase();
    match expected.as_str() {
        "x64" | "x86_64" | "amd64" => actual == "x64" || actual == "neutral",
        "arm64" | "aarch64" => actual == "arm64" || actual == "neutral",
        _ => actual == expected,
    }
}

pub fn validate_codex_identity(
    identity: &MsixIdentity,
    expected_version: &str,
    expected_architecture: Option<&str>,
) -> Result<(), EngineError> {
    if identity.name != OPENAI_PACKAGE_IDENTITY {
        return Err(EngineError::Msix(format!(
            "unexpected package identity {}; expected {}",
            identity.name, OPENAI_PACKAGE_IDENTITY
        )));
    }
    if identity.version != expected_version {
        return Err(EngineError::Msix(format!(
            "unexpected package version {}; expected {}",
            identity.version, expected_version
        )));
    }
    if !arch_matches(&identity.processor_architecture, expected_architecture) {
        return Err(EngineError::Msix(format!(
            "unexpected architecture {}; expected {}",
            identity.processor_architecture,
            expected_architecture.unwrap_or("any")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_appx_identity_xml() {
        let xml = r#"
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex"
            Publisher="CN=OpenAI OpCo, LLC, O=OpenAI OpCo, LLC, C=US"
            Version="26.602.3474.0"
            ProcessorArchitecture="x64" />
</Package>"#;
        let identity = parse_appx_manifest_xml(xml).unwrap();
        assert_eq!(identity.name, "OpenAI.Codex");
        assert_eq!(identity.version, "26.602.3474.0");
        validate_codex_identity(&identity, "26.602.3474.0", Some("x64")).unwrap();
    }

    #[test]
    fn rejects_wrong_identity() {
        let identity = MsixIdentity {
            name: "OpenAI.Other".to_string(),
            publisher: "CN=OpenAI".to_string(),
            version: "26.602.3474.0".to_string(),
            processor_architecture: "x64".to_string(),
        };
        assert!(validate_codex_identity(&identity, "26.602.3474.0", Some("x64")).is_err());
    }
}
