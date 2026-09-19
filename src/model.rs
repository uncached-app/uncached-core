use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalOutcome {
    /// `unreadable` counts directories the walk could not open. They are skipped
    /// rather than failing the walk, so the result may be missing artifacts.
    Completed {
        unreadable: usize,
    },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    NodeModules,
    CargoTarget,
    NextJs,
    Turbo,
    PyCache,
    Gradle,
    DotNetObj,
    Docker,
    XcodeDerivedData,
    SwiftBuild,
    DartTool,
    /// Maven's `target`, confirmed by the POM beside it.
    MavenTarget,
    /// Gradle's per-project `build`, distinct from the `.gradle` cache.
    GradleBuild,
    /// sbt's `target`, at the root and under `project/`.
    SbtTarget,
    /// CMake's out-of-source build trees, including the ones JetBrains IDEs
    /// generate per configuration.
    CMakeBuild,
    /// Nuxt's build state.
    NuxtCache,
    /// SvelteKit's generated app.
    SvelteKitCache,
    /// Flutter's `build` and the per-platform ephemeral directories.
    FlutterBuild,
    /// Mix's compiled output and the language servers' state beside it.
    ///
    /// `deps/` is deliberately absent: fetched dependencies cost network to
    /// restore, which puts them in a different class from compiled output.
    ElixirBuild,
    /// Stack and Cabal build trees.
    HaskellBuild,
    /// Zig's build output and compilation cache.
    ZigCache,
    /// Unity's imported asset database and scratch directories.
    UnityLibrary,
    /// Unreal's generated build state.
    UnrealIntermediate,
    /// Godot 4's import cache and editor state.
    GodotCache,
    /// Notebook checkpoints written beside the notebook itself.
    JupyterCheckpoints,
    /// Pixi's per-project environment.
    PixiEnv,
    /// Composer's installed PHP packages.
    ComposerVendor,
    /// Terraform's downloaded providers and modules.
    TerraformDir,
    /// CocoaPods' installed pods.
    CocoapodsPods,
    /// React Native's caches and its per-platform build directories, which are
    /// only artifacts inside `ios/` and `android/` of an RN project.
    ReactNative,
    /// A registry, store or download cache shared by every project on the
    /// machine. Distinct from an artifact because emptying it also slows down
    /// every future restore, which is the opposite of what a cleaner is for.
    PackageManagerCache,
    /// State written by an AI coding tool. Its own directory usually mixes
    /// disposable scratch with configuration and history, so only proven
    /// subpaths ever appear under this category.
    AiToolCache,
    /// Indexes and caches written by an editor or IDE.
    EditorCache,
    /// The Android SDK's machine-wide downloads and the emulators built from
    /// them. Nowhere near a project: one copy serves every app on the machine.
    AndroidSdk,
    Custom(String),
}

impl Category {
    /// Wire form of the category. Known categories keep their stable v1 names;
    /// a custom category is prefixed so the field is always a plain string.
    pub fn as_wire(&self) -> String {
        match self {
            Self::NodeModules => "NodeModules".to_owned(),
            Self::CargoTarget => "CargoTarget".to_owned(),
            Self::NextJs => "NextJs".to_owned(),
            Self::Turbo => "Turbo".to_owned(),
            Self::PyCache => "PyCache".to_owned(),
            Self::Gradle => "Gradle".to_owned(),
            Self::DotNetObj => "DotNetObj".to_owned(),
            Self::Docker => "Docker".to_owned(),
            Self::XcodeDerivedData => "XcodeDerivedData".to_owned(),
            Self::SwiftBuild => "SwiftBuild".to_owned(),
            Self::DartTool => "DartTool".to_owned(),
            Self::MavenTarget => "MavenTarget".to_owned(),
            Self::GradleBuild => "GradleBuild".to_owned(),
            Self::SbtTarget => "SbtTarget".to_owned(),
            Self::CMakeBuild => "CMakeBuild".to_owned(),
            Self::NuxtCache => "NuxtCache".to_owned(),
            Self::SvelteKitCache => "SvelteKitCache".to_owned(),
            Self::FlutterBuild => "FlutterBuild".to_owned(),
            Self::ElixirBuild => "ElixirBuild".to_owned(),
            Self::HaskellBuild => "HaskellBuild".to_owned(),
            Self::ZigCache => "ZigCache".to_owned(),
            Self::UnityLibrary => "UnityLibrary".to_owned(),
            Self::UnrealIntermediate => "UnrealIntermediate".to_owned(),
            Self::GodotCache => "GodotCache".to_owned(),
            Self::JupyterCheckpoints => "JupyterCheckpoints".to_owned(),
            Self::PixiEnv => "PixiEnv".to_owned(),
            Self::ComposerVendor => "ComposerVendor".to_owned(),
            Self::TerraformDir => "TerraformDir".to_owned(),
            Self::CocoapodsPods => "CocoapodsPods".to_owned(),
            Self::ReactNative => "ReactNative".to_owned(),
            Self::PackageManagerCache => "PackageManagerCache".to_owned(),
            Self::AiToolCache => "AiToolCache".to_owned(),
            Self::EditorCache => "EditorCache".to_owned(),
            Self::AndroidSdk => "AndroidSdk".to_owned(),
            Self::Custom(name) => format!("{CUSTOM_CATEGORY_PREFIX}{name}"),
        }
    }

