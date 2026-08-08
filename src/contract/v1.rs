use crate::error::CoreErrorInfo;
use crate::model::{Category, CleanReport, ScanTarget};
use serde::{Deserialize, Serialize};

impl<'de> Deserialize<'de> for CoreErrorInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ErrorInfo {
            code: crate::error::CoreErrorCode,
            message: String,
            path: Option<Option<String>>,
            operation: Option<Option<String>>,
        }

        let value = ErrorInfo::deserialize(deserializer)?;
        let path = value.path.ok_or_else(|| {
            serde::de::Error::custom("error object is missing required key: path")
        })?;
        let operation = value.operation.ok_or_else(|| {
            serde::de::Error::custom("error object is missing required key: operation")
        })?;
        Ok(Self {
            code: value.code,
            message: value.message,
            path,
            operation,
        })
    }
}

impl<'de> Deserialize<'de> for crate::error::CoreErrorCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "invalid_workspace" => Ok(Self::InvalidWorkspace),
            "invalid_target" => Ok(Self::InvalidTarget),
            "io" => Ok(Self::Io),
            "trash" => Ok(Self::Trash),
            "recycle_bin_too_large" => Ok(Self::RecycleBinTooLarge),
            "target_identity_unavailable" => Ok(Self::TargetIdentityUnavailable),
            "target_identity_changed" => Ok(Self::TargetIdentityChanged),
            "target_reparse_point" => Ok(Self::TargetReparsePoint),
            "secure_operation_unsupported" => Ok(Self::SecureOperationUnsupported),
            "secure_target_open_failed" => Ok(Self::SecureTargetOpenFailed),
            "secure_enumeration_failed" => Ok(Self::SecureEnumerationFailed),
            "secure_handle_query_failed" => Ok(Self::SecureHandleQueryFailed),
            "secure_deletion_failed" => Ok(Self::SecureDeletionFailed),
            "secure_trash_unavailable" => Ok(Self::SecureTrashUnavailable),
            value => Err(serde::de::Error::custom(format!(
                "unknown error code: {value}"
            ))),
        }
    }
}

pub const API_VERSION: &str = "v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
    Android,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiEnvelope<T> {
    api_version: String,
    platform: Platform,
    data: Option<T>,
    error: Option<CoreErrorInfo>,
}

impl<'de, T> Deserialize<'de> for ApiEnvelope<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[serde(deny_unknown_fields)]
        struct ApiEnvelopeFields<T> {
            api_version: String,
            platform: Platform,
            data: Option<Option<T>>,
            error: Option<Option<CoreErrorInfo>>,
        }

        let fields = ApiEnvelopeFields::deserialize(deserializer)?;
        let data = fields.data.ok_or_else(|| {
            serde::de::Error::custom("API envelope is missing required key: data")
        })?;
        let error = fields.error.ok_or_else(|| {
            serde::de::Error::custom("API envelope is missing required key: error")
        })?;
        if data.is_some() == error.is_some() {
            return Err(serde::de::Error::custom(
                "API envelope must contain exactly one of data or error",
            ));
        }

        Ok(Self {
            api_version: fields.api_version,
            platform: fields.platform,
            data,
            error,
        })
    }
}

impl<T> ApiEnvelope<T> {
    pub fn api_version(&self) -> &str {
        &self.api_version
    }

    pub fn platform(&self) -> &Platform {
        &self.platform
    }

    pub fn data(&self) -> Option<&T> {
        self.data.as_ref()
    }

    pub fn error(&self) -> Option<&CoreErrorInfo> {
        self.error.as_ref()
    }

    pub fn success(platform: Platform, data: T) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            platform,
            data: Some(data),
            error: None,
        }
    }

    pub fn failure(platform: Platform, error: CoreErrorInfo) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            platform,
            data: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupCapabilities {
    pub permanent_cleanup: bool,
    pub move_to_trash: bool,
    pub secure_identity: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanTargetDto {
    pub path: String,
    pub category: Category,
    pub size_bytes: u64,
    pub size_pending: bool,
    pub size_has_errors: bool,
}

impl ScanTargetDto {
    /// Converts an explicitly settled target without losing sizing errors.
    pub fn from_settled(target: SettledScanTarget<'_>) -> Self {
        Self {
            path: target.target.path.to_string_lossy().into_owned(),
            category: target.target.category.clone(),
            size_bytes: target.target.size_bytes,
            size_pending: false,
            size_has_errors: target.size_has_errors,
        }
    }
}

pub struct SettledScanTarget<'a> {
    target: &'a ScanTarget,
    size_has_errors: bool,
}

impl<'a> SettledScanTarget<'a> {
    pub fn new(target: &'a ScanTarget, size_has_errors: bool) -> Self {
        Self {
            target,
            size_has_errors,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetFoundDto {
    pub scan_id: String,
    pub target: ScanTargetDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetSizedDto {
    pub scan_id: String,
    pub target_id: String,
    pub size_bytes: u64,
    pub size_pending: bool,
    pub size_has_errors: bool,
    pub formatted_size: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanFinishedDto {
    pub scan_id: String,
    pub total_targets: u64,
    pub total_bytes_found: u64,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanFailedDto {
    pub scan_id: String,
    pub error: CoreErrorInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResultDto {
    pub success: bool,
    pub target_path: String,
    pub freed_bytes: u64,
    pub size_has_errors: bool,
}

impl From<(String, CleanReport)> for CleanupResultDto {
    fn from((target_path, report): (String, CleanReport)) -> Self {
        Self {
            success: true,
            target_path,
            freed_bytes: report.freed_bytes,
            size_has_errors: report.size_has_errors,
        }
    }
}
