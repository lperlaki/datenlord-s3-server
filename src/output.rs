//! Types which can be converted into a response

#![allow(clippy::wildcard_imports)] // for `use super::*`

use super::{
    error::{S3Error, S3Result},
    error_code::S3ErrorCode,
};

use crate::{
    utils::{time, Apply, ResponseExt, XmlWriterExt},
    BoxStdError, Response,
};

use hyper::{header, Body, StatusCode};
use xml::{
    common::XmlVersion,
    writer::{EventWriter, XmlEvent},
};

/// Types which can be converted into a response
pub trait S3Output {
    /// Try to convert into a response
    ///
    /// # Errors
    /// Returns an `Err` if the output can not be converted into a response
    fn try_into_response(self) -> S3Result<Response>;
}

impl<T, E> S3Output for S3Result<T, E>
where
    T: S3Output,
    E: S3Output,
{
    fn try_into_response(self) -> S3Result<Response> {
        match self {
            Ok(output) => output.try_into_response(),
            Err(err) => match err {
                S3Error::Operation(e) => e.try_into_response(),
                S3Error::InvalidRequest(e) => Err(<S3Error>::InvalidRequest(e)),
                S3Error::InvalidOutput(e) => Err(<S3Error>::InvalidOutput(e)),
                S3Error::Storage(e) => Err(<S3Error>::Storage(e)),
                S3Error::NotSupported => Err(S3Error::NotSupported),
            },
        }
    }
}

/// helper function for error converting
fn wrap_output(f: impl FnOnce() -> Result<Response, BoxStdError>) -> S3Result<Response> {
    match f() {
        Ok(res) => Ok(res),
        Err(e) => Err(<S3Error>::InvalidOutput(e)),
    }
}

/// a typed `None`
const NONE_CALLBACK: Option<fn(Body) -> Response> = None;

/// helper function for generating xml response
fn wrap_xml_output<F>(
    f: F,
    r: Option<impl FnOnce(Body) -> Response>,
    cap: usize,
) -> S3Result<Response>
where
    F: FnOnce(&mut EventWriter<&mut Vec<u8>>) -> Result<(), xml::writer::Error>,
{
    wrap_output(move || {
        let mut body = Vec::with_capacity(cap);
        {
            let mut w = EventWriter::new(&mut body);
            w.write(XmlEvent::StartDocument {
                version: XmlVersion::Version10,
                encoding: Some("UTF-8"),
                standalone: None,
            })?;

            f(&mut w)?;
        }

        let mut res = match r {
            None => Response::new(Body::from(body)),
            Some(r) => r(Body::from(body)),
        };
        res.set_mime(&mime::TEXT_XML)?;

        Ok(res)
    })
}

/// Type representing an error response
#[derive(Debug)]
struct XmlErrorResponse {
    /// code
    code: S3ErrorCode,
    /// message
    message: Option<String>,
    /// resource
    resource: Option<String>,
    /// request_id
    request_id: Option<String>,
}

impl XmlErrorResponse {
    /// Constructs a `XmlErrorResponse`
    const fn from_code_msg(code: S3ErrorCode, message: Option<String>) -> Self {
        Self {
            code,
            message,
            resource: None,
            request_id: None,
        }
    }
}

impl S3Output for XmlErrorResponse {
    fn try_into_response(self) -> S3Result<Response> {
        wrap_xml_output(
            |w| {
                w.stack("Error", |w| {
                    w.opt_element("Code", Some(&self.code.to_string()))?;
                    w.opt_element("Message", self.message.as_deref())?;
                    w.opt_element("Resource", self.resource.as_deref())?;
                    w.opt_element("RequestId", self.request_id.as_deref())?;
                    Ok(())
                })
            },
            Some(|body| {
                let status = self
                    .code
                    .as_status_code()
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

                Response::new_with_status(body, status)
            }),
            64,
        )
    }
}

mod create_bucket {
    //! [`CreateBucket`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CreateBucket.html)

    use super::*;
    use crate::dto::{CreateBucketError, CreateBucketOutput};

    impl S3Output for CreateBucketOutput {
        fn try_into_response(self) -> S3Result<Response> {
            wrap_output(|| {
                let mut res = Response::new(Body::empty());
                res.set_opt_header(header::LOCATION, self.location)?;
                Ok(res)
            })
        }
    }

    impl S3Output for CreateBucketError {
        fn try_into_response(self) -> S3Result<Response> {
            let resp = match self {
                Self::BucketAlreadyExists(msg) => {
                    XmlErrorResponse::from_code_msg(S3ErrorCode::BucketAlreadyExists, msg.into())
                }
                Self::BucketAlreadyOwnedByYou(msg) => XmlErrorResponse::from_code_msg(
                    S3ErrorCode::BucketAlreadyOwnedByYou,
                    msg.into(),
                ),
            };
            resp.try_into_response()
        }
    }
}