    pub fn custom(name: impl Into<String>) -> Option<Self> {
        let name = name.into();
        valid_custom_category_name(&name).then_some(Self::Custom(name))
    }

    /// Parses the wire form. Unknown names are rejected rather than silently
    /// folded into `Custom`, which is reserved for the prefixed form.
    pub fn from_wire(value: &str) -> Option<Self> {
        Some(match value {
            "NodeModules" => Self::NodeModules,
            "CargoTarget" => Self::CargoTarget,
            "NextJs" => Self::NextJs,
            "Turbo" => Self::Turbo,
            "PyCache" => Self::PyCache,
            "Gradle" => Self::Gradle,
            "DotNetObj" => Self::DotNetObj,
            "Docker" => Self::Docker,
            "XcodeDerivedData" => Self::XcodeDerivedData,
            "SwiftBuild" => Self::SwiftBuild,
            "DartTool" => Self::DartTool,
            "MavenTarget" => Self::MavenTarget,
            "GradleBuild" => Self::GradleBuild,
            "SbtTarget" => Self::SbtTarget,
            "CMakeBuild" => Self::CMakeBuild,
            "NuxtCache" => Self::NuxtCache,
            "SvelteKitCache" => Self::SvelteKitCache,
            "FlutterBuild" => Self::FlutterBuild,
            "ElixirBuild" => Self::ElixirBuild,
            "HaskellBuild" => Self::HaskellBuild,
            "ZigCache" => Self::ZigCache,
            "UnityLibrary" => Self::UnityLibrary,
            "UnrealIntermediate" => Self::UnrealIntermediate,
            "GodotCache" => Self::GodotCache,
            "JupyterCheckpoints" => Self::JupyterCheckpoints,
            "PixiEnv" => Self::PixiEnv,
            "ComposerVendor" => Self::ComposerVendor,
            "TerraformDir" => Self::TerraformDir,
            "CocoapodsPods" => Self::CocoapodsPods,
            "ReactNative" => Self::ReactNative,
            "PackageManagerCache" => Self::PackageManagerCache,
            "AiToolCache" => Self::AiToolCache,
            "EditorCache" => Self::EditorCache,
            "AndroidSdk" => Self::AndroidSdk,
            _ => {
                let name = value.strip_prefix(CUSTOM_CATEGORY_PREFIX)?;
                if !valid_custom_category_name(name) {
                    return None;
                }
                Self::Custom(name.to_owned())
            }
        })
    }
}

const CUSTOM_CATEGORY_PREFIX: &str = "Custom:";
const MAX_CUSTOM_CATEGORY_NAME_BYTES: usize = 128;

fn valid_custom_category_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_CUSTOM_CATEGORY_NAME_BYTES
        && !name.chars().any(char::is_control)
}

impl Serialize for Category {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if let Self::Custom(name) = self {
            if !valid_custom_category_name(name) {
                return Err(serde::ser::Error::custom("invalid custom category name"));
            }
        }
        serializer.serialize_str(&self.as_wire())
    }
}

impl<'de> Deserialize<'de> for Category {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_wire(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown category: {value}")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanTarget {
    pub path: PathBuf,
    pub category: Category,
    pub size_bytes: u64,
    #[serde(default)]
    pub size_pending: bool,
    #[serde(default)]
    pub size_has_errors: bool,
    pub identity: Option<FileIdentity>,
}

impl ScanTarget {
    pub fn new(path: PathBuf, category: Category, size_bytes: u64) -> Self {
        Self {
            path,
            category,
            size_bytes,
            size_pending: false,
            size_has_errors: false,
            identity: None,
        }
    }

    pub fn with_identity(mut self, identity: Option<FileIdentity>) -> Self {
        self.identity = identity;
        self
    }

    pub fn with_size_state(mut self, size_pending: bool, size_has_errors: bool) -> Self {
        self.size_pending = size_pending;
        self.size_has_errors = size_has_errors;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIdentity {
    pub volume_serial: u64,
    pub file_id: [u8; 16],
}

/// Outcome of a cleanup. Secure cleanup reports the size measured during the
/// scan, while `size_pending` distinguishes an unavailable size from a failed
/// measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanReport {
    pub freed_bytes: u64,
    pub size_pending: bool,
    pub size_has_errors: bool,
}

#[cfg(test)]
mod tests {
    use super::{Category, CleanReport, FileIdentity};
    use serde_json::json;

