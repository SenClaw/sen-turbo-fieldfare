//! The one checkpoint TurboFieldfare will load.
//!
//! Keep these constants in lockstep with `SupportedModelSource` in
//! TurboFieldfareRepack. The installer has no repo flag: it always fetches
//! this revision and writes `sourceSnapshotHash` into `manifest.json`. A
//! directory that does not carry that identity is not a model this runtime
//! can run.

use std::path::Path;

use anyhow::{bail, Context};

pub const REPO_ID: &str = "mlx-community/gemma-4-26b-a4b-it-4bit";
pub const REVISION: &str = "0d77464eeb233a2da68ebf9d7dc4edaac7db956d";
pub const SOURCE_INDEX_SHA256: &str =
    "bf198c9f5ea6462addca1966e5dd669c407537a876e82cf06db9084c5c850b13";

const MANIFEST_CAP: u64 = 4 * 1024 * 1024;

/// Refuse to start unless `dir` is a finished text install of the pinned
/// Gemma 4 checkpoint. Vision is a sibling pack and is not required.
pub fn require_pinned(dir: &Path) -> anyhow::Result<()> {
    let manifest_path = dir.join("manifest.json");
    let len = std::fs::metadata(&manifest_path)
        .with_context(|| format!("no manifest.json in {}", dir.display()))?
        .len();
    if len > MANIFEST_CAP {
        bail!("manifest.json in {} is too large to be a TurboFieldfare install", dir.display());
    }
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("{} is not JSON", manifest_path.display()))?;
    if manifest.get("magic").and_then(|v| v.as_str()) != Some("GTURBO") {
        bail!("{} is not a TurboFieldfare text model (magic must be GTURBO)", dir.display());
    }
    let model_id = manifest.get("modelID").and_then(|v| v.as_str()).unwrap_or("");
    if model_id != REPO_ID {
        bail!(
            "TurboFieldfare only runs {REPO_ID} (revision {REVISION}). This directory says `{model_id}`."
        );
    }
    let hash = manifest.get("sourceSnapshotHash").and_then(|v| v.as_str()).unwrap_or("");
    if !snapshot_matches(hash) {
        bail!(
            "TurboFieldfare only runs source snapshot {SOURCE_INDEX_SHA256}. This directory says `{hash}`."
        );
    }
    if !dir.join("model_weights.bin").is_file() {
        bail!("{} is an incomplete .gturbo install (model_weights.bin is missing)", dir.display());
    }
    Ok(())
}

fn snapshot_matches(value: &str) -> bool {
    let hex = value.strip_prefix("sha256:").unwrap_or(value);
    hex.eq_ignore_ascii_case(SOURCE_INDEX_SHA256)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_install(dir: &Path, manifest: &str, weights: bool) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        if weights {
            std::fs::write(dir.join("model_weights.bin"), b"w").unwrap();
        }
    }

    #[test]
    fn the_pinned_manifest_is_accepted() {
        let dir = std::env::temp_dir().join(format!("sen-tf-pin-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_install(
            &dir,
            &format!(
                r#"{{"magic":"GTURBO","modelID":"{REPO_ID}","sourceSnapshotHash":"sha256:{SOURCE_INDEX_SHA256}"}}"#
            ),
            true,
        );
        assert!(require_pinned(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_different_model_id_is_refused() {
        let dir = std::env::temp_dir().join(format!("sen-tf-pin-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write_install(
            &dir,
            r#"{"magic":"GTURBO","modelID":"other/model","sourceSnapshotHash":"sha256:bf198c9f5ea6462addca1966e5dd669c407537a876e82cf06db9084c5c850b13"}"#,
            true,
        );
        let err = require_pinned(&dir).unwrap_err().to_string();
        assert!(err.contains(REPO_ID), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
