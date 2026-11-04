//! Parse only the machine-readable error code; never expose server messages.
use instant_xml::FromXml;
#[derive(FromXml)]
#[xml(rename = "Error")]
struct ErrorCode {
    #[xml(rename = "Code")]
    code: String,
}

#[derive(FromXml)]
#[xml(rename = "Error", ns("http://s3.amazonaws.com/doc/2006-03-01/"))]
struct NamespacedErrorCode {
    #[xml(rename = "Code")]
    code: String,
}

pub(super) fn is_missing_bucket(body: &[u8]) -> bool {
    let Ok(xml) = std::str::from_utf8(body) else {
        return false;
    };
    let code = instant_xml::from_str::<ErrorCode>(xml)
        .map(|e| e.code)
        .or_else(|_| instant_xml::from_str::<NamespacedErrorCode>(xml).map(|e| e.code));
    code.is_ok_and(|code| code == "NoSuchBucket")
}

#[cfg(test)]
mod tests {
    use crate::backend::{Backend, BackendError, ObjectName, s3::tests::mock_headers};

    #[test]
    fn s3_missing_bucket_xml_is_distinct_from_missing_key_or_plain_404() {
        for xml in [
            "<Error><Code>NoSuchBucket</Code><Message>secret</Message></Error>",
            "<Error xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Code>NoSuchBucket</Code></Error>",
        ] {
            let (backend, server) = mock_headers(vec![(404, xml, "")]);
            let error = backend
                .read(&ObjectName::new("config").unwrap())
                .unwrap_err();
            assert!(matches!(error, BackendError::BucketNotFound(_)));
            assert_eq!(error.to_string(), "bucket not found: bucket");
            assert!(!format!("{error:?}").contains("secret"));
            assert_eq!(server.join().unwrap().len(), 1);
        }
        for xml in [
            "",
            "<Error><Code>NoSuchKey</Code></Error>",
            "<Error><Message>NoSuchBucket</Message><Code>NoSuchKey</Code></Error>",
            "<invalid>",
        ] {
            let (backend, server) = mock_headers(vec![(404, xml, "")]);
            assert!(matches!(
                backend.read(&ObjectName::new("config").unwrap()),
                Err(BackendError::NotFound(_))
            ));
            assert_eq!(server.join().unwrap().len(), 1);
        }
    }
}
