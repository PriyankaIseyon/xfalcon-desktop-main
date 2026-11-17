//! Turning a refused DA operation into a code the datasets surface can write
//! copy from.
//!
//! DA's `detail` is never forwarded. It is prose written for a person, but
//! prose that can name a presigned URL, a storage key, or a bucket — so the
//! stable `code` is the only part of a refusal this app carries, and an
//! unrecognised one becomes [`DatasetErrorCode::ServerRefused`] rather than a
//! string nobody can act on.
//!
//! The transfer queue's own failures take the parallel path:
//! `uploads::da_repository::refusal_message` writes the sentence that lands in
//! `UploadProjectionV1::safe_error`. Both read the same `UploadRefusalCode`, so
//! there is one place that knows DA's vocabulary.

use xfalcon_contracts::datasets::DatasetErrorCode;

use crate::clients::da::chat_api::DaChatApiError;
use crate::uploads::da_repository::{UploadRefusal, UploadRefusalCode};
use crate::uploads::service::UploadServiceError;

/// The code a refused catalog or upload operation reports.
pub(crate) fn from_refusal(refusal: &UploadRefusal) -> DatasetErrorCode {
    use UploadRefusalCode as Code;
    match refusal.code {
        Some(Code::UnsupportedFormat | Code::MimeMismatch) => DatasetErrorCode::UnsupportedFormat,
        Some(Code::FileTooLarge | Code::DatasetTooLarge | Code::DecompressedTooLarge) => {
            DatasetErrorCode::FileTooLarge
        }
        Some(Code::EncryptedFile) => DatasetErrorCode::EncryptedFile,
        Some(Code::EmptyFile) => DatasetErrorCode::EmptyFile,
        Some(Code::UploadsDisabled | Code::SourceDisabled) => DatasetErrorCode::UploadsDisabled,
        Some(Code::UserIdentityRequired) => DatasetErrorCode::AuthRequired,
        Some(Code::StorageError) => DatasetErrorCode::Storage,
        // An encoding or structure DA could not read, a cross-tenant URL, a
        // busy or expired upload, and every internal failure are all "the
        // server refused this": the person's next move is the same, and the
        // distinction is only meaningful to whoever reads DA's own logs.
        Some(_) => DatasetErrorCode::ServerRefused,
        None => match refusal.error {
            DaChatApiError::Unauthorized => DatasetErrorCode::AuthRequired,
            DaChatApiError::NotFound => DatasetErrorCode::NotFound,
            DaChatApiError::InvalidResponse => DatasetErrorCode::ServerRefused,
            _ => DatasetErrorCode::Network,
        },
    }
}

/// The code a refused queueing operation reports.
pub(crate) fn from_service(error: &UploadServiceError) -> DatasetErrorCode {
    match error {
        UploadServiceError::AuthRequired => DatasetErrorCode::AuthRequired,
        UploadServiceError::NotConfigured => DatasetErrorCode::NotConfigured,
        UploadServiceError::NoFileChosen => DatasetErrorCode::NoFileChosen,
        UploadServiceError::Unreadable => DatasetErrorCode::Unreadable,
        UploadServiceError::Empty => DatasetErrorCode::EmptyFile,
        UploadServiceError::ContentMismatch => DatasetErrorCode::UnsupportedFormat,
        UploadServiceError::OfficeExtraction(_) => DatasetErrorCode::UnsupportedFormat,
        UploadServiceError::TooLarge => DatasetErrorCode::FileTooLarge,
        UploadServiceError::UnsupportedType => DatasetErrorCode::UnsupportedFormat,
        // Every remaining arm belongs to the composer's queue — a conversation
        // that was not named, a retry of something terminal, an id that is not
        // in the queue, or three transfers already running. None can reach the
        // datasets path except the concurrency bound, which reads as the
        // workspace being busy.
        UploadServiceError::TooManyUploads
        | UploadServiceError::NotFound
        | UploadServiceError::ConversationRequired
        | UploadServiceError::NotRetryable => DatasetErrorCode::ServerRefused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_office_reader_error_maps_to_a_safe_unsupported_format_code() {
        let error = UploadServiceError::OfficeExtraction(
            "The Office file is corrupt or malformed.".to_owned(),
        );
        assert_eq!(from_service(&error), DatasetErrorCode::UnsupportedFormat);
    }
}