mod delete_bucket {
    //! [`DeleteBucket`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_DeleteBucket.html)

    use super::*;
    use crate::dto::{DeleteBucketError, DeleteBucketOutput};
    use crate::utils::Apply;

    impl S3Output for DeleteBucketOutput {
        fn try_into_response(self) -> S3Result<Response> {
            Response::new_with_status(Body::empty(), StatusCode::NO_CONTENT).apply(Ok)
        }
    }

    impl S3Output for DeleteBucketError {
        fn try_into_response(self) -> S3Result<Response> {
            match self {}
        }
    }
}

mod delete_object {
    //! [`DeleteObject`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_DeleteObject.html)

    use super::*;
    use crate::dto::{DeleteObjectError, DeleteObjectOutput};

    impl S3Output for DeleteObjectOutput {
        fn try_into_response(self) -> S3Result<Response> {
            let res = Response::new(Body::empty());
            // TODO: handle other fields
            Ok(res)
        }
    }

    impl S3Output for DeleteObjectError {
        fn try_into_response(self) -> S3Result<Response> {
            match self {}
        }
    }
}

mod get_object {
    //! [`GetObject`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetObject.html)

    use super::*;
    use crate::dto::{GetObjectError, GetObjectOutput};

    impl S3Output for GetObjectOutput {
        fn try_into_response(self) -> S3Result<Response> {
            wrap_output(|| {
                let mut res = Response::new(Body::empty());
                if let Some(body) = self.body {
                    *res.body_mut() = Body::wrap_stream(body);
                }
                res.set_opt_header(
                    header::CONTENT_LENGTH,
                    self.content_length.map(|l| format!("{}", l)),
                )?;
                res.set_opt_header(header::CONTENT_TYPE, self.content_type)?;

                res.set_opt_last_modified(time::map_opt_rfc3339_to_last_modified(
                    self.last_modified,
                )?)?;
                // TODO: handle other fields
                Ok(res)
            })
        }
    }

    impl S3Output for GetObjectError {
        fn try_into_response(self) -> S3Result<Response> {
            let resp = match self {
                Self::NoSuchKey(msg) => {
                    XmlErrorResponse::from_code_msg(S3ErrorCode::NoSuchKey, msg.into())
                }
            };
            resp.try_into_response()
        }
    }
}

mod get_bucket_location {
    //! [`GetBucketLocation`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_GetBucketLocation.html)

    use super::*;
    use crate::dto::{GetBucketLocationError, GetBucketLocationOutput};

    impl S3Output for GetBucketLocationOutput {
        fn try_into_response(self) -> S3Result<Response> {
            wrap_xml_output(
                |w| {
                    w.element(
                        "LocationConstraint",
                        self.location_constraint.as_deref().unwrap_or(""),
                    )
                },
                NONE_CALLBACK,
                4096,
            )
        }
    }

    impl S3Output for GetBucketLocationError {
        fn try_into_response(self) -> S3Result<Response> {
            match self {}
        }
    }
}

mod head_bucket {
    //! [`HeadBucket`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_HeadBucket.html)

    use super::*;
    use crate::dto::{HeadBucketError, HeadBucketOutput};

    impl S3Output for HeadBucketOutput {
        fn try_into_response(self) -> S3Result<Response> {
            Response::new(Body::empty()).apply(Ok)
        }
    }

    impl S3Output for HeadBucketError {
        fn try_into_response(self) -> S3Result<Response> {
            let resp = match self {
                Self::NoSuchBucket(msg) => {
                    XmlErrorResponse::from_code_msg(S3ErrorCode::NoSuchBucket, msg.into())
                }
            };
            resp.try_into_response()
        }
    }
}

mod head_object {
    //! [`HeadObject`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_HeadObject.html)

    use super::*;
    use crate::dto::{HeadObjectError, HeadObjectOutput};

    impl S3Output for HeadObjectOutput {
        fn try_into_response(self) -> S3Result<Response> {
            wrap_output(|| {
                let mut res = Response::new(Body::empty());
                res.set_opt_header(header::CONTENT_TYPE, self.content_type)?;
                res.set_opt_header(
                    header::CONTENT_LENGTH,
                    self.content_length.map(|l| l.to_string()),
                )?;
                res.set_opt_last_modified(time::map_opt_rfc3339_to_last_modified(
                    self.last_modified,
                )?)?;
                res.set_opt_header(header::ETAG, self.e_tag)?;
                res.set_opt_header(header::EXPIRES, self.expires)?;
                // TODO: handle other fields
                Ok(res)
            })
        }
    }