    #[test]
    fn file_identity_round_trips_through_json() {
        let identity = FileIdentity {
            volume_serial: 42,
            file_id: [7; 16],
        };

        let encoded = serde_json::to_string(&identity).expect("identity should serialize");
        let decoded: FileIdentity =
            serde_json::from_str(&encoded).expect("identity should deserialize");

        assert_eq!(decoded, identity);
    }

    #[test]
    fn file_identity_detects_volume_and_file_id_mismatches() {
        let identity = FileIdentity {
            volume_serial: 42,
            file_id: [7; 16],
        };

        assert_ne!(
            identity,
            FileIdentity {
                volume_serial: 43,
                file_id: [7; 16],
            }
        );
        assert_ne!(
            identity,
            FileIdentity {
                volume_serial: 42,
                file_id: [8; 16],
            }
        );
    }

    #[test]
    fn known_categories_keep_their_v1_string_form() {
        for (category, expected) in [
            (Category::NodeModules, "NodeModules"),
            (Category::CargoTarget, "CargoTarget"),
            (Category::NextJs, "NextJs"),
            (Category::Turbo, "Turbo"),
            (Category::PyCache, "PyCache"),
            (Category::Gradle, "Gradle"),
            (Category::DotNetObj, "DotNetObj"),
            (Category::Docker, "Docker"),
            (Category::XcodeDerivedData, "XcodeDerivedData"),
            (Category::SwiftBuild, "SwiftBuild"),
            (Category::DartTool, "DartTool"),
            (Category::PackageManagerCache, "PackageManagerCache"),
            (Category::AiToolCache, "AiToolCache"),
            (Category::EditorCache, "EditorCache"),
            (Category::AndroidSdk, "AndroidSdk"),
        ] {
            assert_eq!(serde_json::to_value(&category).unwrap(), json!(expected));
            assert_eq!(Category::from_wire(expected), Some(category));
        }
    }

    #[test]
    fn custom_categories_serialize_as_prefixed_strings() {
        let category = Category::Custom("docker-buildx".into());

        assert_eq!(
            serde_json::to_value(&category).unwrap(),
            json!("Custom:docker-buildx")
        );
        assert_eq!(
            serde_json::from_value::<Category>(json!("Custom:docker-buildx")).unwrap(),
            category
        );
    }

    #[test]
    fn custom_category_constructor_rejects_invalid_names() {
        assert!(Category::custom("docker-buildx").is_some());
        assert!(Category::custom("").is_none());
        assert!(Category::custom("line\nfeed").is_none());
        assert!(Category::custom("x".repeat(super::MAX_CUSTOM_CATEGORY_NAME_BYTES + 1)).is_none());
    }

    #[test]
    fn invalid_custom_categories_cannot_serialize() {
        assert!(serde_json::to_value(Category::Custom(String::new())).is_err());
        assert!(serde_json::to_value(Category::Custom("line\nfeed".into())).is_err());
        assert!(serde_json::to_value(Category::Custom(
            "x".repeat(super::MAX_CUSTOM_CATEGORY_NAME_BYTES + 1),
        ))
        .is_err());
    }

    #[test]
    fn scan_target_defaults_to_settled_size_state() {
        let target = super::ScanTarget::new("target".into(), Category::NodeModules, 42);
        assert!(!target.size_pending);
        assert!(!target.size_has_errors);
    }

    #[test]
    fn scan_target_can_preserve_sizing_state() {
        let target = super::ScanTarget::new("target".into(), Category::NodeModules, 42)
            .with_size_state(false, true);
        assert!(!target.size_pending);
        assert!(target.size_has_errors);
    }

    #[test]
    fn unknown_category_strings_are_rejected() {
        assert!(serde_json::from_value::<Category>(json!("Whatever")).is_err());
        assert!(Category::from_wire("Whatever").is_none());
    }

    #[test]
    fn custom_category_wire_names_require_bounded_printable_content() {
        assert!(Category::from_wire("Custom:docker-buildx").is_some());
        assert!(Category::from_wire("Custom:").is_none());
        assert!(Category::from_wire("Custom:line\nfeed").is_none());
        assert!(Category::from_wire(&format!(
            "Custom:{}",
            "x".repeat(super::MAX_CUSTOM_CATEGORY_NAME_BYTES + 1)
        ))
        .is_none());
    }

    #[test]
    fn clean_report_distinguishes_pending_from_failed_measurement() {
        let pending = CleanReport {
            freed_bytes: 0,
            size_pending: true,
            size_has_errors: false,
        };

        assert_eq!(
            serde_json::to_value(pending).unwrap(),
            json!({ "freed_bytes": 0, "size_pending": true, "size_has_errors": false })
        );
    }
}