    impl S3Output for HeadObjectError {
        fn try_into_response(self) -> S3Result<Response> {
            let resp = match self {
                Self::NoSuchKey(msg) => {
                    XmlErrorResponse::from_code_msg(S3ErrorCode::NoSuchKey, msg.into())
                }
            };
            resp.try_into_response()
        }
    }
}

mod list_buckets {
    //! [`ListBuckets`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_ListBuckets.html)

    use super::*;
    use crate::dto::{ListBucketsError, ListBucketsOutput};

    impl S3Output for ListBucketsOutput {
        fn try_into_response(self) -> S3Result<Response> {
            wrap_xml_output(
                |w| {
                    w.stack("ListBucketsOutput", |w| {
                        w.opt_stack("Buckets", self.buckets, |w, buckets| {
                            for bucket in buckets {
                                w.stack("Bucket", |w| {
                                    w.opt_element("CreationDate", bucket.creation_date.as_deref())?;
                                    w.opt_element("Name", bucket.name.as_deref())
                                })?;
                            }
                            Ok(())
                        })?;

                        w.opt_stack("Owner", self.owner, |w, owner| {
                            w.opt_element("DisplayName", owner.display_name.as_deref())?;
                            w.opt_element("ID", owner.id.as_deref())
                        })?;
                        Ok(())
                    })
                },
                NONE_CALLBACK,
                4096,
            )
        }
    }

    impl S3Output for ListBucketsError {
        fn try_into_response(self) -> S3Result<Response> {
            match self {}
        }
    }
}

mod list_objects {
    //! [`ListObjects`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_ListObjects.html)

    use super::*;
    use crate::dto::{ListObjectsError, ListObjectsOutput};

    impl S3Output for ListObjectsError {
        fn try_into_response(self) -> S3Result<Response> {
            let resp = match self {
                Self::NoSuchBucket(msg) => {
                    XmlErrorResponse::from_code_msg(S3ErrorCode::NoSuchBucket, msg.into())
                }
            };
            resp.try_into_response()
        }
    }

    impl S3Output for ListObjectsOutput {
        fn try_into_response(self) -> S3Result<Response> {
            wrap_xml_output(
                |w| {
                    w.stack("ListBucketResult", |w| {
                        w.opt_element(
                            "IsTruncated",
                            self.is_truncated.map(|b| b.to_string()).as_deref(),
                        )?;
                        w.opt_element("Marker", self.marker.as_deref())?;
                        w.opt_element("NextMarker", self.next_marker.as_deref())?;
                        if let Some(contents) = self.contents {
                            for content in contents {
                                w.stack("Contents", |w| {
                                    w.opt_element("Key", content.key.as_deref())?;
                                    w.opt_element(
                                        "LastModified",
                                        content.last_modified.as_deref(),
                                    )?;
                                    w.opt_element("ETag", content.e_tag.as_deref())?;
                                    w.opt_element(
                                        "Size",
                                        content.size.map(|s| s.to_string()).as_deref(),
                                    )?;
                                    w.opt_element(
                                        "StorageClass",
                                        content.storage_class.as_deref(),
                                    )?;
                                    w.opt_stack("Owner", content.owner, |w, owner| {
                                        w.opt_element("ID", owner.id.as_deref())?;
                                        w.opt_element(
                                            "DisplayName",
                                            owner.display_name.as_deref(),
                                        )?;
                                        Ok(())
                                    })
                                })?;
                            }
                        }
                        w.opt_element("Name", self.name.as_deref())?;
                        w.opt_element("Prefix", self.prefix.as_deref())?;
                        w.opt_element("Delimiter", self.delimiter.as_deref())?;
                        w.opt_element("MaxKeys", self.max_keys.map(|k| k.to_string()).as_deref())?;
                        w.opt_stack("CommonPrefixes", self.common_prefixes, |w, prefixes| {
                            w.iter_element(prefixes.into_iter(), |w, common_prefix| {
                                w.opt_element("Prefix", common_prefix.prefix.as_deref())
                            })
                        })?;
                        w.opt_element("EncodingType", self.encoding_type.as_deref())?;
                        Ok(())
                    })
                },
                NONE_CALLBACK,
                4096,
            )
        }
    }
}

mod put_object {
    //! [`PutObject`](https://docs.aws.amazon.com/AmazonS3/latest/API/API_PutObject.html)

    use super::*;
    use crate::dto::{PutObjectError, PutObjectOutput};

    impl S3Output for PutObjectOutput {
        fn try_into_response(self) -> S3Result<Response> {
            let res = Response::new(Body::empty());
            // TODO: handle other fields
            Ok(res)
        }
    }

    impl S3Output for PutObjectError {
        fn try_into_response(self) -> S3Result<Response> {
            match self {}
        }
    }
}